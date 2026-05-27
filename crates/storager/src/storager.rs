use crate::ads::{AccTreeAds, AccTrieAds, AdsOperations, MestAds, MptAds};
use common::metrics_output;
use std::collections::HashMap;
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
        }
    }

    pub fn set_metrics_tag(&self, tag: impl Into<String>) {
        *self.metrics_tag.write().unwrap() = tag.into();
    }

    pub fn set_ads_mode(&self, ads_mode: impl Into<String>) {
        *self.ads_mode.write().unwrap() = ads_mode.into();
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
        let record_count = ads.record_count();
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
        let report = format!(
            "storager_tag={}\nstorager_id={}\nupload_kv_pairs_total={}\nrecord_count={}\nstorage_bytes={}\nquery_count={}\naverage_query_proof_size_bytes={:.3}\n",
            tag,
            storager_id,
            upload_kv_pairs_total,
            record_count,
            storage_bytes,
            stats.query_count,
            avg_query_proof_size
        );
        let file_name = format!("{}.txt", storager_id);
        if let Err(err) =
            metrics_output::write_scoped_report_file(&["storagers", &ads_mode], &file_name, &report)
        {
            eprintln!(
                "failed to write storager metrics report {}: {}",
                file_name, err
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
