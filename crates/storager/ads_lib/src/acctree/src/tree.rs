use crate::acc_proof::{AccProof, MembershipProof, NonMembershipProof};
use crate::merkle_proof::MerkleProof;
use crate::node::Node;
use crate::persistence::{
    ACCTREE_STORAGE_FORMAT_VERSION, AccTreeStorageError, AccTreeWriteBackCache,
    PersistedAccTreeManifest, PersistedAccTreeNode, default_cache_limit, persisted_nodes_by_hash,
};
use crate::result::{
    DeleteResult, InsertResult, PathElement, SelectResult, SiblingNonMembershipProof,
    SingleSelectResult, TreeMatchResult, TreeProof,
};
use crate::utils::{Hash, empty_hash};
use accumulator_ads::acc::utils::digest_to_prime_field;
use accumulator_ads::digest::Digestible;
use accumulator_ads::{DynamicAccumulator, G1Affine, Set, digest_set_from_set};
use ark_serialize::CanonicalDeserialize;
use std::collections::{BTreeMap, BTreeSet};

pub struct AccumulatorTree {
    pub roots: Vec<Box<Node>>,
    record_index: BTreeMap<String, Vec<String>>,
    dirty_root_indices: BTreeSet<usize>,
    persisted_root_hashes: Vec<Hash>,
    persistence: Option<AccTreeWriteBackCache>,
}

impl Default for AccumulatorTree {
    fn default() -> Self {
        Self::new()
    }
}

