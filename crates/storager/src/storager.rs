use crate::ads::{AccTreeAds, AccTrieAds, AdsOperations, MestAds, MptAds};
use common::metrics_output;
use std::fs;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::Notify;

pub struct Storager {
    pub(crate) ads: Arc<RwLock<Box<dyn AdsOperations>>>,
    pub(crate) active_prefix_queries: Arc<RwLock<HashMap<String, usize>>>,
    pub(crate) active_mutations: Arc<RwLock<usize>>,
    pub(crate) retained_prefix_notifier: Arc<Notify>,
    pub(crate) query_stats: Arc<RwLock<StoragerQueryStats>>,
    pub(crate) storager_id: Arc<RwLock<String>>,
    pub(crate) upload_kv_pairs_total: Arc<RwLock<u64>>,
    pub(crate) metrics_tag: Arc<RwLock<String>>,
    pub(crate) ads_mode: Arc<RwLock<String>>,
    pub(crate) route_mode: Arc<RwLock<String>>,
    pub(crate) dataset: Arc<RwLock<String>>,
    pub(crate) concurrency: Arc<RwLock<u32>>,
    pub(crate) total_uploads: Arc<RwLock<u64>>,
    pub(crate) total_queries: Arc<RwLock<u64>>,
    pub(crate) total_updates: Arc<RwLock<u64>>,
    pub(crate) report_metadata: Arc<RwLock<Option<(String, u32, u64, u64, u64)>>>,
    pub(crate) report_file_path: Arc<RwLock<Option<PathBuf>>>,
    pub(crate) report_record_count: Arc<RwLock<u64>>,
    pub(crate) report_record_count_after_update: Arc<RwLock<Option<u64>>>,
    pub(crate) acctrie_persistence_mode: Arc<RwLock<String>>,
}

#[derive(Default)]
pub(crate) struct StoragerQueryStats {
    pub query_count: u64,
    pub query_proof_bytes: u64,
}

pub(crate) struct PrefixQueryGuard {
    pub(crate) prefixes: Vec<String>,
    pub(crate) active_prefix_queries: Arc<RwLock<HashMap<String, usize>>>,
    pub(crate) retained_prefix_notifier: Arc<Notify>,
}

pub(crate) struct MutationGuard {
    pub(crate) active_mutations: Arc<RwLock<usize>>,
    pub(crate) retained_prefix_notifier: Arc<Notify>,
}

impl Drop for PrefixQueryGuard {
    fn drop(&mut self) {
        let mut queries = self.active_prefix_queries.write().unwrap();
        for prefix in &self.prefixes {
            if let Some(active) = queries.get_mut(prefix) {
                *active = active.saturating_sub(1);
                if *active == 0 {
                    queries.remove(prefix);
                }
            }
        }
        self.retained_prefix_notifier.notify_waiters();
    }
}

impl Drop for MutationGuard {
    fn drop(&mut self) {
        let mut active = self.active_mutations.write().unwrap();
        *active = active.saturating_sub(1);
        self.retained_prefix_notifier.notify_waiters();
    }
}

impl Storager {
    pub fn new() -> Self {
        Self::with_mest()
    }

    pub fn with_mpt() -> Self {
        let ads: Box<dyn AdsOperations> = Box::new(MptAds::new());
        Storager {
            ads: Arc::new(RwLock::new(ads)),
            active_prefix_queries: Arc::new(RwLock::new(HashMap::new())),
            active_mutations: Arc::new(RwLock::new(0)),
            retained_prefix_notifier: Arc::new(Notify::new()),
            query_stats: Arc::new(RwLock::new(StoragerQueryStats::default())),
            storager_id: Arc::new(RwLock::new("storager".to_string())),
            upload_kv_pairs_total: Arc::new(RwLock::new(0)),
            metrics_tag: Arc::new(RwLock::new("storager".to_string())),
            ads_mode: Arc::new(RwLock::new("mpt".to_string())),
            route_mode: Arc::new(RwLock::new("unknown".to_string())),
            dataset: Arc::new(RwLock::new("default".to_string())),
            concurrency: Arc::new(RwLock::new(1)),
            total_uploads: Arc::new(RwLock::new(0)),
            total_queries: Arc::new(RwLock::new(0)),
            total_updates: Arc::new(RwLock::new(0)),
            report_metadata: Arc::new(RwLock::new(None)),
            report_file_path: Arc::new(RwLock::new(None)),
            report_record_count: Arc::new(RwLock::new(0)),
            report_record_count_after_update: Arc::new(RwLock::new(None)),
            acctrie_persistence_mode: Arc::new(RwLock::new("kvdb".to_string())),
        }
    }

