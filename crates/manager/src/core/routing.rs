use super::{EPRing, EPRingSplitEvent};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteTarget {
    pub prefix: String,
    pub node_name: String,
    pub addr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixSplitPlan {
    pub parent_prefix: String,
    pub source: RouteTarget,
    pub children: Vec<RouteTarget>,
}

/// Router backed by an expandable prefix ring.
pub struct Router {
    ring: Arc<RwLock<EPRing>>,
    storager_addrs: HashMap<String, String>,
    node_names: Vec<String>,
    keyword_overrides: Arc<RwLock<HashMap<String, RouteTarget>>>,
}

impl Router {
    /// `split_threshold` controls when a prefix expands into a child ring.
    pub fn new(storager_addrs: Vec<String>, split_threshold: usize) -> Self {
        let mut node_names = Vec::with_capacity(storager_addrs.len());
        let mut addr_map = HashMap::with_capacity(storager_addrs.len());

        for (idx, addr) in storager_addrs.iter().enumerate() {
            let node_name = format!("storager-{}", idx);
            node_names.push(node_name.clone());
            addr_map.insert(node_name, addr.clone());
        }

        Self {
            ring: Arc::new(RwLock::new(EPRing::new(
                &node_names,
                split_threshold as u64,
            ))),
            storager_addrs: addr_map,
            node_names,
            keyword_overrides: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn route_keyword(&self, keyword: &str) -> Option<RouteTarget> {
        if let Some(route) = self
            .keyword_overrides
            .read()
            .expect("Failed to acquire read lock on keyword_overrides")
            .get(keyword)
            .cloned()
        {
            return Some(route);
        }

        let ring = self
            .ring
            .read()
            .expect("Failed to acquire read lock on epring");
        let route = ring.route_keyword(keyword)?;
        let node_name = ring.node_name(route.node_index)?.to_string();
        let addr = self.storager_addrs.get(&node_name)?.clone();

        Some(RouteTarget {
            prefix: route.prefix,
            node_name,
            addr,
        })
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
        self.keyword_overrides
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

        let mut ring = self
            .ring
            .write()
            .expect("Failed to acquire write lock on epring");
        let split = ring.record_insert(prefix, root_summary);
        drop(ring);

        split.and_then(|event| self.build_split_plan(event))
    }

    pub fn record_delete(&self, keyword: &str, prefix: &str, root_summary: Vec<u8>) {
        self.keyword_overrides
            .write()
            .expect("Failed to acquire write lock on keyword_overrides")
            .remove(keyword);

        let mut ring = self
            .ring
            .write()
            .expect("Failed to acquire write lock on epring");
        ring.record_delete(prefix, root_summary);
    }

    pub fn update_prefix_summary(&self, prefix: &str, root_summary: Vec<u8>) {
        let mut ring = self
            .ring
            .write()
            .expect("Failed to acquire write lock on epring");
        ring.update_root_summary(prefix, root_summary);
    }

    pub fn presplit_empty_prefixes(
        &self,
        prefix_counts: &HashMap<String, usize>,
    ) -> Vec<PrefixSplitPlan> {
        let mut ring = self
            .ring
            .write()
            .expect("Failed to acquire write lock on epring");

        let mut prefixes = prefix_counts.keys().cloned().collect::<Vec<_>>();
        prefixes.sort_by_key(|prefix| prefix.len());

        let mut events = Vec::new();
        for prefix in prefixes {
            let Some(count) = prefix_counts.get(&prefix).copied() else {
                continue;
            };
            if let Some(event) = ring.maybe_presplit_empty_prefix(&prefix, count as u64) {
                events.push(event);
            }
        }
        drop(ring);

        events
            .into_iter()
            .filter_map(|event| self.build_split_plan(event))
            .collect()
    }

    pub fn add_storager(&mut self, addr: String, split_threshold: usize) {
        let idx = self.storager_addrs.len();
        let node_name = format!("storager-{}", idx);
        self.storager_addrs.insert(node_name, addr);

        let node_names: Vec<String> = self.storager_addrs.keys().cloned().collect::<Vec<_>>();
        *self
            .ring
            .write()
            .expect("Failed to acquire write lock on epring") =
            EPRing::new(&node_names, split_threshold as u64);
    }

    pub fn remove_storager(&mut self, node_name: &str, split_threshold: usize) {
        self.storager_addrs.remove(node_name);
        let node_names: Vec<String> = self.storager_addrs.keys().cloned().collect::<Vec<_>>();
        *self
            .ring
            .write()
            .expect("Failed to acquire write lock on epring") =
            EPRing::new(&node_names, split_threshold as u64);
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
        self.keyword_overrides
            .write()
            .expect("Failed to acquire write lock on keyword_overrides")
            .retain(|keyword, _| !EPRing::keyword_matches_prefix(keyword, prefix));
    }

    pub fn reset(&self, split_threshold: usize) {
        *self
            .ring
            .write()
            .expect("Failed to acquire write lock on epring") =
            EPRing::new(&self.node_names, split_threshold as u64);
        self.keyword_overrides
            .write()
            .expect("Failed to acquire write lock on keyword_overrides")
            .clear();
    }

    pub fn epring_structure_lines(&self) -> Vec<String> {
        self.ring
            .read()
            .expect("Failed to acquire read lock on epring")
            .structure_lines()
    }

    fn build_split_plan(&self, event: EPRingSplitEvent) -> Option<PrefixSplitPlan> {
        let source_node_name = format!("storager-{}", event.original_owner_index);
        let source_addr = self.storager_addrs.get(&source_node_name)?.clone();
        let children = event
            .child_routes
            .into_iter()
            .map(|route| {
                let node_name = format!("storager-{}", route.node_index);
                let addr = self.storager_addrs.get(&node_name)?.clone();
                Some(RouteTarget {
                    prefix: route.prefix,
                    node_name,
                    addr,
                })
            })
            .collect::<Option<Vec<_>>>()?;

        Some(PrefixSplitPlan {
            parent_prefix: event.parent_prefix.clone(),
            source: RouteTarget {
                prefix: event.parent_prefix,
                node_name: source_node_name,
                addr: source_addr,
            },
            children,
        })
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
}
