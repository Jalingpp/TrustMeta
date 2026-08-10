use crate::core::RootSummaryChange;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, Instant};

#[derive(Clone)]
pub struct Blockchain {
    inner: Option<Arc<ChainHandle>>,
}

struct ChainHandle {
    sender: mpsc::UnboundedSender<ChainCommand>,
}

enum ChainCommand {
    Commit(Vec<PendingCommit>),
    Reset(oneshot::Sender<Result<(), String>>),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct PendingCommit {
    prefix: String,
    summary_hex: String,
}

#[derive(Clone)]
struct ChainConfig {
    state_file: PathBuf,
    submitter: PathBuf,
    reset_script: PathBuf,
    outbox_file: PathBuf,
    batch_size: usize,
    batch_interval: Duration,
    retry_base: Duration,
    submit_timeout: Duration,
}

#[derive(Serialize)]
struct SubmitRequest<'a> {
    changes: &'a [PendingCommit],
}

impl Blockchain {
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    pub fn from_env() -> Result<Self, String> {
        let enabled = std::env::var("MANAGER_CHAIN_ENABLED")
            .ok()
            .map(|value| {
                let value = value.trim().to_lowercase();
                value != "0" && value != "false" && value != "off"
            })
            .unwrap_or(false);
        if !enabled {
            return Ok(Self::disabled());
        }

        let config = ChainConfig::from_env()?;
        let (sender, receiver) = mpsc::unbounded_channel();
        let worker_config = config.clone();
        tokio::spawn(async move {
            run_worker(worker_config, receiver).await;
        });

        Ok(Self {
            inner: Some(Arc::new(ChainHandle { sender })),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub fn enqueue<I>(&self, changes: I)
    where
        I: IntoIterator<Item = RootSummaryChange>,
    {
        let Some(inner) = &self.inner else {
            return;
        };

        let commits: Vec<_> = changes
            .into_iter()
            .map(|change| PendingCommit {
                prefix: change.prefix,
                summary_hex: hex_encode(&change.new_summary),
            })
            .collect();
        if commits.is_empty() {
            return;
        }

        if inner.sender.send(ChainCommand::Commit(commits)).is_err() {
            eprintln!("[CHAIN] submission worker is unavailable; root commitments were not queued");
        }
    }

    pub async fn reset(&self) -> Result<(), String> {
        let Some(inner) = &self.inner else {
            return Ok(());
        };

        let (sender, receiver) = oneshot::channel();
        inner
            .sender
            .send(ChainCommand::Reset(sender))
            .map_err(|_| "submission worker is unavailable".to_string())?;
        receiver
            .await
            .map_err(|_| "submission worker dropped reset response".to_string())?
    }
}

impl Default for Blockchain {
    fn default() -> Self {
        Self::disabled()
    }
}

impl ChainConfig {
    fn from_env() -> Result<Self, String> {
        let state_file = env_path(
            "MANAGER_ETH_STATE_FILE",
            PathBuf::from("scripts/data/ethereum.state"),
        );
        let submitter = env_path(
            "MANAGER_ETH_SUBMITTER",
            PathBuf::from("scripts/blockchain/chain_submit.sh"),
        );
        let reset_script = env_path(
            "MANAGER_ETH_RESET_SCRIPT",
            PathBuf::from("scripts/blockchain/reset_chain.sh"),
        );
        let outbox_file = env_path(
            "MANAGER_ETH_OUTBOX_FILE",
            PathBuf::from("scripts/data/ethereum.outbox.jsonl"),
        );

        for (name, path) in [
            ("MANAGER_ETH_STATE_FILE", &state_file),
            ("MANAGER_ETH_SUBMITTER", &submitter),
            ("MANAGER_ETH_RESET_SCRIPT", &reset_script),
        ] {
            if !path.exists() {
                return Err(format!("{name} does not exist: {}", path.display()));
            }
        }

        Ok(Self {
            state_file,
            submitter,
            reset_script,
            outbox_file,
            batch_size: env_positive_usize("MANAGER_ETH_BATCH_SIZE", 32),
            batch_interval: Duration::from_millis(env_positive_u64(
                "MANAGER_ETH_BATCH_INTERVAL_MS",
                100,
            )),
            retry_base: Duration::from_millis(env_positive_u64("MANAGER_ETH_RETRY_BASE_MS", 500)),
            submit_timeout: Duration::from_secs(env_positive_u64(
                "MANAGER_ETH_SUBMIT_TIMEOUT_SECS",
                35,
            )),
        })
    }
}

async fn run_worker(config: ChainConfig, mut receiver: mpsc::UnboundedReceiver<ChainCommand>) {
    let mut pending = match load_outbox(&config.outbox_file) {
        Ok(commits) => commits,
        Err(err) => {
            eprintln!("[CHAIN] failed to load outbox: {err}");
            VecDeque::new()
        }
    };

    if !pending.is_empty() {
        eprintln!(
            "[CHAIN] recovered {} pending root commitment(s) from {}",
            pending.len(),
            config.outbox_file.display()
        );
    }

    let mut tick = tokio::time::interval(config.batch_interval);
    let mut retry_delay = config.retry_base;
    let mut next_retry_at = Instant::now();
    let mut outbox_dirty = false;
    loop {
        tokio::select! {
            biased;
            message = receiver.recv() => {
                match message {
                    Some(ChainCommand::Commit(commits)) => {
                        if let Err(err) = append_outbox(&config.outbox_file, &commits) {
                            eprintln!("[CHAIN] failed to append outbox: {err}");
                            pending.extend(commits);
                            outbox_dirty = true;
                        } else {
                            pending.extend(commits);
                        }
                        if !outbox_dirty
                            && pending.len() >= config.batch_size
                            && Instant::now() >= next_retry_at
                        {
                            if flush_pending(&config, &mut pending, &mut outbox_dirty).await {
                                retry_delay = config.retry_base;
                                next_retry_at = Instant::now();
                            } else {
                                next_retry_at = Instant::now() + retry_delay;
                                retry_delay = doubled_delay(retry_delay);
                            }
                        }
                    }
                    Some(ChainCommand::Reset(response)) => {
                        pending.clear();
                        outbox_dirty = false;
                        if let Err(err) = clear_outbox(&config.outbox_file) {
                            eprintln!("[CHAIN] failed to clear outbox before reset: {err}");
                        }
                        let result = run_reset_script(&config).await;
                        if result.is_ok() {
                            if let Err(err) = clear_outbox(&config.outbox_file) {
                                eprintln!("[CHAIN] failed to clear outbox after reset: {err}");
                            }
                        }
                        let _ = response.send(result);
                    }
                    None => return,
                }
            }
            _ = tick.tick() => {
                if outbox_dirty {
                    match rewrite_outbox(&config.outbox_file, &pending) {
                        Ok(()) => outbox_dirty = false,
                        Err(err) => eprintln!("[CHAIN] failed to recover durable outbox: {err}"),
                    }
                }
                if !outbox_dirty && !pending.is_empty() && Instant::now() >= next_retry_at {
                    if flush_pending(&config, &mut pending, &mut outbox_dirty).await {
                        retry_delay = config.retry_base;
                        next_retry_at = Instant::now();
                    } else {
                        next_retry_at = Instant::now() + retry_delay;
                        retry_delay = doubled_delay(retry_delay);
                    }
                }
            }
        }
    }
}

async fn flush_pending(
    config: &ChainConfig,
    pending: &mut VecDeque<PendingCommit>,
    outbox_dirty: &mut bool,
) -> bool {
    if pending.is_empty() {
        return true;
    }

    let batch_len = pending.len().min(config.batch_size);
    let batch: Vec<_> = pending.iter().take(batch_len).cloned().collect();
    if let Err(err) = submit_once(config, &batch).await {
        eprintln!("[CHAIN] commitBatch attempt failed; retaining outbox: {err}");
        return false;
    }

    pending.drain(..batch_len);
    if let Err(err) = rewrite_outbox(&config.outbox_file, pending) {
        // The transaction is already confirmed. Keeping the in-memory queue
        // moving avoids blocking the manager; a restart may harmlessly replay
        // these entries because the contract ignores the same latest digest.
        eprintln!("[CHAIN] failed to rewrite outbox after confirmation: {err}");
        *outbox_dirty = true;
    } else {
        *outbox_dirty = false;
    }
    true
}

async fn submit_once(config: &ChainConfig, batch: &[PendingCommit]) -> Result<(), String> {
    let request = match serde_json::to_vec(&SubmitRequest { changes: batch }) {
        Ok(request) => request,
        Err(err) => return Err(format!("failed to encode commit request: {err}")),
    };

    let mut child = Command::new(&config.submitter)
        .arg("--state-file")
        .arg(&config.state_file)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| format!("failed to start submitter: {err}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(&request)
            .await
            .map_err(|err| format!("failed to write submit request: {err}"))?;
        stdin
            .shutdown()
            .await
            .map_err(|err| format!("failed to close submit request: {err}"))?;
    }

    let output = tokio::time::timeout(config.submit_timeout, child.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "submitter timed out after {} seconds",
                config.submit_timeout.as_secs()
            )
        })?
        .map_err(|err| format!("submitter process failed: {err}"))?;

    if output.status.success() {
        eprintln!(
            "[CHAIN] committed {} root summary change(s): {}",
            batch.len(),
            String::from_utf8_lossy(&output.stdout).trim()
        );
        Ok(())
    } else {
        Err(format!(
            "submitter exited with status={}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn doubled_delay(delay: Duration) -> Duration {
    delay
        .checked_mul(2)
        .unwrap_or(Duration::from_secs(30))
        .min(Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_empty_summary_as_hex_zero() {
        assert_eq!(hex_encode(&[]), "0x");
    }

    #[test]
    fn encodes_summary_as_lowercase_hex() {
        assert_eq!(hex_encode(&[0x00, 0xab, 0xff]), "0x00abff");
    }

    #[test]
    fn retry_delay_is_capped() {
        assert_eq!(
            doubled_delay(Duration::from_secs(20)),
            Duration::from_secs(30)
        );
        assert_eq!(
            doubled_delay(Duration::from_secs(30)),
            Duration::from_secs(30)
        );
    }
}

async fn run_reset_script(config: &ChainConfig) -> Result<(), String> {
    let output = Command::new(&config.reset_script)
        .output()
        .await
        .map_err(|err| format!("failed to start reset script: {err}"))?;
    if output.status.success() {
        eprintln!("[CHAIN] private Ethereum chain reset and redeployed");
        Ok(())
    } else {
        Err(format!(
            "reset script failed (status={}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn load_outbox(path: &Path) -> Result<VecDeque<PendingCommit>, String> {
    if !path.exists() {
        return Ok(VecDeque::new());
    }
    let content =
        fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let mut commits = VecDeque::new();
    for (line_number, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let commit = serde_json::from_str(line).map_err(|err| {
            format!(
                "parse outbox line {} in {}: {err}",
                line_number + 1,
                path.display()
            )
        })?;
        commits.push_back(commit);
    }
    Ok(commits)
}

fn append_outbox(path: &Path, commits: &[PendingCommit]) -> Result<(), String> {
    if commits.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| format!("open {}: {err}", path.display()))?;
    for commit in commits {
        let line =
            serde_json::to_string(commit).map_err(|err| format!("encode outbox entry: {err}"))?;
        writeln!(file, "{line}").map_err(|err| format!("append {}: {err}", path.display()))?;
    }
    file.sync_data()
        .map_err(|err| format!("sync {}: {err}", path.display()))
}

fn rewrite_outbox(path: &Path, commits: &VecDeque<PendingCommit>) -> Result<(), String> {
    if commits.is_empty() {
        return clear_outbox(path);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    }

    let temp_path = path.with_extension("jsonl.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp_path)
        .map_err(|err| format!("open {}: {err}", temp_path.display()))?;
    for commit in commits {
        let line =
            serde_json::to_string(commit).map_err(|err| format!("encode outbox entry: {err}"))?;
        writeln!(file, "{line}").map_err(|err| format!("write {}: {err}", temp_path.display()))?;
    }
    file.sync_all()
        .map_err(|err| format!("sync {}: {err}", temp_path.display()))?;
    fs::rename(&temp_path, path).map_err(|err| format!("replace {}: {err}", path.display()))
}

fn clear_outbox(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("remove {}: {err}", path.display())),
    }
}

fn env_path(name: &str, default: PathBuf) -> PathBuf {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or(default)
}

fn env_positive_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_positive_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(2 + bytes.len() * 2);
    output.push_str("0x");
    for byte in bytes {
        output.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        output.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    output
}