    pub fn with_mest() -> Self {
        let ads: Box<dyn AdsOperations> = Box::new(MestAds::new_default());
        Storager {
            ads: Arc::new(RwLock::new(ads)),
            active_prefix_queries: Arc::new(RwLock::new(HashMap::new())),
            active_mutations: Arc::new(RwLock::new(0)),
            retained_prefix_notifier: Arc::new(Notify::new()),
            query_stats: Arc::new(RwLock::new(StoragerQueryStats::default())),
            storager_id: Arc::new(RwLock::new("storager".to_string())),
            upload_kv_pairs_total: Arc::new(RwLock::new(0)),
            metrics_tag: Arc::new(RwLock::new("storager".to_string())),
            ads_mode: Arc::new(RwLock::new("mest".to_string())),
            route_mode: Arc::new(RwLock::new("unknown".to_string())),
            dataset: Arc::new(RwLock::new("default".to_string())),
            concurrency: Arc::new(RwLock::new(1)),
            total_uploads: Arc::new(RwLock::new(0)),
            total_queries: Arc::new(RwLock::new(0)),
            total_updates: Arc::new(RwLock::new(0)),
            report_metadata: Arc::new(RwLock::new(None)),
            report_file_path: Arc::new(RwLock::new(None)),
            report_record_count: Arc::new(RwLock::new(0)),
            report_record_count_after_update: Arc::new(RwLock::new(None)),
            acctrie_persistence_mode: Arc::new(RwLock::new("kvdb".to_string())),
        }
    }

    pub fn with_acctrie() -> Self {
        let ads: Box<dyn AdsOperations> = Box::new(AccTrieAds::new());
        Storager {
            ads: Arc::new(RwLock::new(ads)),
            active_prefix_queries: Arc::new(RwLock::new(HashMap::new())),
            active_mutations: Arc::new(RwLock::new(0)),
            retained_prefix_notifier: Arc::new(Notify::new()),
            query_stats: Arc::new(RwLock::new(StoragerQueryStats::default())),
            storager_id: Arc::new(RwLock::new("storager".to_string())),
            upload_kv_pairs_total: Arc::new(RwLock::new(0)),
            metrics_tag: Arc::new(RwLock::new("storager".to_string())),
            ads_mode: Arc::new(RwLock::new("acctrie".to_string())),
            route_mode: Arc::new(RwLock::new("unknown".to_string())),
            dataset: Arc::new(RwLock::new("default".to_string())),
            concurrency: Arc::new(RwLock::new(1)),
            total_uploads: Arc::new(RwLock::new(0)),
            total_queries: Arc::new(RwLock::new(0)),
            total_updates: Arc::new(RwLock::new(0)),
            report_metadata: Arc::new(RwLock::new(None)),
            report_file_path: Arc::new(RwLock::new(None)),
            report_record_count: Arc::new(RwLock::new(0)),
            report_record_count_after_update: Arc::new(RwLock::new(None)),
            acctrie_persistence_mode: Arc::new(RwLock::new("page".to_string())),
        }
    }

