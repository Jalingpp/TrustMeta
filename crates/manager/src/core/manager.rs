use super::{PrefixSplitPlan, RouteMode, RouteTarget, Router};
use common::rpc::storager_service_client::StoragerServiceClient;
use common::ProofVerifier;
use common::{metrics_output, AdsMode, RootHash, SetProofMode};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::OnceCell;
use tokio::sync::{
    Mutex as AsyncMutex, Notify, OwnedRwLockWriteGuard, OwnedSemaphorePermit,
    RwLock as AsyncRwLock, Semaphore,
};
use tokio::time::{sleep, Duration};
use tonic::transport::Channel;

fn env_duration_secs(key: &str, default_secs: u64) -> std::time::Duration {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| std::time::Duration::from_secs(default_secs))
}

fn env_optional_duration_secs(key: &str, default_secs: Option<u64>) -> Option<std::time::Duration> {
    match std::env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .and_then(|secs| (secs > 0).then(|| std::time::Duration::from_secs(secs))),
        Err(_) => default_secs.map(std::time::Duration::from_secs),
    }
}

fn env_positive_usize(key: &str, default_value: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default_value)
}

#[derive(Clone, Debug)]
pub enum PendingOperation {
    Add {
        keyword: String,
        fid: String,
    },
    Delete {
        keyword: String,
        fid: String,
    },
    Update {
        keyword: String,
        old_fid: String,
        new_fid: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct PrefixMigrationState {
    pub parent_prefix: String,
    pub source_node: String,
    pub source_addr: String,
    pub child_prefixes: Vec<String>,
    pub target_nodes: HashMap<String, String>,
    pub confirmed: bool,
    pub pending_operations: Vec<PendingOperation>,
}

#[derive(Clone)]
pub struct Manager {
    pub(crate) router: Arc<Router>,
    pub(crate) verifier: Arc<ProofVerifier>,
    pub(crate) set_proof_mode: SetProofMode,
    pub(crate) root_hashes: Arc<RwLock<HashMap<String, RootHash>>>,
    pub(crate) root_accumulators: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    pub(crate) prefix_migrations: Arc<RwLock<HashMap<String, PrefixMigrationState>>>,
    pub(crate) client_pool:
        Arc<RwLock<HashMap<String, Arc<OnceCell<StoragerServiceClient<Channel>>>>>>,
    pub(crate) boolean_query_stats: Arc<RwLock<BooleanQueryStats>>,
    pub(crate) split_migration_stats: Arc<RwLock<SplitMigrationStats>>,
    pub(crate) metrics_tag: Arc<RwLock<String>>,
    pub(crate) dataset: Arc<RwLock<String>>,
    pub(crate) concurrency: Arc<RwLock<u32>>,
    pub(crate) total_uploads: Arc<RwLock<u64>>,
    pub(crate) total_queries: Arc<RwLock<u64>>,
    pub(crate) total_updates: Arc<RwLock<u64>>,
    pub(crate) report_file_path: Arc<RwLock<Option<PathBuf>>>,
    pub(crate) persistence_mode: Arc<RwLock<String>>,
    pub(crate) upload_prefix_report_file_path: Arc<RwLock<Option<PathBuf>>>,
    pub(crate) upload_prefix_import_counts: Arc<RwLock<HashMap<String, HashMap<String, u64>>>>,
    pub(crate) run_report_dirty: Arc<AtomicBool>,
    pub(crate) upload_prefix_report_dirty: Arc<AtomicBool>,
    pub(crate) subrequest_global_semaphore: Arc<Semaphore>,
    pub(crate) subrequest_local_semaphores: Arc<RwLock<HashMap<String, Arc<Semaphore>>>>,
    pub(crate) proof_task_semaphore: Arc<Semaphore>,
    pub(crate) max_inflight_subrequests: usize,
    pub(crate) max_inflight_per_storager: usize,
    pub(crate) storager_count: usize,
    pub(crate) reset_lock: Arc<AsyncRwLock<()>>,
    pub(crate) migration_lock: Arc<AsyncMutex<()>>,
    pub(crate) reset_notifier: Arc<Notify>,
    pub(crate) reset_in_progress: Arc<RwLock<bool>>,
    pub(crate) split_threshold: usize,
}

#[derive(Clone, Debug, Default)]
pub struct BooleanQueryStats {
    pub query_count: u64,
    pub storager_visits: u64,
    pub proof_generation_duration: Duration,
}

#[derive(Clone, Debug, Default)]
pub struct SplitMigrationStats {
    pub migration_count: u64,
    pub total_duration_ms: u64,
    pub last_duration_ms: u64,
    pub max_duration_ms: u64,
    pub total_io_read_bytes: u64,
    pub total_io_write_bytes: u64,
    pub last_io_src: ProcessIoStats,
    pub last_io_tgt: ProcessIoStats,
    pub last_io_payload_bytes: u64,
    pub max_io_total_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessIoStats {
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub read_ops: u64,
    pub write_ops: u64,
}

impl ProcessIoStats {
    pub fn total_bytes(&self) -> u64 {
        self.read_bytes.saturating_add(self.write_bytes)
    }
}

pub struct ResetStateGuard {
    _lock: OwnedRwLockWriteGuard<()>,
    reset_in_progress: Arc<RwLock<bool>>,
    reset_notifier: Arc<Notify>,
}

pub(crate) struct SubrequestPermit {
    _global: OwnedSemaphorePermit,
    _local: OwnedSemaphorePermit,
}

impl Drop for ResetStateGuard {
    fn drop(&mut self) {
        *self.reset_in_progress.write().unwrap() = false;
        self.reset_notifier.notify_waiters();
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        let stats = self.boolean_query_stats.read().unwrap().clone();
        if stats.query_count > 0 {
            self.write_boolean_query_report();
        }
        self.flush_dirty_reports();
    }
}

impl Manager {
    pub(crate) fn active_prefix_migration_for_keyword(
        &self,
        keyword: &str,
    ) -> Option<PrefixMigrationState> {
        let migrations = self.prefix_migrations.read().unwrap();
        migrations
            .values()
            .find(|state| {
                !state.confirmed
                    && state
                        .child_prefixes
                        .iter()
                        .any(|prefix| super::EPRing::keyword_matches_prefix(keyword, prefix))
            })
            .cloned()
    }

    #[allow(dead_code)]
    pub(crate) async fn wait_for_keyword_migration(&self, keyword: &str) {
        loop {
            if self.active_prefix_migration_for_keyword(keyword).is_none() {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    }

    pub(crate) async fn buffer_operation_during_migration(
        &self,
        operation: PendingOperation,
    ) -> Result<(), String> {
        let keyword = match &operation {
            PendingOperation::Add { keyword, .. } => keyword.clone(),
            PendingOperation::Delete { keyword, .. } => keyword.clone(),
            PendingOperation::Update { keyword, .. } => keyword.clone(),
        };

        let mut migrations = self.prefix_migrations.write().unwrap();
        for state in migrations.values_mut() {
            if !state.confirmed
                && state
                    .child_prefixes
                    .iter()
                    .any(|prefix| super::EPRing::keyword_matches_prefix(&keyword, prefix))
            {
                state.pending_operations.push(operation);
                return Ok(());
            }
        }
        Err("no active migration found for keyword".to_string())
    }

    fn normalize_addr(storager_addr: &str) -> String {
        if storager_addr.starts_with("http://") || storager_addr.starts_with("https://") {
            storager_addr.to_string()
        } else {
            format!("http://{}", storager_addr)
        }
    }

    fn loopback_fallback_addr(addr: &str) -> Option<String> {
        if addr.contains("127.0.0.1") {
            Some(addr.replace("127.0.0.1", "[::1]"))
        } else if addr.contains("[::1]") {
            Some(addr.replace("[::1]", "127.0.0.1"))
        } else {
            None
        }
    }

    async fn connect_storager_endpoint(
        &self,
        addr: &str,
    ) -> Result<StoragerServiceClient<Channel>, tonic::transport::Error> {
        let use_heavy_profile =
            matches!(self.ads_mode(), AdsMode::Mpt | AdsMode::AccTree | AdsMode::AccTrie);
        let request_timeout = if use_heavy_profile {
            env_duration_secs("MANAGER_HEAVY_STORAGER_RPC_TIMEOUT_SECS", 3600)
        } else {
            env_duration_secs("MANAGER_STORAGER_RPC_TIMEOUT_SECS", 600)
        };
        let connect_timeout = if use_heavy_profile {
            env_duration_secs("MANAGER_HEAVY_STORAGER_CONNECT_TIMEOUT_SECS", 30)
        } else {
            env_duration_secs("MANAGER_STORAGER_CONNECT_TIMEOUT_SECS", 30)
        };
        let tcp_keepalive = if use_heavy_profile {
            env_duration_secs("MANAGER_HEAVY_STORAGER_TCP_KEEPALIVE_SECS", 300)
        } else {
            env_duration_secs("MANAGER_STORAGER_TCP_KEEPALIVE_SECS", 120)
        };
        let http2_keepalive_interval = if use_heavy_profile {
            env_optional_duration_secs(
                "MANAGER_HEAVY_STORAGER_HTTP2_KEEPALIVE_INTERVAL_SECS",
                Some(120),
            )
        } else {
            env_optional_duration_secs("MANAGER_STORAGER_HTTP2_KEEPALIVE_INTERVAL_SECS", Some(60))
        };
        let keep_alive_timeout = if use_heavy_profile {
            env_optional_duration_secs("MANAGER_HEAVY_STORAGER_KEEPALIVE_TIMEOUT_SECS", Some(3600))
        } else {
            env_optional_duration_secs("MANAGER_STORAGER_KEEPALIVE_TIMEOUT_SECS", Some(120))
        };
        let mut endpoint = tonic::transport::Endpoint::from_shared(addr.to_string())?
            .timeout(request_timeout)
            .connect_timeout(connect_timeout)
            .tcp_keepalive(Some(tcp_keepalive))
            .concurrency_limit(256);

        if let Some(interval) = http2_keepalive_interval {
            endpoint = endpoint.http2_keep_alive_interval(interval);
            if let Some(timeout) = keep_alive_timeout {
                endpoint = endpoint.keep_alive_timeout(timeout);
            }
        }

        StoragerServiceClient::connect(endpoint).await
    }

    pub fn new(
        storager_addrs: Vec<String>,
        ads_mode: AdsMode,
        set_proof_mode: SetProofMode,
        split_threshold: usize,
    ) -> Self {
        Self::new_with_route_mode(
            storager_addrs,
            ads_mode,
            set_proof_mode,
            split_threshold,
            RouteMode::Epring,
        )
    }

    pub fn new_with_route_mode(
        storager_addrs: Vec<String>,
        ads_mode: AdsMode,
        set_proof_mode: SetProofMode,
        split_threshold: usize,
        route_mode: RouteMode,
    ) -> Self {
        let router = Arc::new(Router::new_with_mode(
            storager_addrs,
            split_threshold,
            route_mode,
        ));
        let verifier = Arc::new(ProofVerifier::new(ads_mode));
        let root_hashes = Arc::new(RwLock::new(HashMap::new()));
        let root_accumulators = Arc::new(RwLock::new(HashMap::new()));
        let prefix_migrations = Arc::new(RwLock::new(HashMap::new()));
        let client_pool = Arc::new(RwLock::new(HashMap::new()));
        let boolean_query_stats = Arc::new(RwLock::new(BooleanQueryStats::default()));
        let split_migration_stats = Arc::new(RwLock::new(SplitMigrationStats::default()));
        let metrics_tag = Arc::new(RwLock::new("manager".to_string()));
        let dataset = Arc::new(RwLock::new("default".to_string()));
        let concurrency = Arc::new(RwLock::new(1));
        let total_uploads = Arc::new(RwLock::new(0));
        let total_queries = Arc::new(RwLock::new(0));
        let total_updates = Arc::new(RwLock::new(0));
        let report_file_path = Arc::new(RwLock::new(None));
        let persistence_mode = Arc::new(RwLock::new("unknown".to_string()));
        let upload_prefix_report_file_path = Arc::new(RwLock::new(None));
        let upload_prefix_import_counts = Arc::new(RwLock::new(HashMap::new()));
        let run_report_dirty = Arc::new(AtomicBool::new(false));
        let upload_prefix_report_dirty = Arc::new(AtomicBool::new(false));
        let storager_count = std::env::var("MANAGER_STORAGER_COUNT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or_else(|| router.storager_count());
        let default_global_inflight = std::cmp::max(storager_count.saturating_mul(8), 8);
        let max_inflight_subrequests =
            env_positive_usize("MANAGER_MAX_INFLIGHT_SUBREQUESTS", default_global_inflight);
        let max_inflight_per_storager = env_positive_usize("MANAGER_MAX_INFLIGHT_PER_STORAGER", 8);
        let default_blocking_proof_tasks = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4);
        let max_blocking_proof_tasks = env_positive_usize(
            "MANAGER_MAX_BLOCKING_PROOF_TASKS",
            default_blocking_proof_tasks,
        );
        let subrequest_global_semaphore = Arc::new(Semaphore::new(max_inflight_subrequests));
        let subrequest_local_semaphores = Arc::new(RwLock::new(HashMap::new()));
        let proof_task_semaphore = Arc::new(Semaphore::new(max_blocking_proof_tasks));
        let reset_lock = Arc::new(AsyncRwLock::new(()));
        let migration_lock = Arc::new(AsyncMutex::new(()));
        let reset_notifier = Arc::new(Notify::new());
        let reset_in_progress = Arc::new(RwLock::new(false));

        Manager {
            router,
            verifier,
            set_proof_mode,
            root_hashes,
            root_accumulators,
            prefix_migrations,
            client_pool,
            boolean_query_stats,
            split_migration_stats,
            metrics_tag,
            dataset,
            concurrency,
            total_uploads,
            total_queries,
            total_updates,
            report_file_path,
            persistence_mode,
            upload_prefix_report_file_path,
            upload_prefix_import_counts,
            run_report_dirty,
            upload_prefix_report_dirty,
            subrequest_global_semaphore,
            subrequest_local_semaphores,
            proof_task_semaphore,
            max_inflight_subrequests,
            max_inflight_per_storager,
            storager_count,
            reset_lock,
            migration_lock,
            reset_notifier,
            reset_in_progress,
            split_threshold,
        }
    }

    pub fn set_metrics_tag(&self, tag: impl Into<String>) {
        *self.metrics_tag.write().unwrap() = tag.into();
    }

    pub fn set_run_metadata(
        &self,
        dataset: impl Into<String>,
        concurrency: u32,
        total_uploads: u32,
        total_queries: u32,
        total_updates: u32,
    ) {
        *self.dataset.write().unwrap() = dataset.into();
        *self.concurrency.write().unwrap() = concurrency;
        if total_uploads > 0 {
            *self.total_uploads.write().unwrap() = total_uploads as u64;
        }
        if total_queries > 0 {
            *self.total_queries.write().unwrap() = total_queries as u64;
        }
        if total_updates > 0 {
            *self.total_updates.write().unwrap() = total_updates as u64;
        }
    }

    pub(crate) fn run_metadata_snapshot(&self) -> (String, u32, u64, u64, u64) {
        (
            self.dataset.read().unwrap().clone(),
            *self.concurrency.read().unwrap(),
            *self.total_uploads.read().unwrap(),
            *self.total_queries.read().unwrap(),
            *self.total_updates.read().unwrap(),
        )
    }

    pub(crate) fn run_metadata_snapshot_u32(&self) -> (String, u32, u32, u32, u32) {
        let (dataset, concurrency, total_uploads, total_queries, total_updates) =
            self.run_metadata_snapshot();
        (
            dataset,
            concurrency,
            std::cmp::min(total_uploads, u32::MAX as u64) as u32,
            std::cmp::min(total_queries, u32::MAX as u64) as u32,
            std::cmp::min(total_updates, u32::MAX as u64) as u32,
        )
    }

    fn report_file_name(&self, dataset: &str, concurrency: u32, route_mode: &str) -> String {
        let upload_records = *self.total_uploads.read().unwrap();
        let persistence_mode = self.persistence_mode.read().unwrap().clone();
        format!(
            "{}-{}-{}-{}-{}.txt",
            dataset, concurrency, route_mode, persistence_mode, upload_records
        )
    }

    pub(crate) fn record_persistence_mode(&self, mode: &str) {
        let normalized = mode.trim().to_lowercase();
        if normalized.is_empty() {
            return;
        }
        if normalized != "page" && normalized != "kvdb" {
            return;
        }

        let mut guard = self.persistence_mode.write().unwrap();
        Self::merge_persistence_mode(&mut guard, &normalized);
    }

    pub(crate) fn record_persistence_modes<I>(&self, modes: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut guard = self.persistence_mode.write().unwrap();
        for mode in modes {
            let normalized = mode.trim().to_lowercase();
            if normalized.is_empty() {
                continue;
            }
            if normalized != "page" && normalized != "kvdb" {
                continue;
            }
            Self::merge_persistence_mode(&mut guard, &normalized);
        }
    }

    pub(crate) fn persistence_mode_snapshot(&self) -> String {
        self.persistence_mode.read().unwrap().clone()
    }

    pub(crate) fn record_split_migration_duration(&self, duration: std::time::Duration) {
        let ms = duration.as_millis();
        let ms_u64 = if ms > u64::MAX as u128 {
            u64::MAX
        } else {
            ms as u64
        };

        let mut stats = self.split_migration_stats.write().unwrap();
        stats.migration_count = stats.migration_count.saturating_add(1);
        stats.total_duration_ms = stats.total_duration_ms.saturating_add(ms_u64);
        stats.last_duration_ms = ms_u64;
        stats.max_duration_ms = std::cmp::max(stats.max_duration_ms, ms_u64);
    }

    pub(crate) fn record_split_migration_io(
        &self,
        src: ProcessIoStats,
        tgt: ProcessIoStats,
        payload_bytes: u64,
    ) {
        let mut stats = self.split_migration_stats.write().unwrap();
        stats.total_io_read_bytes = stats
            .total_io_read_bytes
            .saturating_add(src.read_bytes.saturating_add(tgt.read_bytes));
        stats.total_io_write_bytes = stats
            .total_io_write_bytes
            .saturating_add(src.write_bytes.saturating_add(tgt.write_bytes));
        stats.last_io_src = src;
        stats.last_io_tgt = tgt;
        stats.last_io_payload_bytes = payload_bytes;
        stats.max_io_total_bytes = std::cmp::max(
            stats.max_io_total_bytes,
            src.total_bytes().saturating_add(tgt.total_bytes()),
        );
    }

    pub(crate) fn write_run_report(&self) {
        self.run_report_dirty.store(true, Ordering::Release);
    }

    pub(crate) fn flush_run_report(&self) {
        if !self.run_report_dirty.swap(false, Ordering::AcqRel) {
            return;
        }

        let stats = self.boolean_query_stats.read().unwrap().clone();
        let average_storagers_per_boolean_query = if stats.query_count > 0 {
            stats.storager_visits as f64 / stats.query_count as f64
        } else {
            0.0
        };
        let average_query_proof_generation_ms = if stats.query_count > 0 {
            stats.proof_generation_duration.as_secs_f64() * 1000.0 / stats.query_count as f64
        } else {
            0.0
        };
        let tag = self.metrics_tag.read().unwrap().clone();
        let route_mode = self.router.route_mode().as_str().to_string();
        let persistence_mode = self.persistence_mode_snapshot();
        let split_stats = self.split_migration_stats.read().unwrap().clone();
        let (dataset, concurrency, total_uploads, total_queries, total_updates) =
            self.run_metadata_snapshot();

        let src_io = split_stats.last_io_src;
        let tgt_io = split_stats.last_io_tgt;
        let payload_mb = split_stats.last_io_payload_bytes as f64 / 1048576.0;
        let total_io_bytes = src_io.total_bytes().saturating_add(tgt_io.total_bytes());
        let total_io_mb = total_io_bytes as f64 / 1048576.0;
        let amp_ratio = if split_stats.last_io_payload_bytes > 0 {
            total_io_bytes as f64 / split_stats.last_io_payload_bytes as f64
        } else {
            0.0
        };
        let amp_mb = (total_io_mb - payload_mb).max(0.0);

        let split_io_src_line = format!(
            "read_mb:{:.3},write_mb:{:.3},read_ops:{},write_ops:{}",
            src_io.read_bytes as f64 / 1048576.0,
            src_io.write_bytes as f64 / 1048576.0,
            src_io.read_ops,
            src_io.write_ops
        );
        let split_io_tgt_line = format!(
            "read_mb:{:.3},write_mb:{:.3},read_ops:{},write_ops:{}",
            tgt_io.read_bytes as f64 / 1048576.0,
            tgt_io.write_bytes as f64 / 1048576.0,
            tgt_io.read_ops,
            tgt_io.write_ops
        );
        let split_io_line = format!(
            "payload_mb:{:.3},io_total_mb:{:.3},io_amp_mb:{:.3},io_amp_ratio:{:.3}",
            payload_mb, total_io_mb, amp_mb, amp_ratio
        );
        let report = format!(
            "manager_tag={}\ndataset={}\nconcurrency={}\nroute_mode={}\npersistence_mode={}\nupload_record_count={}\ntotal_uploads={}\ntotal_queries={}\ntotal_updates={}\nstorager_count={}\nboolean_query_count={}\ntotal_storager_visits={}\naverage_storagers_per_boolean_query={:.3}\naverage_query_proof_generation_ms={:.3}\nsplit_migration_count={}\nsplit_migration_total_duration_ms={}\nsplit_migration_last_duration_ms={}\nsplit_migration_max_duration_ms={}\nsplit_migration_io_src={}\nsplit_migration_io_tgt={}\nsplit_migration_io={}\nsplit_migration_io_total_read_mb={:.3}\nsplit_migration_io_total_write_mb={:.3}\n",
            tag,
            dataset,
            concurrency,
            route_mode,
            persistence_mode,
            total_uploads,
            total_uploads,
            total_queries,
            total_updates,
            self.storager_count,
            stats.query_count,
            stats.storager_visits,
            average_storagers_per_boolean_query,
            average_query_proof_generation_ms,
            split_stats.migration_count,
            split_stats.total_duration_ms,
            split_stats.last_duration_ms,
            split_stats.max_duration_ms,
            split_io_src_line,
            split_io_tgt_line,
            split_io_line,
            split_stats.total_io_read_bytes as f64 / 1048576.0,
            split_stats.total_io_write_bytes as f64 / 1048576.0
        );

        let existing_path = self.report_file_path.read().unwrap().clone();
        if let Some(path) = existing_path {
            if let Err(err) = fs::write(&path, &report) {
                eprintln!(
                    "failed to write manager metrics report {}: {}",
                    path.display(),
                    err
                );
            }
            return;
        }

        let file_name = self.report_file_name(&dataset, concurrency, &route_mode);
        match metrics_output::write_scoped_report_file(
            &["manager", self.verifier.ads_mode().as_str()],
            &file_name,
            &report,
        ) {
            Ok(path) => {
                *self.report_file_path.write().unwrap() = Some(path);
            }
            Err(err) => {
                eprintln!(
                    "failed to write manager metrics report {}: {}",
                    file_name, err
                );
            }
        }
    }

    fn upload_prefix_report_file_name(&self) -> String {
        format!(
            "upload-prefix-imports-{}.txt",
            metrics_output::timestamp_token()
        )
    }

    pub(crate) fn record_upload_prefix_import(&self, node_name: &str, prefix: &str, count: u64) {
        if count == 0 {
            return;
        }

        let mut counts = self.upload_prefix_import_counts.write().unwrap();
        let node_counts = counts.entry(node_name.to_string()).or_default();
        let entry = node_counts.entry(prefix.to_string()).or_insert(0);
        *entry = entry.saturating_add(count);
    }

    pub(crate) fn write_upload_prefix_import_report(&self) {
        self.upload_prefix_report_dirty
            .store(true, Ordering::Release);
    }

    pub(crate) fn flush_upload_prefix_import_report(&self) {
        if !self
            .upload_prefix_report_dirty
            .swap(false, Ordering::AcqRel)
        {
            return;
        }

        let counts = self.upload_prefix_import_counts.read().unwrap();
        let mut lines = Vec::new();
        let mut nodes: Vec<_> = counts.keys().cloned().collect();
        nodes.sort();

        for node_name in nodes {
            if let Some(prefix_counts) = counts.get(&node_name) {
                let mut prefixes: Vec<_> = prefix_counts.keys().cloned().collect();
                prefixes.sort();
                for prefix in prefixes {
                    if let Some(record_count) = prefix_counts.get(&prefix) {
                        lines.push(format!("{},{},{}", node_name, prefix, record_count));
                    }
                }
            }
        }

        let report = if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        };

        let existing_path = self.upload_prefix_report_file_path.read().unwrap().clone();
        if let Some(path) = existing_path {
            if let Err(err) = fs::write(&path, &report) {
                eprintln!(
                    "failed to write upload prefix import report {}: {}",
                    path.display(),
                    err
                );
            }
            return;
        }

        let file_name = self.upload_prefix_report_file_name();
        match metrics_output::write_scoped_log_file(&[], &file_name, &report) {
            Ok(path) => {
                *self.upload_prefix_report_file_path.write().unwrap() = Some(path);
            }
            Err(err) => {
                eprintln!(
                    "failed to write upload prefix import report {}: {}",
                    file_name, err
                );
            }
        }
    }

    pub(crate) fn flush_dirty_reports(&self) {
        self.flush_run_report();
        self.flush_upload_prefix_import_report();
    }

    pub fn spawn_background_report_flushers(
        &self,
        metrics_interval: Duration,
        prefix_interval: Duration,
    ) {
        let manager_metrics_flusher = self.clone();
        tokio::spawn(async move {
            loop {
                sleep(metrics_interval).await;
                manager_metrics_flusher.flush_run_report();
            }
        });

        let manager_prefix_flusher = self.clone();
        tokio::spawn(async move {
            loop {
                sleep(prefix_interval).await;
                manager_prefix_flusher.flush_upload_prefix_import_report();
            }
        });
    }

    pub(crate) async fn begin_reset(&self) -> ResetStateGuard {
        let guard = self.reset_lock.clone().write_owned().await;
        *self.reset_in_progress.write().unwrap() = true;
        ResetStateGuard {
            _lock: guard,
            reset_in_progress: Arc::clone(&self.reset_in_progress),
            reset_notifier: Arc::clone(&self.reset_notifier),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn get_storager_for_keyword(&self, keyword: &str) -> Option<(String, String)> {
        self.router.get_storager_for_keyword(keyword)
    }

    pub(crate) fn route_keyword(&self, keyword: &str) -> Option<RouteTarget> {
        self.router.route_keyword(keyword)
    }

    pub(crate) fn query_route_keyword(&self, keyword: &str) -> Option<RouteTarget> {
        if let Some(migration) = self.active_prefix_migration_for_keyword(keyword) {
            let prefix = migration
                .child_prefixes
                .into_iter()
                .find(|prefix| super::EPRing::keyword_matches_prefix(keyword, prefix))
                .unwrap_or_default();
            return Some(RouteTarget {
                prefix,
                node_name: migration.source_node,
                addr: migration.source_addr,
            });
        }

        self.route_keyword(keyword)
    }

    pub(crate) fn write_route_keyword(&self, keyword: &str) -> Option<RouteTarget> {
        self.route_keyword(keyword)
    }

    pub(crate) fn max_inflight_subrequests(&self) -> usize {
        self.max_inflight_subrequests
    }

    pub(crate) async fn run_blocking_proof_task<T, F>(
        &self,
        task_name: &'static str,
        task: F,
    ) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        let permit = self
            .proof_task_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| format!("failed to acquire {} permit: {}", task_name, err))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            task()
        })
        .await
        .map_err(|err| format!("{} task join error: {}", task_name, err))?
    }

    pub(crate) async fn verify_proof_blocking(
        &self,
        proof: Vec<u8>,
        root_hash: Vec<u8>,
    ) -> Result<bool, String> {
        let verifier = Arc::clone(&self.verifier);
        self.run_blocking_proof_task("proof verification", move || {
            Ok(verifier.verify(&proof, &root_hash))
        })
        .await
    }

    pub(crate) async fn verify_query_response_blocking(
        &self,
        proof: Vec<u8>,
        root_hash: Vec<u8>,
        fids: Vec<String>,
    ) -> Result<bool, String> {
        let verifier = Arc::clone(&self.verifier);
        self.run_blocking_proof_task("query proof verification", move || {
            Ok(verifier.verify(&proof, &root_hash)
                && verifier.verify_query_result_fids(&proof, &fids))
        })
        .await
    }

    pub(crate) fn update_root_hash(&self, storager_name: String, root_hash: RootHash) {
        let mut hashes = self
            .root_hashes
            .write()
            .expect("Failed to acquire write lock on root_hashes");
        hashes.insert(storager_name, root_hash);
    }

    pub(crate) fn apply_root_state_updates<I>(&self, updates: I)
    where
        I: IntoIterator<Item = (String, RootHash, Vec<u8>)>,
    {
        let mut hashes = self
            .root_hashes
            .write()
            .expect("Failed to acquire write lock on root_hashes");
        let mut accumulators = self
            .root_accumulators
            .write()
            .expect("Failed to acquire write lock on root_accumulators");
        for (storager_name, root_hash, root_accumulator) in updates {
            hashes.insert(storager_name.clone(), root_hash);
            accumulators.insert(storager_name, root_accumulator);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn get_root_hash(&self, storager_name: &str) -> Option<RootHash> {
        let hashes = self
            .root_hashes
            .read()
            .expect("Failed to acquire read lock on root_hashes");
        hashes.get(storager_name).cloned()
    }

    pub(crate) fn update_root_accumulator(&self, storager_name: String, root_accumulator: Vec<u8>) {
        let mut accumulators = self
            .root_accumulators
            .write()
            .expect("Failed to acquire write lock on root_accumulators");
        accumulators.insert(storager_name, root_accumulator);
    }

    pub(crate) fn record_boolean_query(&self, storager_count: usize) {
        let mut stats = self.boolean_query_stats.write().unwrap();
        stats.query_count = stats.query_count.saturating_add(1);
        stats.storager_visits = stats.storager_visits.saturating_add(storager_count as u64);
    }

    pub(crate) fn record_boolean_query_proof_generation(&self, duration: Duration) {
        let mut stats = self.boolean_query_stats.write().unwrap();
        stats.proof_generation_duration += duration;
    }

    pub(crate) fn write_boolean_query_report(&self) {
        self.write_run_report();
    }

    #[allow(dead_code)]
    pub(crate) fn get_root_accumulator(&self, storager_name: &str) -> Option<Vec<u8>> {
        let accumulators = self
            .root_accumulators
            .read()
            .expect("Failed to acquire read lock on root_accumulators");
        accumulators.get(storager_name).cloned()
    }

    pub(crate) fn root_summary_for_values(
        &self,
        root_hash: &[u8],
        root_accumulator: &[u8],
    ) -> Vec<u8> {
        if !root_accumulator.is_empty() {
            root_accumulator.to_vec()
        } else {
            root_hash.to_vec()
        }
    }

    #[allow(dead_code)]
    pub(crate) fn root_summary_for_storager(&self, storager_name: &str) -> Vec<u8> {
        if let Some(root_accumulator) = self.get_root_accumulator(storager_name) {
            if !root_accumulator.is_empty() {
                return root_accumulator;
            }
        }

        self.get_root_hash(storager_name).unwrap_or_default()
    }

    pub(crate) fn record_prefix_insert(
        &self,
        keyword: &str,
        prefix: &str,
        node_name: &str,
        root_summary: Vec<u8>,
    ) -> Option<PrefixSplitPlan> {
        self.router
            .record_insert(keyword, prefix, node_name, root_summary)
    }

    pub(crate) fn presplit_empty_prefixes(
        &self,
        prefix_counts: &std::collections::HashMap<String, usize>,
    ) -> Vec<PrefixSplitPlan> {
        self.router.presplit_empty_prefixes(prefix_counts)
    }

    pub(crate) fn record_prefix_delete(&self, keyword: &str, prefix: &str, root_summary: Vec<u8>) {
        self.router.record_delete(keyword, prefix, root_summary);
    }

    pub(crate) fn update_prefix_summary(&self, prefix: &str, root_summary: Vec<u8>) {
        self.router.update_prefix_summary(prefix, root_summary);
    }

    pub(crate) fn update_prefix_summaries<I>(&self, updates: I)
    where
        I: IntoIterator<Item = (String, Vec<u8>)>,
    {
        self.router.update_prefix_summaries(updates);
    }

    pub(crate) fn clear_keyword_overrides_for_prefix(&self, prefix: &str) {
        self.router.clear_keyword_overrides_for_prefix(prefix);
    }

    #[allow(dead_code)]
    pub(crate) fn combine_proofs(&self, proofs: &[Vec<u8>]) -> Vec<u8> {
        self.verifier.combine_proofs(proofs)
    }

    pub fn ads_mode(&self) -> AdsMode {
        self.verifier.ads_mode()
    }

    pub fn set_proof_mode(&self) -> SetProofMode {
        self.set_proof_mode
    }

    pub fn get_storagers(&self) -> Vec<(String, String)> {
        self.router.get_all_storagers()
    }

    pub fn route_mode(&self) -> RouteMode {
        self.router.route_mode()
    }

    pub(crate) async fn get_storager_client(
        &self,
        storager_addr: &str,
    ) -> Result<StoragerServiceClient<Channel>, tonic::transport::Error> {
        let addr_with_scheme = Self::normalize_addr(storager_addr);

        let cell = {
            let mut pool = self
                .client_pool
                .write()
                .expect("Failed to acquire write lock on client_pool");
            pool.entry(addr_with_scheme.clone())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };

        let client = cell
            .get_or_try_init(|| async {
                match self.connect_storager_endpoint(&addr_with_scheme).await {
                    Ok(client) => Ok(client),
                    Err(primary_err) => {
                        if let Some(fallback_addr) = Self::loopback_fallback_addr(&addr_with_scheme)
                        {
                            eprintln!(
                                "Storager connect failed on {}, retrying with {}",
                                addr_with_scheme, fallback_addr
                            );
                            match self.connect_storager_endpoint(&fallback_addr).await {
                                Ok(client) => Ok(client),
                                Err(_) => Err(primary_err),
                            }
                        } else {
                            Err(primary_err)
                        }
                    }
                }
            })
            .await?;

        Ok(client.clone())
    }

    pub(crate) async fn acquire_subrequest_permits(
        &self,
        storager_addr: &str,
    ) -> Result<SubrequestPermit, String> {
        let global = self
            .subrequest_global_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| format!("failed to acquire global subrequest permit: {}", err))?;
        let normalized_addr = Self::normalize_addr(storager_addr);
        let local_semaphore = {
            let mut semaphores = self.subrequest_local_semaphores.write().unwrap();
            semaphores
                .entry(normalized_addr)
                .or_insert_with(|| Arc::new(Semaphore::new(self.max_inflight_per_storager)))
                .clone()
        };
        let local = local_semaphore
            .acquire_owned()
            .await
            .map_err(|err| format!("failed to acquire storager subrequest permit: {}", err))?;

        Ok(SubrequestPermit {
            _global: global,
            _local: local,
        })
    }

    fn merge_persistence_mode(current: &mut String, normalized: &str) {
        if current.as_str() == "unknown" {
            *current = normalized.to_string();
            return;
        }
        if current.as_str() != normalized {
            *current = "mixed".to_string();
        }
    }
}