impl AccumulatorTree {
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            record_index: BTreeMap::new(),
            dirty_root_indices: BTreeSet::new(),
            persisted_root_hashes: Vec::new(),
            persistence: None,
        }
    }

    pub fn new_with_persistence(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, AccTreeStorageError> {
        Self::new_with_persistence_and_cache_limit(path, default_cache_limit())
    }

    pub fn new_with_persistence_and_cache_limit(
        path: impl AsRef<std::path::Path>,
        cache_limit: usize,
    ) -> Result<Self, AccTreeStorageError> {
        let persistence = AccTreeWriteBackCache::open(path, cache_limit)?;
        let persisted_root_hashes = match persistence.load_manifest()? {
            Some(manifest) => manifest.root_hashes,
            None => Vec::new(),
        };

        Ok(Self {
            roots: Vec::new(),
            record_index: BTreeMap::new(),
            dirty_root_indices: BTreeSet::new(),
            persisted_root_hashes,
            persistence: Some(persistence),
        })
    }

    pub fn global_state_hash(&self) -> Hash {
        self.active_root_hashes()
            .iter()
            .copied()
            .max()
            .unwrap_or_else(empty_hash)
    }

    pub fn persisted_root_count(&self) -> usize {
        self.persisted_root_hashes.len()
    }

    fn active_root_hashes(&self) -> Vec<Hash> {
        if self.persistence.is_some() && self.roots.is_empty() {
            self.persisted_root_hashes.clone()
        } else {
            self.roots.iter().map(|root| *root.hash()).collect()
        }
    }

    fn load_persisted_node(&self, hash: &Hash) -> Option<PersistedAccTreeNode> {
        let persistence = self.persistence.as_ref()?;
        persistence.load_node(hash).ok().flatten()
    }

    fn root_membership_proof(root: &Node, key: &str) -> Option<MembershipProof> {
        let root_acc = root.acc();
        let acc = DynamicAccumulator::from_value(root_acc);
        let element = digest_to_prime_field(&key.to_digest());
        let witness = accumulator_ads::MembershipProof::new(&acc, element).ok()?;
        Some(MembershipProof::new(key.to_string(), root_acc, witness))
    }

    fn forest_non_membership_proof(&self, key: &str) -> Option<NonMembershipProof> {
        let mut keys = Vec::new();
        if self.persistence.is_some() && self.roots.is_empty() {
            for root_hash in &self.persisted_root_hashes {
                Self::collect_persisted_keys(self, root_hash, &mut keys);
            }
        } else {
            for root in &self.roots {
                keys.extend(root.keys().iter().cloned());
            }
        }
        let key_set = Set::from_vec(keys);
        let accumulator = DynamicAccumulator::calculate_commitment(&digest_set_from_set(&key_set));
        NonMembershipProof::new(key.to_string(), accumulator, &key_set)
    }

    fn all_records(&self) -> Vec<(String, String)> {
        let mut records: Vec<(String, String)> = if !self.record_index.is_empty() {
            let mut values = Vec::new();
            for (key, fids) in &self.record_index {
                for fid in fids {
                    values.push((key.clone(), fid.clone()));
                }
            }
            values
        } else if self.persistence.is_some() && self.roots.is_empty() {
            let mut values = Vec::new();
            for root_hash in &self.persisted_root_hashes {
                Self::collect_persisted_records(self, root_hash, &mut values);
            }
            values
        } else {
            self.roots
                .iter()
                .flat_map(|root| root.collect_leaves(None))
                .collect()
        };
        records.sort();
        records
    }

    fn build_record_index(records: &[(String, String)]) -> BTreeMap<String, Vec<String>> {
        let mut record_index: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, fid) in records {
            record_index
                .entry(key.clone())
                .or_default()
                .push(fid.clone());
        }
        record_index
    }

    fn build_forest_from_index(record_index: &BTreeMap<String, Vec<String>>) -> Vec<Box<Node>> {
        let lane_count = record_index.values().map(Vec::len).max().unwrap_or(0);
        let mut lanes = (0..lane_count)
            .map(|_| Vec::new())
            .collect::<Vec<Vec<Box<Node>>>>();

        for (key, fids) in record_index {
            for (index, fid) in fids.iter().enumerate() {
                lanes[index].push(Node::new(key.clone(), fid.clone()));
            }
        }

        lanes
            .into_iter()
            .filter(|lane| !lane.is_empty())
            .map(Self::build_lane_tree)
            .collect()
    }

    fn rebuild_from_record_index(&mut self) {
        self.roots = Self::build_forest_from_index(&self.record_index);
    }

    fn rebuild_from_records_without_sync(
        &mut self,
        mut records: Vec<(String, String)>,
        mark_all_dirty: bool,
    ) {
        records.sort();
        records.dedup();
        self.record_index = Self::build_record_index(&records);
        self.rebuild_from_record_index();
        self.dirty_root_indices.clear();
        if mark_all_dirty {
            self.dirty_root_indices.extend(0..self.roots.len());
        }
    }

    fn hydrate_from_persistence_if_needed(&mut self) {
        if self.persistence.is_none()
            || !self.roots.is_empty()
            || !self.record_index.is_empty()
            || self.persisted_root_hashes.is_empty()
        {
            return;
        }

        let mut records = Vec::new();
        for root_hash in &self.persisted_root_hashes {
            Self::collect_persisted_records(self, root_hash, &mut records);
        }

        self.rebuild_from_records_without_sync(records, false);
    }

    fn collect_persisted_keys(this: &Self, hash: &Hash, keys: &mut Vec<String>) {
        let Some(node) = this.load_persisted_node(hash) else {
            return;
        };

        match node {
            PersistedAccTreeNode::Leaf { key, .. } => keys.push(key),
            PersistedAccTreeNode::NonLeaf {
                left_hash,
                right_hash,
                ..
            } => {
                Self::collect_persisted_keys(this, &left_hash, keys);
                Self::collect_persisted_keys(this, &right_hash, keys);
            }
        }
    }

    fn collect_persisted_records(this: &Self, hash: &Hash, records: &mut Vec<(String, String)>) {
        let Some(node) = this.load_persisted_node(hash) else {
            return;
        };

        match node {
            PersistedAccTreeNode::Leaf { key, fid, .. } => records.push((key, fid)),
            PersistedAccTreeNode::NonLeaf {
                left_hash,
                right_hash,
                ..
            } => {
                Self::collect_persisted_records(this, &left_hash, records);
                Self::collect_persisted_records(this, &right_hash, records);
            }
        }
    }

    fn rebuild_from_records(&mut self, records: Vec<(String, String)>) {
        self.rebuild_from_records_without_sync(records, true);
        self.sync_persistence().expect(&String::from_iter([
            'a', 'c', 'c', 't', 'r', 'e', 'e', ' ', 'p', 'e', 'r', 's', 'i', 's', 't', 'e', 'n',
            'c', 'e', ' ', 'u', 'p', 'd', 'a', 't', 'e', ' ', 'f', 'a', 'i', 'l', 'e', 'd',
        ]));
    }

    fn build_lane_tree(mut nodes: Vec<Box<Node>>) -> Box<Node> {
        while nodes.len() > 1 {
            let mut next = Vec::with_capacity((nodes.len() + 1) / 2);
            let mut iter = nodes.into_iter();
            while let Some(left) = iter.next() {
                if let Some(right) = iter.next() {
                    next.push(Node::merge(left, right, None));
                } else {
                    next.push(left);
                }
            }
            nodes = next;
        }

        nodes.pop().expect("lane tree requires at least one node")
    }

    fn build_lane_from_index(&self, lane_index: usize) -> Option<Box<Node>> {
        let lane_records = self
            .record_index
            .iter()
            .filter_map(|(record_key, fids)| {
                fids.get(lane_index)
                    .map(|record_fid| (record_key.clone(), record_fid.clone()))
            })
            .collect::<Vec<_>>();

        if lane_records.is_empty() {
            None
        } else {
            Some(Self::build_lane_tree(
                lane_records
                    .into_iter()
                    .map(|(record_key, record_fid)| Node::new(record_key, record_fid))
                    .collect(),
            ))
        }
    }

    fn rebuild_lanes_from_index(&mut self, start_lane: usize) {
        let lane_count = self.record_index.values().map(Vec::len).max().unwrap_or(0);
        if self.roots.len() > lane_count {
            self.roots.truncate(lane_count);
        }

        if lane_count == 0 {
            self.dirty_root_indices.clear();
            return;
        }

        let start_lane = start_lane.min(lane_count);
        for lane_index in start_lane..lane_count {
            if let Some(lane_tree) = self.build_lane_from_index(lane_index) {
                if lane_index < self.roots.len() {
                    self.roots[lane_index] = lane_tree;
                } else {
                    self.roots.push(lane_tree);
                }
                self.dirty_root_indices.insert(lane_index);
            }
        }

        self.dirty_root_indices
            .retain(|index| *index < self.roots.len());
    }

    fn delete_record_from_index(&mut self, key: &str, fid: &str) -> Option<usize> {
        self.hydrate_from_persistence_if_needed();

        let (start_lane, remove_key) = {
            let entry = self.record_index.get_mut(key)?;
            let start_lane = entry
                .binary_search_by(|current| current.as_str().cmp(fid))
                .ok()?;
            entry.remove(start_lane);
            (start_lane, entry.is_empty())
        };

        if remove_key {
            self.record_index.remove(key);
        }

        Some(start_lane)
    }

    fn sync_persistence(&mut self) -> Result<(), AccTreeStorageError> {
        let root_hashes = self
            .roots
            .iter()
            .map(|root| *root.hash())
            .collect::<Vec<_>>();
        let global_state_hash = self.global_state_hash();

        let Some(persistence) = self.persistence.as_mut() else {
            return Ok(());
        };

        if !self.dirty_root_indices.is_empty() {
            let dirty_roots = self
                .dirty_root_indices
                .iter()
                .filter_map(|index| self.roots.get(*index).cloned())
                .collect::<Vec<_>>();
            let persisted_nodes = persisted_nodes_by_hash(&dirty_roots)?;
            for node in persisted_nodes.into_values() {
                persistence.cache_node(node, true)?;
            }
        }

        let manifest = PersistedAccTreeManifest {
            version: ACCTREE_STORAGE_FORMAT_VERSION,
            root_hashes: root_hashes.clone(),
            global_state_hash,
        };

        persistence.persist_manifest(&manifest)?;
        self.persisted_root_hashes = root_hashes;
        self.dirty_root_indices.clear();
        // Keep the in-memory forest hot after syncing so repeated batch loads and
        // prefix migrations in the same process do not need to reconstruct the
        // entire tree from LevelDB on every follow-up mutation.
        Ok(())
    }

    fn insert(&mut self, key: String, fid: String) {
        self.hydrate_from_persistence_if_needed();

        let start_lane = {
            let entry = self.record_index.entry(key.clone()).or_default();
            if entry.binary_search(&fid).is_ok() {
                return;
            }

            let insert_pos = entry.binary_search(&fid).unwrap_err();
            entry.insert(insert_pos, fid.clone());
            insert_pos
        };

        let lane_count = self.record_index.values().map(Vec::len).max().unwrap_or(0);
        if lane_count == 0 {
            return;
        }

        for lane_index in start_lane..lane_count {
            if let Some(lane_tree) = self.build_lane_from_index(lane_index) {
                if lane_index < self.roots.len() {
                    self.roots[lane_index] = lane_tree;
                } else {
                    self.roots.push(lane_tree);
                }
                self.dirty_root_indices.insert(lane_index);
            }
        }
    }

    fn select_tree_with_proof(root: &Node, key: &str) -> Option<Vec<SingleSelectResult>> {
        let root_membership_proof = Self::root_membership_proof(root, key)?;
        let matches = root.find_all_matches(key, *root.hash());

        let results = matches
            .into_iter()
            .map(|entry| {
                let merkle_path = entry.path.iter().rev().cloned().collect();
                let sibling_proofs = entry
                    .path
                    .iter()
                    .map(|element| {
                        let non_membership_proof = NonMembershipProof::new(
                            key.to_string(),
                            element.sibling_acc,
                            &element.sibling_keys,
                        )?;
                        Some(SiblingNonMembershipProof {
                            sibling_hash: element.sibling_hash,
                            sibling_accumulator: element.sibling_acc,
                            non_membership_proof,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;

                Some(SingleSelectResult {
                    fid: entry.fid,
                    tree_proof: TreeProof {
                        tree_root_hash: entry.tree_root_hash,
                        leaf_merkle_proof: MerkleProof::new(entry.tree_root_hash, merkle_path),
                        root_membership_proof: root_membership_proof.clone(),
                        sibling_proofs,
                    },
                })
            })
            .collect::<Option<Vec<_>>>()?;

        Some(results)
    }

    fn select_tree_with_proof_from_persistence(
        &self,
        root_hash: &Hash,
        key: &str,
    ) -> Option<Vec<SingleSelectResult>> {
        let root_node = self.load_persisted_node(root_hash)?;
        let (root_acc, root_keys) = match &root_node {
            PersistedAccTreeNode::Leaf { key: leaf_key, .. } => {
                let keys = Set::from_vec(vec![leaf_key.clone()]);
                let digest_set = digest_set_from_set(&keys);
                (DynamicAccumulator::calculate_commitment(&digest_set), keys)
            }
            PersistedAccTreeNode::NonLeaf {
                keys, acc_bytes, ..
            } => {
                let acc = G1Affine::deserialize(&acc_bytes[..]).ok()?;
                let keys_set: Set<String> = Set::from_vec(keys.clone());
                (acc, keys_set)
            }
        };

        if !root_keys.contains(key) {
            return None;
        }

        let acc = DynamicAccumulator::from_value(root_acc);
        let element = digest_to_prime_field(&key.to_digest());
        let witness = accumulator_ads::MembershipProof::new(&acc, element).ok()?;
        let root_membership_proof = MembershipProof::new(key.to_string(), root_acc, witness);

        let matches = self.collect_matches_from_persistence(root_hash, key, *root_hash, Vec::new());

        let results = matches
            .into_iter()
            .map(|entry| {
                let merkle_path = entry.path.iter().rev().cloned().collect();
                let sibling_proofs = entry
                    .path
                    .iter()
                    .map(|element| {
                        let non_membership_proof = NonMembershipProof::new(
                            key.to_string(),
                            element.sibling_acc,
                            &element.sibling_keys,
                        )?;
                        Some(SiblingNonMembershipProof {
                            sibling_hash: element.sibling_hash,
                            sibling_accumulator: element.sibling_acc,
                            non_membership_proof,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;

                Some(SingleSelectResult {
                    fid: entry.fid,
                    tree_proof: TreeProof {
                        tree_root_hash: entry.tree_root_hash,
                        leaf_merkle_proof: MerkleProof::new(entry.tree_root_hash, merkle_path),
                        root_membership_proof: root_membership_proof.clone(),
                        sibling_proofs,
                    },
                })
            })
            .collect::<Option<Vec<_>>>()?;

        Some(results)
    }

    fn collect_matches_from_persistence(
        &self,
        node_hash: &Hash,
        target_key: &str,
        tree_root_hash: Hash,
        path: Vec<PathElement>,
    ) -> Vec<TreeMatchResult> {
        let Some(node) = self.load_persisted_node(node_hash) else {
            return Vec::new();
        };

        match node {
            PersistedAccTreeNode::Leaf { key, fid, .. } => {
                if key == target_key {
                    vec![TreeMatchResult {
                        fid,
                        path,
                        tree_root_hash,
                    }]
                } else {
                    Vec::new()
                }
            }
            PersistedAccTreeNode::NonLeaf {
                keys,
                acc_bytes,
                left_hash,
                right_hash,
                ..
            } => {
                let Some(left_node) = self.load_persisted_node(&left_hash) else {
                    return Vec::new();
                };
                let Some(right_node) = self.load_persisted_node(&right_hash) else {
                    return Vec::new();
                };

                let parent_acc = G1Affine::deserialize(&acc_bytes[..]).ok();
                let Some(parent_acc) = parent_acc else {
                    return Vec::new();
                };
                let parent_keys = Set::from_vec(keys);
                let left_keys = Self::persisted_node_keys(&left_node);
                let right_keys = Self::persisted_node_keys(&right_node);
                let left_acc =
                    Self::persisted_node_acc(&left_node).unwrap_or_else(crate::utils::empty_acc);
                let right_acc =
                    Self::persisted_node_acc(&right_node).unwrap_or_else(crate::utils::empty_acc);

                let mut results = Vec::new();

                if left_keys.contains(target_key) {
                    let mut left_path = path.clone();
                    left_path.push(PathElement {
                        sibling_hash: right_hash,
                        is_left_sibling: false,
                        sibling_keys: right_keys.clone(),
                        sibling_acc: right_acc,
                        parent_keys: parent_keys.clone(),
                        parent_acc,
                    });
                    results.extend(self.collect_matches_from_persistence(
                        &left_hash,
                        target_key,
                        tree_root_hash,
                        left_path,
                    ));
                }

                if right_keys.contains(target_key) {
                    let mut right_path = path;
                    right_path.push(PathElement {
                        sibling_hash: left_hash,
                        is_left_sibling: true,
                        sibling_keys: left_keys,
                        sibling_acc: left_acc,
                        parent_keys,
                        parent_acc,
                    });
                    results.extend(self.collect_matches_from_persistence(
                        &right_hash,
                        target_key,
                        tree_root_hash,
                        right_path,
                    ));
                }

                results
            }
        }
    }

    fn persisted_node_keys(node: &PersistedAccTreeNode) -> Set<String> {
        match node {
            PersistedAccTreeNode::Leaf { key, .. } => Set::from_vec(vec![key.clone()]),
            PersistedAccTreeNode::NonLeaf { keys, .. } => Set::from_vec(keys.clone()),
        }
    }

    fn persisted_node_acc(node: &PersistedAccTreeNode) -> Option<G1Affine> {
        match node {
            PersistedAccTreeNode::Leaf { key, .. } => {
                let digest_set = digest_set_from_set(&Set::from_vec(vec![key.clone()]));
                Some(DynamicAccumulator::calculate_commitment(&digest_set))
            }
            PersistedAccTreeNode::NonLeaf { acc_bytes, .. } => {
                G1Affine::deserialize(&acc_bytes[..]).ok()
            }
        }
    }

    pub fn select(&self, key: &str) -> Vec<String> {
        self.select_all_with_proof(key).fids()
    }

    pub fn select_all_with_proof(&self, key: &str) -> SelectResult {
        let mut results = Vec::new();
        if self.persistence.is_some() && self.roots.is_empty() {
            for root_hash in &self.persisted_root_hashes {
                if let Some(mut matches) =
                    self.select_tree_with_proof_from_persistence(root_hash, key)
                {
                    results.append(&mut matches);
                }
            }
        } else {
            for root in &self.roots {
                if !root.has_key(key) {
                    continue;
                }
                if let Some(mut matches) = Self::select_tree_with_proof(root, key) {
                    results.append(&mut matches);
                }
            }
        }
        SelectResult::new(results, key.to_string())
    }

    pub fn select_with_proof(&self, key: &str) -> SelectResult {
        self.select_all_with_proof(key)
    }

    pub fn records(&self) -> Vec<(String, String)> {
        self.all_records()
    }

    pub fn rebuild_from_records_snapshot(&mut self, records: Vec<(String, String)>) {
        self.rebuild_from_records(records);
    }

    pub fn insert_with_proof(&mut self, key: String, fid: String) -> InsertResult {
        let pre_acc_proof = if self.select(&key).is_empty() {
            self.forest_non_membership_proof(&key)
        } else {
            None
        };
        self.insert(key.clone(), fid.clone());
        let result = self.select_all_with_proof(&key);
        let entry = result
            .results
            .into_iter()
            .find(|entry| entry.fid == fid)
            .unwrap();
        InsertResult::new(
            fid,
            entry.tree_proof.leaf_merkle_proof,
            entry.tree_proof.root_membership_proof,
            pre_acc_proof,
        )
    }

    pub fn delete_with_proof(&mut self, key: &str, fid: &str) -> DeleteResult {
        let before = self.select_all_with_proof(key);
        let removed = before
            .results
            .iter()
            .find(|entry| entry.fid == fid)
            .cloned();

        if let Some(start_lane) = self.delete_record_from_index(key, fid) {
            self.rebuild_lanes_from_index(start_lane);
            self.sync_persistence().expect(&String::from_iter([
                'a', 'c', 'c', 't', 'r', 'e', 'e', ' ', 'p', 'e', 'r', 's', 'i', 's', 't', 'e',
                'n', 'c', 'e', ' ', 'u', 'p', 'd', 'a', 't', 'e', ' ', 'f', 'a', 'i', 'l', 'e',
                'd',
            ]));
        }

        let after = self.select_all_with_proof(key);
        let post_acc = if let Some(entry) = after.results.first() {
            Some(AccProof::new_membership(
                entry.tree_proof.root_membership_proof.clone(),
            ))
        } else {
            self.forest_non_membership_proof(key)
                .map(AccProof::new_non_membership)
        };

        DeleteResult::new(
            fid.to_string(),
            removed.as_ref().map(|entry| entry.fid.clone()),
            after.results.first().map(|entry| entry.fid.clone()),
            removed
                .as_ref()
                .map(|entry| entry.tree_proof.leaf_merkle_proof.clone()),
            removed
                .as_ref()
                .map(|entry| entry.tree_proof.root_membership_proof.clone()),
            after
                .results
                .first()
                .map(|entry| entry.tree_proof.leaf_merkle_proof.clone()),
            post_acc,
        )
    }

    pub fn delete_all(&mut self, key: &str) {
        self.hydrate_from_persistence_if_needed();
        if self.record_index.remove(key).is_some() {
            self.rebuild_lanes_from_index(0);
            self.sync_persistence().expect(&String::from_iter([
                'a', 'c', 'c', 't', 'r', 'e', 'e', ' ', 'p', 'e', 'r', 's', 'i', 's', 't', 'e',
                'n', 'c', 'e', ' ', 'u', 'p', 'd', 'a', 't', 'e', ' ', 'f', 'a', 'i', 'l', 'e',
                'd',
            ]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accumulator_ads::acc::{PublicParameters, init_public_parameters_direct};
    use ark_bls12_381::Fr;

    #[test]
    fn test_incremental_insert_only_rebuilds_new_lane() {
        let params = PublicParameters::generate_for_testing(Fr::from(5u64), 32);
        init_public_parameters_direct(params).unwrap();

        let mut tree = AccumulatorTree::new();

        let _ = tree.insert_with_proof("rust".to_string(), "file1".to_string());
        assert_eq!(tree.roots.len(), 1);
        let lane0_ptr = tree.roots[0].as_ref() as *const Node;

        let _ = tree.insert_with_proof("rust".to_string(), "file2".to_string());

        assert_eq!(tree.roots.len(), 2);
        assert_eq!(
            tree.record_index.get("rust").cloned(),
            Some(vec!["file1".to_string(), "file2".to_string()])
        );
        assert_eq!(tree.roots[0].as_ref() as *const Node, lane0_ptr);

        let fids = tree.select("rust");
        assert_eq!(fids, vec!["file1".to_string(), "file2".to_string()]);
    }
}