    pub fn with_acctree() -> Self {
        let ads: Box<dyn AdsOperations> = Box::new(AccTreeAds::new());
        Storager {
            ads: Arc::new(RwLock::new(ads)),
            active_prefix_queries: Arc::new(RwLock::new(HashMap::new())),
            active_mutations: Arc::new(RwLock::new(0)),
            retained_prefix_notifier: Arc::new(Notify::new()),
            query_stats: Arc::new(RwLock::new(StoragerQueryStats::default())),
            storager_id: Arc::new(RwLock::new("storager".to_string())),
            upload_kv_pairs_total: Arc::new(RwLock::new(0)),
            metrics_tag: Arc::new(RwLock::new("storager".to_string())),
            ads_mode: Arc::new(RwLock::new("acctree".to_string())),
            route_mode: Arc::new(RwLock::new("unknown".to_string())),
            dataset: Arc::new(RwLock::new("default".to_string())),
            concurrency: Arc::new(RwLock::new(1)),
            total_uploads: Arc::new(RwLock::new(0)),
            total_queries: Arc::new(RwLock::new(0)),
            total_updates: Arc::new(RwLock::new(0)),
            report_metadata: Arc::new(RwLock::new(None)),
            report_file_path: Arc::new(RwLock::new(None)),
            report_record_count: Arc::new(RwLock::new(0)),
            report_record_count_after_update: Arc::new(RwLock::new(None)),
            acctrie_persistence_mode: Arc::new(RwLock::new("kvdb".to_string())),
        }
    }

    pub fn with_acctree_persistence(path: impl Into<std::path::PathBuf>) -> Self {
        let ads: Box<dyn AdsOperations> = Box::new(AccTreeAds::new_with_persistence(path.into()));
        Storager {
            ads: Arc::new(RwLock::new(ads)),
            active_prefix_queries: Arc::new(RwLock::new(HashMap::new())),
            active_mutations: Arc::new(RwLock::new(0)),
            retained_prefix_notifier: Arc::new(Notify::new()),
            query_stats: Arc::new(RwLock::new(StoragerQueryStats::default())),
            storager_id: Arc::new(RwLock::new("storager".to_string())),
            upload_kv_pairs_total: Arc::new(RwLock::new(0)),
            metrics_tag: Arc::new(RwLock::new("storager".to_string())),
            ads_mode: Arc::new(RwLock::new("acctree".to_string())),
            route_mode: Arc::new(RwLock::new("unknown".to_string())),
            dataset: Arc::new(RwLock::new("default".to_string())),
            concurrency: Arc::new(RwLock::new(1)),
            total_uploads: Arc::new(RwLock::new(0)),
            total_queries: Arc::new(RwLock::new(0)),
            total_updates: Arc::new(RwLock::new(0)),
            report_metadata: Arc::new(RwLock::new(None)),
            report_file_path: Arc::new(RwLock::new(None)),
            report_record_count: Arc::new(RwLock::new(0)),
            report_record_count_after_update: Arc::new(RwLock::new(None)),
            acctrie_persistence_mode: Arc::new(RwLock::new("kvdb".to_string())),
        }
    }

    pub fn with_mpt_persistence(path: impl Into<std::path::PathBuf>) -> Self {
        let ads: Box<dyn AdsOperations> = Box::new(MptAds::new_with_persistence(path.into()));
        Storager {
            ads: Arc::new(RwLock::new(ads)),
            active_prefix_queries: Arc::new(RwLock::new(HashMap::new())),
            active_mutations: Arc::new(RwLock::new(0)),
            retained_prefix_notifier: Arc::new(Notify::new()),
            query_stats: Arc::new(RwLock::new(StoragerQueryStats::default())),
            storager_id: Arc::new(RwLock::new("storager".to_string())),
            upload_kv_pairs_total: Arc::new(RwLock::new(0)),
            metrics_tag: Arc::new(RwLock::new("storager".to_string())),
            ads_mode: Arc::new(RwLock::new("mpt".to_string())),
            route_mode: Arc::new(RwLock::new("unknown".to_string())),
            dataset: Arc::new(RwLock::new("default".to_string())),
            concurrency: Arc::new(RwLock::new(1)),
            total_uploads: Arc::new(RwLock::new(0)),
            total_queries: Arc::new(RwLock::new(0)),
            total_updates: Arc::new(RwLock::new(0)),
            report_metadata: Arc::new(RwLock::new(None)),
            report_file_path: Arc::new(RwLock::new(None)),
            report_record_count: Arc::new(RwLock::new(0)),
            report_record_count_after_update: Arc::new(RwLock::new(None)),
            acctrie_persistence_mode: Arc::new(RwLock::new("kvdb".to_string())),
        }
    }

