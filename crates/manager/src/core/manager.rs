use super::{PrefixSplitPlan, RouteTarget, Router};
use common::rpc::storager_service_client::StoragerServiceClient;
use common::ProofVerifier;
use common::{metrics_output, AdsMode, RootHash, SetProofMode};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::OnceCell;
use tokio::sync::{Mutex as AsyncMutex, Notify, OwnedMutexGuard};
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
    pub(crate) metrics_tag: Arc<RwLock<String>>,
    pub(crate) storager_count: usize,
    pub(crate) reset_lock: Arc<AsyncMutex<()>>,
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

pub struct ResetStateGuard {
    _lock: OwnedMutexGuard<()>,
    reset_in_progress: Arc<RwLock<bool>>,
    reset_notifier: Arc<Notify>,
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
        let use_heavy_profile = matches!(self.ads_mode(), AdsMode::Mpt | AdsMode::AccTree);
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
            env_optional_duration_secs(
                "MANAGER_HEAVY_STORAGER_KEEPALIVE_TIMEOUT_SECS",
                Some(3600),
            )
        } else {
            env_optional_duration_secs("MANAGER_STORAGER_KEEPALIVE_TIMEOUT_SECS", Some(120))
        };
        let mut endpoint = tonic::transport::Endpoint::from_shared(addr.to_string())
            .expect("Failed to create endpoint from validated address")
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
        let router = Arc::new(Router::new(storager_addrs, split_threshold));
        let verifier = Arc::new(ProofVerifier::new(ads_mode));
        let root_hashes = Arc::new(RwLock::new(HashMap::new()));
        let root_accumulators = Arc::new(RwLock::new(HashMap::new()));
        let prefix_migrations = Arc::new(RwLock::new(HashMap::new()));
        let client_pool = Arc::new(RwLock::new(HashMap::new()));
        let boolean_query_stats = Arc::new(RwLock::new(BooleanQueryStats::default()));
        let metrics_tag = Arc::new(RwLock::new("manager".to_string()));
        let storager_count = std::env::var("MANAGER_STORAGER_COUNT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or_else(|| router.storager_count());
        let reset_lock = Arc::new(AsyncMutex::new(()));
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
            metrics_tag,
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

    pub(crate) async fn begin_reset(&self) -> ResetStateGuard {
        let guard = self.reset_lock.clone().lock_owned().await;
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

    pub(crate) fn verify_proof(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        self.verifier.verify(proof, root_hash)
    }

    pub(crate) fn update_root_hash(&self, storager_name: String, root_hash: RootHash) {
        let mut hashes = self
            .root_hashes
            .write()
            .expect("Failed to acquire write lock on root_hashes");
        hashes.insert(storager_name, root_hash);
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
        let stats = self.boolean_query_stats.read().unwrap().clone();
        let average = if stats.query_count > 0 {
            stats.storager_visits as f64 / stats.query_count as f64
        } else {
            0.0
        };
        let average_proof_generation_ms = if stats.query_count > 0 {
            stats.proof_generation_duration.as_secs_f64() * 1000.0 / stats.query_count as f64
        } else {
            0.0
        };
        let tag = self.metrics_tag.read().unwrap().clone();
        let report = format!(
            "manager_tag={}\nstorager_count={}\nboolean_query_count={}\ntotal_storager_visits={}\naverage_storagers_per_boolean_query={:.3}\naverage_query_proof_generation_ms={:.3}\n",
            tag,
            self.storager_count,
            stats.query_count,
            stats.storager_visits,
            average,
            average_proof_generation_ms
        );
        let file_name = format!("{}.txt", self.storager_count);
        if let Err(err) =
            metrics_output::write_scoped_report_file(
                &["manager", self.verifier.ads_mode().as_str()],
                &file_name,
                &report,
            )
        {
            eprintln!(
                "failed to write manager metrics report {}: {}",
                file_name, err
            );
        }
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
}
