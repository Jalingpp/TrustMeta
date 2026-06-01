use super::{ConsistentHashRing, EPRing, EPRingSplitEvent};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

const DEFAULT_CHRING_VIRTUAL_NODES_PER_NODE: usize = 150;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteMode {
    Epring,
    Chring,
}

impl RouteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RouteMode::Epring => "epring",
            RouteMode::Chring => "chring",
        }
    }
}

impl fmt::Display for RouteMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RouteMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "epring" => Ok(RouteMode::Epring),
            "chring" => Ok(RouteMode::Chring),
            other => Err(format!(
                "invalid route mode: {other}. expected epring|chring"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteTarget {
    pub prefix: String,
    pub node_name: String,
    pub addr: String,
}

#[derive(Debug, Clone)]
struct CachedRoute {
    route: RouteTarget,
    epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixSplitPlan {
    pub parent_prefix: String,
    pub source: RouteTarget,
    pub children: Vec<RouteTarget>,
}

enum RouterBackend {
    Epring {
        ring: Arc<RwLock<EPRing>>,
        keyword_overrides: Arc<RwLock<HashMap<String, RouteTarget>>>,
    },
    Chring {
        ring: Arc<RwLock<ConsistentHashRing>>,
    },
}

/// Router backed by either an expandable prefix ring or a consistent hash ring.
pub struct Router {
    storager_addrs: HashMap<String, String>,
    node_names: Vec<String>,
    node_addrs: Vec<String>,
    route_mode: RouteMode,
    backend: RouterBackend,
    route_cache: Arc<RwLock<HashMap<String, CachedRoute>>>,
    route_epoch: Arc<AtomicU64>,
}

impl Router {
    /// `split_threshold` controls when a prefix expands into a child ring.
    pub fn new(storager_addrs: Vec<String>, split_threshold: usize) -> Self {
        Self::new_with_mode(storager_addrs, split_threshold, RouteMode::Epring)
    }

    pub fn new_with_mode(
        storager_addrs: Vec<String>,
        split_threshold: usize,
        route_mode: RouteMode,
    ) -> Self {
        let mut node_names = Vec::with_capacity(storager_addrs.len());
        let mut node_addrs = Vec::with_capacity(storager_addrs.len());
        let mut addr_map = HashMap::with_capacity(storager_addrs.len());

        for (idx, addr) in storager_addrs.iter().enumerate() {
            let node_name = format!("storager-{}", idx);
            node_names.push(node_name.clone());
            node_addrs.push(addr.clone());
            addr_map.insert(node_name, addr.clone());
        }

        let backend = match route_mode {
            RouteMode::Epring => RouterBackend::Epring {
                ring: Arc::new(RwLock::new(EPRing::new(
                    &node_names,
                    split_threshold as u64,
                ))),
                keyword_overrides: Arc::new(RwLock::new(HashMap::new())),
            },
            RouteMode::Chring => RouterBackend::Chring {
                ring: Arc::new(RwLock::new(Self::build_consistent_hash_ring(&node_names))),
            },
        };

        Self {
            storager_addrs: addr_map,
            node_names,
            node_addrs,
            route_mode,
            backend,
            route_cache: Arc::new(RwLock::new(HashMap::new())),
            route_epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn route_mode(&self) -> RouteMode {
        self.route_mode
    }

    pub fn route_keyword(&self, keyword: &str) -> Option<RouteTarget> {
        loop {
            let epoch = self.route_epoch.load(Ordering::Acquire);
            if epoch % 2 == 1 {
                std::thread::yield_now();
                continue;
            }

            if let Some(route) = self
                .route_cache
                .read()
                .expect("Failed to acquire read lock on route_cache")
                .get(keyword)
                .filter(|cached| cached.epoch == epoch)
                .map(|cached| cached.route.clone())
            {
                if self.route_epoch.load(Ordering::Acquire) == epoch {
                    return Some(route);
                }
                continue;
            }

            if let RouterBackend::Epring {
                keyword_overrides, ..
            } = &self.backend
            {
                if let Some(route) = keyword_overrides
                    .read()
                    .expect("Failed to acquire read lock on keyword_overrides")
                    .get(keyword)
                    .cloned()
                {
                    if self.route_epoch.load(Ordering::Acquire) == epoch {
                        self.route_cache
                            .write()
                            .expect("Failed to acquire write lock on route_cache")
                            .insert(
                                keyword.to_string(),
                                CachedRoute {
                                    route: route.clone(),
                                    epoch,
                                },
                            );
                        if self.route_epoch.load(Ordering::Acquire) == epoch {
                            return Some(route);
                        }
                    }
                    continue;
                }
            }

            let route = match &self.backend {
                RouterBackend::Epring { ring, .. } => {
                    let ring = ring.read().expect("Failed to acquire read lock on epring");
                    let route = ring.route_keyword(keyword)?;
                    let prefix = ring.entry_prefix(route.entry_index)?.to_string();
                    let node_name = ring.node_name(route.node_index)?.to_string();
                    let addr = self.storager_addrs.get(&node_name)?.clone();

                    RouteTarget {
                        prefix,
                        node_name,
                        addr,
                    }
                }
                RouterBackend::Chring { ring } => {
                    let ring = ring
                        .read()
                        .expect("Failed to acquire read lock on consistent hash ring");
                    let node_name = ring.get_node(keyword)?.to_string();
                    let addr = self.storager_addrs.get(&node_name)?.clone();

                    RouteTarget {
                        prefix: String::new(),
                        node_name,
                        addr,
                    }
                }
            };

            if self.route_epoch.load(Ordering::Acquire) == epoch {
                self.route_cache
                    .write()
                    .expect("Failed to acquire write lock on route_cache")
                    .insert(
                        keyword.to_string(),
                        CachedRoute {
                            route: route.clone(),
                            epoch,
                        },
                    );
                if self.route_epoch.load(Ordering::Acquire) == epoch {
                    return Some(route);
                }
            }
        }
    }

    pub fn route_key_hex(&self, key_hex: &str) -> Option<RouteTarget> {
        match &self.backend {
            RouterBackend::Epring { ring, .. } => {
                let ring = ring.read().expect("Failed to acquire read lock on epring");
                let route = ring.route_key_hex(key_hex)?;
                let prefix = ring.entry_prefix(route.entry_index)?.to_string();
                let node_name = ring.node_name(route.node_index)?.to_string();
                let addr = self.storager_addrs.get(&node_name)?.clone();

                Some(RouteTarget {
                    prefix,
                    node_name,
                    addr,
                })
            }
            RouterBackend::Chring { .. } => None,
        }
    }

    pub fn get_storager_for_keyword(&self, keyword: &str) -> Option<(String, String)> {
        let route = self.route_keyword(keyword)?;
        Some((route.node_name, route.addr))
    }

    pub fn record_insert(
        &self,
        keyword: &str,
        prefix: &str,
        node_name: &str,
        root_summary: Vec<u8>,
    ) -> Option<PrefixSplitPlan> {
        match &self.backend {
            RouterBackend::Epring {
                ring,
                keyword_overrides,
            } => {
                let _cache_guard = self.begin_route_cache_mutation();
                keyword_overrides
                    .write()
                    .expect("Failed to acquire write lock on keyword_overrides")
                    .insert(
                        keyword.to_string(),
                        RouteTarget {
                            prefix: prefix.to_string(),
                            node_name: node_name.to_string(),
                            addr: self
                                .storager_addrs
                                .get(node_name)
                                .cloned()
                                .unwrap_or_default(),
                        },
                    );

                let mut ring = ring
                    .write()
                    .expect("Failed to acquire write lock on epring");
                let split = ring.record_insert(prefix, root_summary);
                let split_plan = split
                    .as_ref()
                    .and_then(|event| self.build_split_plan(event, &ring));
                drop(ring);

                self.clear_route_cache();
                split_plan
            }
            RouterBackend::Chring { .. } => None,
        }
    }

    pub fn record_delete(&self, keyword: &str, prefix: &str, root_summary: Vec<u8>) {
        match &self.backend {
            RouterBackend::Epring {
                ring,
                keyword_overrides,
            } => {
                let _cache_guard = self.begin_route_cache_mutation();
                keyword_overrides
                    .write()
                    .expect("Failed to acquire write lock on keyword_overrides")
                    .remove(keyword);

                let mut ring = ring
                    .write()
                    .expect("Failed to acquire write lock on epring");
                ring.record_delete(prefix, root_summary);
                self.clear_route_cache();
            }
            RouterBackend::Chring { .. } => {}
        }
    }

    pub fn update_prefix_summary(&self, prefix: &str, root_summary: Vec<u8>) {
        if let RouterBackend::Epring { ring, .. } = &self.backend {
            let mut ring = ring
                .write()
                .expect("Failed to acquire write lock on epring");
            ring.update_root_summary(prefix, root_summary);
        }
    }

    pub fn presplit_empty_prefixes(
        &self,
        prefix_counts: &HashMap<String, usize>,
    ) -> Vec<PrefixSplitPlan> {
        let RouterBackend::Epring { ring, .. } = &self.backend else {
            return Vec::new();
        };

        let _cache_guard = self.begin_route_cache_mutation();
        let mut ring = ring
            .write()
            .expect("Failed to acquire write lock on epring");

        let mut prefixes = prefix_counts.keys().cloned().collect::<Vec<_>>();
        prefixes.sort_by_key(|prefix| prefix.len());

        let mut plans = Vec::new();
        for prefix in prefixes {
            let Some(count) = prefix_counts.get(&prefix).copied() else {
                continue;
            };
            if let Some(event) = ring.maybe_presplit_empty_prefix(&prefix, count as u64) {
                if let Some(plan) = self.build_split_plan(&event, &ring) {
                    plans.push(plan);
                }
            }
        }
        drop(ring);
        self.clear_route_cache();

        plans
    }

    pub fn add_storager(&mut self, addr: String, split_threshold: usize) {
        let _cache_guard = self.begin_route_cache_mutation();
        let idx = self.storager_addrs.len();
        let node_name = format!("storager-{}", idx);
        self.node_addrs.push(addr.clone());
        self.node_names.push(node_name.clone());
        self.storager_addrs.insert(node_name, addr);
        self.rebuild_backend(split_threshold);
    }

    pub fn remove_storager(&mut self, node_name: &str, split_threshold: usize) {
        let _cache_guard = self.begin_route_cache_mutation();
        if let Some(index) = self.node_names.iter().position(|name| name == node_name) {
            self.node_addrs.remove(index);
        }
        self.storager_addrs.remove(node_name);
        self.node_names.retain(|name| name != node_name);
        self.rebuild_backend(split_threshold);
    }

    pub fn get_all_storagers(&self) -> Vec<(String, String)> {
        self.storager_addrs
            .iter()
            .map(|(name, addr)| (name.clone(), addr.clone()))
            .collect()
    }

    pub fn storager_count(&self) -> usize {
        self.storager_addrs.len()
    }

    pub fn clear_keyword_overrides_for_prefix(&self, prefix: &str) {
        if let RouterBackend::Epring {
            keyword_overrides, ..
        } = &self.backend
        {
            let _cache_guard = self.begin_route_cache_mutation();
            keyword_overrides
                .write()
                .expect("Failed to acquire write lock on keyword_overrides")
                .retain(|keyword, _| !EPRing::keyword_matches_prefix(keyword, prefix));
            self.clear_route_cache_for_prefix(prefix);
        }
    }

    pub fn reset(&self, split_threshold: usize) {
        let _cache_guard = self.begin_route_cache_mutation();
        match &self.backend {
            RouterBackend::Epring {
                ring,
                keyword_overrides,
            } => {
                *ring
                    .write()
                    .expect("Failed to acquire write lock on epring") =
                    EPRing::new(&self.node_names, split_threshold as u64);
                keyword_overrides
                    .write()
                    .expect("Failed to acquire write lock on keyword_overrides")
                    .clear();
            }
            RouterBackend::Chring { ring } => {
                *ring
                    .write()
                    .expect("Failed to acquire write lock on consistent hash ring") =
                    Self::build_consistent_hash_ring(&self.node_names);
            }
        }
        self.clear_route_cache();
    }

    pub fn epring_structure_lines(&self) -> Vec<String> {
        self.routing_structure_lines()
    }

    pub fn routing_structure_lines(&self) -> Vec<String> {
        match &self.backend {
            RouterBackend::Epring { ring, .. } => ring
                .read()
                .expect("Failed to acquire read lock on epring")
                .structure_lines(),
            RouterBackend::Chring { ring } => {
                let ring = ring
                    .read()
                    .expect("Failed to acquire read lock on consistent hash ring");
                let mut lines = Vec::new();
                let mut nodes = ring.get_all_nodes();
                nodes.sort();
                lines.push(format!(
                    "mode=chring, nodes={}, virtual_nodes={}, per_node={}",
                    ring.node_count(),
                    ring.virtual_node_count(),
                    DEFAULT_CHRING_VIRTUAL_NODES_PER_NODE
                ));
                for node in nodes {
                    let vnode_count = ring.get_virtual_node_count(&node).unwrap_or_default();
                    lines.push(format!("[{}] virtual_nodes={}", node, vnode_count));
                }
                lines
            }
        }
    }

    fn build_split_plan(&self, event: &EPRingSplitEvent, ring: &EPRing) -> Option<PrefixSplitPlan> {
        let parent_prefix = ring.entry_prefix(event.parent_entry_index)?.to_string();
        let source_node_name = format!("storager-{}", event.original_owner_index);
        let source_addr = self
            .node_addrs
            .get(event.original_owner_index)
            .cloned()
            .or_else(|| self.storager_addrs.get(&source_node_name).cloned())?;
        let children = event
            .child_routes
            .iter()
            .map(|route| {
                let prefix = ring.entry_prefix(route.entry_index)?.to_string();
                let node_name = format!("storager-{}", route.node_index);
                let addr = self.storager_addrs.get(&node_name)?.clone();
                Some(RouteTarget {
                    prefix,
                    node_name,
                    addr,
                })
            })
            .collect::<Option<Vec<_>>>()?;

        Some(PrefixSplitPlan {
            parent_prefix: parent_prefix.clone(),
            source: RouteTarget {
                prefix: parent_prefix,
                node_name: source_node_name,
                addr: source_addr,
            },
            children,
        })
    }

    fn rebuild_backend(&mut self, split_threshold: usize) {
        self.backend = match self.route_mode {
            RouteMode::Epring => RouterBackend::Epring {
                ring: Arc::new(RwLock::new(EPRing::new(
                    &self.node_names,
                    split_threshold as u64,
                ))),
                keyword_overrides: Arc::new(RwLock::new(HashMap::new())),
            },
            RouteMode::Chring => RouterBackend::Chring {
                ring: Arc::new(RwLock::new(Self::build_consistent_hash_ring(
                    &self.node_names,
                ))),
            },
        };
        self.clear_route_cache();
    }

    fn build_consistent_hash_ring(node_names: &[String]) -> ConsistentHashRing {
        let mut ring = ConsistentHashRing::new();
        for node_name in node_names {
            ring.add_node(node_name, DEFAULT_CHRING_VIRTUAL_NODES_PER_NODE)
                .unwrap_or_else(|err| panic!("failed to add node to consistent hash ring: {err}"));
        }
        ring
    }

    fn begin_route_cache_mutation(&self) -> RouteCacheMutationGuard {
        self.route_epoch.fetch_add(1, Ordering::AcqRel);
        RouteCacheMutationGuard {
            route_epoch: Arc::clone(&self.route_epoch),
        }
    }

    fn clear_route_cache(&self) {
        self.route_cache
            .write()
            .expect("Failed to acquire write lock on route_cache")
            .clear();
    }

    fn clear_route_cache_for_prefix(&self, prefix: &str) {
        self.route_cache
            .write()
            .expect("Failed to acquire write lock on route_cache")
            .retain(|keyword, _| !EPRing::keyword_matches_prefix(keyword, prefix));
    }
}

struct RouteCacheMutationGuard {
    route_epoch: Arc<AtomicU64>,
}

impl Drop for RouteCacheMutationGuard {
    fn drop(&mut self) {
        self.route_epoch.fetch_add(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_creation() {
        let addrs = vec![
            "http://[::1]:50052".to_string(),
            "http://[::1]:50053".to_string(),
        ];
        let router = Router::new(addrs, 150);
        assert_eq!(router.storager_count(), 2);
    }

    #[test]
    fn test_keyword_routing() {
        let addrs = vec![
            "http://[::1]:50052".to_string(),
            "http://[::1]:50053".to_string(),
        ];
        let router = Router::new(addrs, 150);

        let result1 = router.get_storager_for_keyword("test");
        let result2 = router.get_storager_for_keyword("test");

        assert_eq!(result1, result2);
    }

    #[test]
    fn test_insert_records_override() {
        let addrs = vec![
            "http://[::1]:50052".to_string(),
            "http://[::1]:50053".to_string(),
        ];
        let router = Router::new(addrs, 1);

        let initial = router.route_keyword("alpha").expect("initial route");
        router.record_insert("alpha", &initial.prefix, &initial.node_name, vec![1, 2, 3]);

        let routed = router.route_keyword("alpha").expect("stored route");
        assert_eq!(routed.prefix, initial.prefix);
        assert_eq!(routed.node_name, initial.node_name);
    }

    #[test]
    fn test_chring_routing_backend() {
        let addrs = vec![
            "http://[::1]:50052".to_string(),
            "http://[::1]:50053".to_string(),
        ];
        let router = Router::new_with_mode(addrs, 150, RouteMode::Chring);

        assert_eq!(router.route_mode(), RouteMode::Chring);

        let route1 = router.route_keyword("test").expect("route 1");
        let route2 = router.route_keyword("test").expect("route 2");
        assert_eq!(route1, route2);
        assert!(route1.prefix.is_empty());
        assert!(route1.node_name.starts_with("storager-"));
        assert_eq!(
            router.record_insert("test", "", &route1.node_name, vec![1]),
            None
        );
    }
}