    pub fn with_acctrie_persistence(path: impl Into<std::path::PathBuf>) -> Self {
        let ads: Box<dyn AdsOperations> = Box::new(AccTrieAds::new_with_persistence(path.into()));
        Storager {
            ads: Arc::new(RwLock::new(ads)),
            active_prefix_queries: Arc::new(RwLock::new(HashMap::new())),
            active_mutations: Arc::new(RwLock::new(0)),
            retained_prefix_notifier: Arc::new(Notify::new()),
            query_stats: Arc::new(RwLock::new(StoragerQueryStats::default())),
            storager_id: Arc::new(RwLock::new("storager".to_string())),
            upload_kv_pairs_total: Arc::new(RwLock::new(0)),
            metrics_tag: Arc::new(RwLock::new("storager".to_string())),
            ads_mode: Arc::new(RwLock::new("acctrie".to_string())),
            route_mode: Arc::new(RwLock::new("unknown".to_string())),
            dataset: Arc::new(RwLock::new("default".to_string())),
            concurrency: Arc::new(RwLock::new(1)),
            total_uploads: Arc::new(RwLock::new(0)),
            total_queries: Arc::new(RwLock::new(0)),
            total_updates: Arc::new(RwLock::new(0)),
            report_metadata: Arc::new(RwLock::new(None)),
            report_file_path: Arc::new(RwLock::new(None)),
            report_record_count: Arc::new(RwLock::new(0)),
            report_record_count_after_update: Arc::new(RwLock::new(None)),
            acctrie_persistence_mode: Arc::new(RwLock::new("page".to_string())),
        }
    }

    pub fn with_acctrie_persistence_mode(
        path: impl Into<std::path::PathBuf>,
        persistence_mode: impl AsRef<str>,
    ) -> Self {
        let normalized_mode = match persistence_mode.as_ref().trim().to_lowercase().as_str() {
            "kvdb" => "kvdb".to_string(),
            _ => "page".to_string(),
        };
        let ads: Box<dyn AdsOperations> = Box::new(AccTrieAds::new_with_persistence_mode(
            path.into(),
            normalized_mode.clone(),
        ));
        Storager {
            ads: Arc::new(RwLock::new(ads)),
            active_prefix_queries: Arc::new(RwLock::new(HashMap::new())),
            active_mutations: Arc::new(RwLock::new(0)),
            retained_prefix_notifier: Arc::new(Notify::new()),
            query_stats: Arc::new(RwLock::new(StoragerQueryStats::default())),
            storager_id: Arc::new(RwLock::new("storager".to_string())),
            upload_kv_pairs_total: Arc::new(RwLock::new(0)),
            metrics_tag: Arc::new(RwLock::new("storager".to_string())),
            ads_mode: Arc::new(RwLock::new("acctrie".to_string())),
            route_mode: Arc::new(RwLock::new("unknown".to_string())),
            dataset: Arc::new(RwLock::new("default".to_string())),
            concurrency: Arc::new(RwLock::new(1)),
            total_uploads: Arc::new(RwLock::new(0)),
            total_queries: Arc::new(RwLock::new(0)),
            total_updates: Arc::new(RwLock::new(0)),
            report_metadata: Arc::new(RwLock::new(None)),
            report_file_path: Arc::new(RwLock::new(None)),
            report_record_count: Arc::new(RwLock::new(0)),
            report_record_count_after_update: Arc::new(RwLock::new(None)),
            acctrie_persistence_mode: Arc::new(RwLock::new(normalized_mode)),
        }
    }

    pub fn set_metrics_tag(&self, tag: impl Into<String>) {
        *self.metrics_tag.write().unwrap() = tag.into();
    }

    pub fn set_ads_mode(&self, ads_mode: impl Into<String>) {
        *self.ads_mode.write().unwrap() = ads_mode.into();
    }

    pub fn set_route_mode(&self, route_mode: impl Into<String>) {
        *self.route_mode.write().unwrap() = route_mode.into();
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

    fn run_metadata_snapshot(&self) -> (String, u32, u64, u64, u64) {
        (
            self.dataset.read().unwrap().clone(),
            *self.concurrency.read().unwrap(),
            *self.total_uploads.read().unwrap(),
            *self.total_queries.read().unwrap(),
            *self.total_updates.read().unwrap(),
        )
    }

    fn report_persistence_mode(&self, ads_mode: &str) -> String {
        if ads_mode.eq_ignore_ascii_case("acctrie")
            || ads_mode.eq_ignore_ascii_case("accumulator")
        {
            self.acctrie_persistence_mode.read().unwrap().clone()
        } else {
            "kvdb".to_string()
        }
    }

    pub(crate) fn persistence_mode(&self) -> String {
        let ads_mode = self.ads_mode.read().unwrap().clone();
        self.report_persistence_mode(&ads_mode)
    }

    pub(crate) fn begin_upload_report(&self) {
        let record_count = self.ads.read().unwrap().record_count() as u64;
        let metadata = self.run_metadata_snapshot();
        *self.report_metadata.write().unwrap() = Some(metadata);
        *self.report_record_count.write().unwrap() = record_count;
        *self.report_record_count_after_update.write().unwrap() = None;
    }

    pub(crate) fn record_after_update_count(&self) {
        let record_count = self.ads.read().unwrap().record_count() as u64;
        *self.report_record_count_after_update.write().unwrap() = Some(record_count);
    }

    pub fn set_storager_id(&self, storager_id: impl Into<String>) {
        *self.storager_id.write().unwrap() = storager_id.into();
    }

    pub fn record_upload_kv_pairs_total(&self, total: u32) {
        if total > 0 {
            *self.upload_kv_pairs_total.write().unwrap() = total as u64;
        }
    }

    pub(crate) fn record_query_metrics(&self, proof_size_bytes: usize) {
        let mut stats = self.query_stats.write().unwrap();
        stats.query_count = stats.query_count.saturating_add(1);
        stats.query_proof_bytes = stats
            .query_proof_bytes
            .saturating_add(proof_size_bytes as u64);
    }

    pub fn write_metrics_report(&self) {
        let stats = self.query_stats.read().unwrap();
        let ads = self.ads.read().unwrap();
        let storage_bytes = ads.storage_bytes();
        let avg_query_proof_size = if stats.query_count > 0 {
            stats.query_proof_bytes as f64 / stats.query_count as f64
        } else {
            0.0
        };
        let tag = self.metrics_tag.read().unwrap().clone();
        let storager_id = self.storager_id.read().unwrap().clone();
        let upload_kv_pairs_total = *self.upload_kv_pairs_total.read().unwrap();
        let ads_mode = self.ads_mode.read().unwrap().clone();
        let persistence_mode = self.report_persistence_mode(&ads_mode);
        let route_mode = self.route_mode.read().unwrap().clone();
        let record_count = *self.report_record_count.read().unwrap();
        let record_count_after_update = *self.report_record_count_after_update.read().unwrap();
        let record_count_after_update_line = record_count_after_update
            .map(|value| format!("record_count_after_update={}\n", value))
            .unwrap_or_default();
        let (dataset, concurrency, total_uploads, total_queries, total_updates) = self
            .report_metadata
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| self.run_metadata_snapshot());
        let report = format!(
            "storager_tag={}\nstorager_id={}\ndataset={}\nconcurrency={}\nroute_mode={}\npersistence_mode={}\ntotal_uploads={}\ntotal_queries={}\ntotal_updates={}\nupload_kv_pairs_total={}\nrecord_count={}\n{}storage_bytes={}\nquery_count={}\naverage_query_proof_size_bytes={:.3}\n",
            tag,
            storager_id,
            dataset,
            concurrency,
            route_mode,
            persistence_mode,
            total_uploads,
            total_queries,
            total_updates,
            upload_kv_pairs_total,
            record_count,
            record_count_after_update_line,
            storage_bytes,
            stats.query_count,
            avg_query_proof_size
        );
        let path = {
            let mut guard = self.report_file_path.write().unwrap();
            if let Some(path) = guard.clone() {
                path
            } else {
                let file_name = format!(
                    "{}-{}-{}-{}-{}-{}.txt",
                    storager_id,
                    dataset,
                    concurrency,
                    route_mode,
                    persistence_mode,
                    total_uploads
                );
                match metrics_output::write_scoped_report_file(
                    &["storagers", &ads_mode],
                    &file_name,
                    &report,
                ) {
                    Ok(path) => {
                        *guard = Some(path.clone());
                        return;
                    }
                    Err(err) => {
                        eprintln!(
                            "failed to write storager metrics report {}: {}",
                            file_name, err
                        );
                        return;
                    }
                }
            }
        };
        if let Err(err) = fs::write(&path, report) {
            eprintln!(
                "failed to write storager metrics report {}: {}",
                path.display(),
                err
            );
        }
    }

    pub(crate) fn begin_mutation(&self) -> MutationGuard {
        let mut active = self.active_mutations.write().unwrap();
        *active += 1;
        MutationGuard {
            active_mutations: Arc::clone(&self.active_mutations),
            retained_prefix_notifier: Arc::clone(&self.retained_prefix_notifier),
        }
    }

    pub(crate) fn begin_query_for_keyword(&self, keyword: &str) -> PrefixQueryGuard {
        self.begin_prefix_queries(vec![keyword.to_string()])
    }

    pub(crate) fn begin_prefix_queries(&self, prefixes: Vec<String>) -> PrefixQueryGuard {
        let mut queries = self.active_prefix_queries.write().unwrap();
        for prefix in &prefixes {
            *queries.entry(prefix.clone()).or_insert(0) += 1;
        }
        PrefixQueryGuard {
            prefixes,
            active_prefix_queries: Arc::clone(&self.active_prefix_queries),
            retained_prefix_notifier: Arc::clone(&self.retained_prefix_notifier),
        }
    }

    pub async fn wait_for_mutations_to_drain(&self) {
        loop {
            let active = *self.active_mutations.read().unwrap();
            if active == 0 {
                return;
            }
            self.retained_prefix_notifier.notified().await;
        }
    }

    pub async fn wait_for_prefix_queries_to_drain(&self, prefix: &str) {
        loop {
            let active = self
                .active_prefix_queries
                .read()
                .unwrap()
                .iter()
                .filter(|(tracked_prefix, _)| tracked_prefix.starts_with(prefix))
                .map(|(_, active)| *active)
                .sum::<usize>();
            if active == 0 {
                return;
            }
            self.retained_prefix_notifier.notified().await;
        }
    }

    pub fn from_config(ads_type: &str) -> Self {
        match ads_type.to_lowercase().as_str() {
            "mpt" => Self::with_mpt(),
            "mest" => Self::with_mest(),
            "acctrie" => Self::with_acctrie(),
            "acctree" => Self::with_acctree(),
            _ => {
                eprintln!("Unknown ADS type '{}', using default (MEST)", ads_type);
                Self::with_mest()
            }
        }
    }
}

impl Default for Storager {
    fn default() -> Self {
        Self::new()
    }
}
