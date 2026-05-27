use std::array;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub use acc;

use acc::acc_mod::{Acc, Fr, G1Affine};
use acc::digest::Digestible;
use acc::set::MultiSet;
use acc::utils::digest_to_prime_field;
use acc::Accumulator;

use crate::unified_ads::{AuthenticatedDataStructure, UnifiedKey, UnifiedValue};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub struct AccTrieProof(pub Vec<u8>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedRecord {
    pub key: Vec<u8>,
    pub values: Vec<String>,
}

pub struct AccTrieAdapter {
    trie: Arc<RwLock<AccTrie>>,
}

impl AccTrieAdapter {
    pub fn new() -> Self {
        Self {
            trie: Arc::new(RwLock::new(AccTrie::new())),
        }
    }
}

impl AuthenticatedDataStructure for AccTrieAdapter {
    type Key = UnifiedKey;
    type Value = UnifiedValue;
    type Proof = AccTrieProof;
    type Database = ();

    fn insert(
        &mut self,
        key: Self::Key,
        value: Self::Value,
        _db: Option<&mut Self::Database>,
    ) -> Result<Self::Proof> {
        let keyv = key.0;
        let val = match value {
            UnifiedValue::Integer(v) => v.to_string(),
            UnifiedValue::String(s) => s,
            UnifiedValue::Bytes(_) => return Err(anyhow!("AccTrie does not support byte values")),
        };

        match self.trie.write().unwrap().insert(keyv, val) {
            Ok(_proof) => Ok(AccTrieProof(Vec::new())),
            Err(e) => Err(anyhow!(e)),
        }
    }

    fn query(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> Result<Option<(Self::Value, Self::Proof)>> {
        let keyv = key.0.clone();
        let guard = self.trie.read().unwrap();
        if let Some(vec) = guard.map.get(&keyv) {
            if !vec.is_empty() {
                return Ok(Some((
                    UnifiedValue::String(vec[0].clone()),
                    AccTrieProof(Vec::new()),
                )));
            }
        }
        Ok(None)
    }

    fn delete(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> Result<Option<Self::Proof>> {
        let keyv = key.0.clone();
        match self.trie.write().unwrap().delete(&keyv, None) {
            Ok(_) => Ok(None),
            Err(_) => Ok(None),
        }
    }

    fn verify(&self, _proof: &Self::Proof) -> bool {
        true
    }

    fn ads_type(&self) -> &'static str {
        "AccTrie"
    }

    fn estimate_proof_size(_proof: &Self::Proof) -> usize {
        0
    }
}

pub use acc::dynamic_accumulator::MembershipProof;

type NodeRef = Arc<RwLock<Node>>;

#[derive(Clone)]
struct KeyedLeaf {
    path: Vec<u8>,
    leaf: NodeRef,
}

#[derive(Clone, Debug)]
pub struct RootEntry {
    pub prefix: Vec<u8>,
    pub acc: G1Affine,
    pub child: NodeRef,
}

#[derive(Debug)]
pub struct RootNode {
    pub entries: Vec<RootEntry>,
    pub acc: G1Affine,
}

#[derive(Debug)]
pub struct ExtensionNode {
    pub children: [Option<NodeRef>; 16],
}

#[derive(Debug)]
pub struct Leaf {
    pub suffix: Vec<u8>,
    pub values: Vec<String>,
    pub prev: Option<NodeRef>,
    pub next: Option<NodeRef>,
    pub acc: G1Affine,
}

#[derive(Debug)]
pub enum Node {
    Root(RootNode),
    Extension(ExtensionNode),
    Leaf(Leaf),
}

#[derive(Clone, Debug)]
pub struct InsertionProof {
    pub key: Vec<u8>,
    pub value: String,
    pub key_prev: Option<Vec<u8>>,
    pub key_next: Option<Vec<u8>>,
    pub ln_acc_old: G1Affine,
    pub ln_acc_new: G1Affine,
    pub ln_prev_acc: Option<G1Affine>,
    pub ln_next_acc_old: Option<G1Affine>,
    pub ln_next_acc_new: Option<G1Affine>,
    pub keyp_in_ln_next_old_proof: Option<MembershipProof>,
    pub keyp_in_ln_proof: Option<MembershipProof>,
    pub no_prev_in_ln_proof: Option<MembershipProof>,
    pub key_in_ln_next_new_proof: Option<MembershipProof>,
    pub keyp_in_ln_next_new_proof: Option<MembershipProof>,
    pub value_in_ln_proof: Option<MembershipProof>,
}

#[derive(Clone, Debug)]
pub struct DeletionProof {
    pub key: Vec<u8>,
    pub delete_entire_leaf: bool,
    pub value: Option<String>,
    pub key_prev: Option<Vec<u8>>,
    pub key_next: Option<Vec<u8>>,
    pub ln_acc_old: G1Affine,
    pub ln_acc_new: Option<G1Affine>,
    pub ln_next_acc_old: Option<G1Affine>,
    pub ln_next_acc_new: Option<G1Affine>,
    pub value_in_ln_old_proof: Option<MembershipProof>,
    pub keyp_in_ln_proof: Option<MembershipProof>,
    pub key_in_ln_next_old_proof: Option<MembershipProof>,
    pub keyp_in_ln_next_new_proof: Option<MembershipProof>,
}

#[derive(Clone, Debug)]
pub struct QueryExistsProof {
    pub key: Vec<u8>,
    pub value: String,
    pub value_count: u64,
    pub ln_acc: G1Affine,
    pub membership_proof: Option<MembershipProof>,
    pub count_membership_proof: Option<MembershipProof>,
    pub root_acc: Option<G1Affine>,
    pub ln_acc_in_root_proof: Option<MembershipProof>,
}

#[derive(Clone, Debug)]
pub struct QueryNotExistsProof {
    pub key: Vec<u8>,
    pub key_prev: Option<Vec<u8>>,
    pub key_next: Option<Vec<u8>>,
    pub ln_next_acc: Option<G1Affine>,
    pub prev_in_next_proof: Option<MembershipProof>,
    pub next_in_next_proof: Option<MembershipProof>,
    pub root_acc: Option<G1Affine>,
    pub ln_next_acc_in_root_proof: Option<MembershipProof>,
}

#[derive(Clone, Debug)]
pub enum QueryResult {
    Exists(QueryExistsProof),
    NotExists(QueryNotExistsProof),
}

#[derive(Debug)]
pub struct AccTrie {
    pub map: HashMap<Vec<u8>, Vec<String>>,
    pub root: NodeRef,
    pub first_leaf: Option<NodeRef>,
    key_to_leaf: HashMap<Vec<u8>, NodeRef>,
    key_to_digest: HashMap<Vec<u8>, [u8; 32]>,
    key_to_root_prefix: HashMap<Vec<u8>, Vec<u8>>,
    sorted_keys: Vec<Vec<u8>>,
    sorted_key_digests: Vec<[u8; 32]>,
    root_snapshot: Vec<Vec<u8>>,
    root_hash: Vec<u8>,
}

impl AccTrie {
    fn key_digest(key: &[u8]) -> [u8; 32] {
        Sha256::digest(key).into()
    }

    fn root_prefix_from_digest(digest: &[u8; 32]) -> Vec<u8> {
        vec![1, digest[0] >> 4]
    }

    fn ensure_key_metadata(&mut self, key: &[u8]) -> ([u8; 32], Vec<u8>) {
        if let (Some(digest), Some(prefix)) = (
            self.key_to_digest.get(key).copied(),
            self.key_to_root_prefix.get(key).cloned(),
        ) {
            return (digest, prefix);
        }

        let digest = Self::key_digest(key);
        let prefix = Self::root_prefix_from_digest(&digest);
        self.key_to_digest.insert(key.to_vec(), digest);
        self.key_to_root_prefix.insert(key.to_vec(), prefix.clone());
        (digest, prefix)
    }

    fn cached_key_digest(&self, key: &[u8]) -> [u8; 32] {
        self.key_to_digest
            .get(key)
            .copied()
            .unwrap_or_else(|| Self::key_digest(key))
    }

    fn cached_root_prefix_for_key(&self, key: &[u8]) -> Vec<u8> {
        self.key_to_root_prefix
            .get(key)
            .cloned()
            .unwrap_or_else(|| Self::root_prefix_from_digest(&self.cached_key_digest(key)))
    }

    fn remove_key_metadata(&mut self, key: &[u8]) {
        self.key_to_digest.remove(key);
        self.key_to_root_prefix.remove(key);
    }

    pub fn hashed_key_hex(key: &[u8]) -> String {
        hex::encode(Self::key_digest(key))
    }

    pub fn key_matches_hashed_prefix(key: &[u8], prefix_hex: &str) -> bool {
        if prefix_hex.is_empty() {
            return true;
        }
        Self::hashed_key_hex(key).starts_with(prefix_hex)
    }

    pub fn root_prefix_from_hex_prefix(prefix_hex: &str) -> Result<Vec<u8>, String> {
        let Some(first) = prefix_hex.chars().next() else {
            return Err("migration prefix must not be empty".to_string());
        };
        let digit = first
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex prefix: {prefix_hex}"))? as u8;
        Ok(vec![1, digit])
    }

    pub fn root_prefix_hex(root_prefix: &[u8]) -> Result<String, String> {
        if root_prefix.len() != 2 || root_prefix[0] != 1 || root_prefix[1] > 0x0f {
            return Err("invalid root prefix encoding".to_string());
        }
        Ok(format!("{:x}", root_prefix[1]))
    }

    pub fn root_prefix_hex_for_key(key: &[u8]) -> String {
        let prefix = Self::root_prefix_for_key(key);
        Self::root_prefix_hex(&prefix).unwrap_or_default()
    }

    fn compare_key_order(left: &[u8], right: &[u8]) -> Ordering {
        Self::key_digest(left).cmp(&Self::key_digest(right))
    }

    fn compare_digest_order(left: &[u8; 32], right: &[u8; 32]) -> Ordering {
        left.cmp(right)
    }

    fn element_to_fr(element: &impl Digestible) -> Fr {
        digest_to_prime_field(&element.to_digest())
    }

    fn key_to_fr(key: &[u8]) -> Fr {
        digest_to_prime_field(&key.to_digest())
    }

    fn membership_proof_for_digest(acc: &G1Affine, element: Fr) -> MembershipProof {
        MembershipProof {
            witness: Acc::create_witness_digest(acc, &element),
            element,
        }
    }

    fn membership_proof_for_key(acc: &G1Affine, key: &[u8]) -> MembershipProof {
        Self::membership_proof_for_digest(acc, Self::key_to_fr(key))
    }

    fn membership_proof_for_value(acc: &G1Affine, value: &str) -> MembershipProof {
        let value = value.to_string();
        Self::membership_proof_for_digest(acc, Self::element_to_fr(&value))
    }

    fn membership_proof_for_acc(acc: &G1Affine, leaf_acc: &G1Affine) -> MembershipProof {
        Self::membership_proof_for_digest(acc, Self::element_to_fr(leaf_acc))
    }

    fn membership_proof_for_count(acc: &G1Affine, count: usize) -> MembershipProof {
        Self::membership_proof_for_digest(acc, Fr::from(count as u64))
    }

    fn empty_accumulator() -> G1Affine {
        Acc::cal_acc_g1(&MultiSet::<Fr>::new())
    }

    fn hash_root_snapshot(snapshot: &[Vec<u8>]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        if snapshot.is_empty() {
            hasher.update(b"empty_acctrie");
        } else {
            for acc in snapshot {
                hasher.update(acc);
            }
        }
        hasher.finalize().to_vec()
    }

    fn refresh_root_hash(&mut self) {
        self.root_hash = Self::hash_root_snapshot(&self.root_snapshot);
    }

    fn key_path(key: &[u8]) -> Vec<u8> {
        let digest = Self::key_digest(key);
        Self::key_path_from_digest(&digest)
    }

    fn key_path_from_digest(digest: &[u8; 32]) -> Vec<u8> {
        let mut path = Vec::with_capacity(digest.len() * 4 + 1);
        for byte in digest.iter().copied() {
            for nibble in [byte >> 4, byte & 0x0f] {
                path.push(1);
                path.push(nibble);
            }
        }
        path.push(0);
        path
    }

    fn root_prefix_from_path(path: &[u8]) -> Vec<u8> {
        path[..2].to_vec()
    }

    fn root_prefix_for_key(key: &[u8]) -> Vec<u8> {
        let digest = Self::key_digest(key);
        Self::root_prefix_from_digest(&digest)
    }

    fn grouped_by_digit(items: &[KeyedLeaf], depth: usize) -> Vec<Vec<KeyedLeaf>> {
        if items.is_empty() {
            return Vec::new();
        }

        if items.iter().all(|item| depth >= item.path.len()) {
            return vec![items.to_vec()];
        }

        let mut groups = Vec::new();
        let mut start = 0;
        while start < items.len() {
            if depth >= items[start].path.len() {
                groups.push(items[start..].to_vec());
                break;
            }
            let digit = items[start].path[depth];
            let mut end = start + 1;
            while end < items.len()
                && depth < items[end].path.len()
                && items[end].path[depth] == digit
            {
                end += 1;
            }
            groups.push(items[start..end].to_vec());
            start = end;
        }
        groups
    }

    fn subtree_acc(items: &[KeyedLeaf]) -> G1Affine {
        let leaf_accs: Vec<Fr> = items
            .iter()
            .map(|item| {
                let guard = item.leaf.read().unwrap();
                match &*guard {
                    Node::Leaf(leaf) => Self::element_to_fr(&leaf.acc),
                    _ => unreachable!("keyed leaf must point to a leaf node"),
                }
            })
            .collect();
        Acc::cal_acc_g1(&MultiSet::from_vec(leaf_accs))
    }

    fn set_leaf_suffix(leaf_ref: &NodeRef, suffix: Vec<u8>) {
        let mut guard = leaf_ref.write().unwrap();
        match &mut *guard {
            Node::Leaf(leaf) => leaf.suffix = suffix,
            _ => unreachable!("expected leaf node"),
        }
    }

    fn build_extension(items: &[KeyedLeaf], depth: usize) -> NodeRef {
        if items.len() == 1 || items.iter().all(|item| depth >= item.path.len()) {
            let suffix = if depth >= items[0].path.len() {
                Vec::new()
            } else {
                items[0].path[depth..].to_vec()
            };
            Self::set_leaf_suffix(&items[0].leaf, suffix);
            return items[0].leaf.clone();
        }

        let mut children: [Option<NodeRef>; 16] = array::from_fn(|_| None);
        for group in Self::grouped_by_digit(items, depth) {
            if group.is_empty() {
                continue;
            }
            if depth >= group[0].path.len() {
                let suffix = Vec::new();
                Self::set_leaf_suffix(&group[0].leaf, suffix);
                return group[0].leaf.clone();
            }
            let digit = group[0].path[depth] as usize;
            let child = if group.len() == 1 {
                let suffix = group[0].path[(depth + 1)..].to_vec();
                Self::set_leaf_suffix(&group[0].leaf, suffix);
                group[0].leaf.clone()
            } else {
                Self::build_extension(&group, depth + 1)
            };
            children[digit] = Some(child);
        }

        Arc::new(RwLock::new(Node::Extension(ExtensionNode { children })))
    }

    fn build_root_child(items: &[KeyedLeaf], prefix_len: usize) -> NodeRef {
        if items.len() == 1 {
            let suffix = items[0].path[prefix_len..].to_vec();
            Self::set_leaf_suffix(&items[0].leaf, suffix);
            items[0].leaf.clone()
        } else {
            Self::build_extension(items, prefix_len)
        }
    }

    #[allow(dead_code)]
    fn build_root_entries(items: &[KeyedLeaf]) -> Vec<RootEntry> {
        let mut entries = Vec::new();
        let mut start = 0;
        while start < items.len() {
            let prefix = Self::root_prefix_from_path(&items[start].path);
            let mut end = start + 1;
            while end < items.len() && Self::root_prefix_from_path(&items[end].path) == prefix {
                end += 1;
            }

            let group = &items[start..end];
            entries.push(RootEntry {
                prefix: prefix.clone(),
                acc: Self::subtree_acc(group),
                child: Self::build_root_child(group, prefix.len()),
            });
            start = end;
        }

        entries.sort_by(|left, right| left.prefix.cmp(&right.prefix));
        entries
    }

    fn leaf_accumulator(key: &[u8], values: &[String], prev_key: Option<&[u8]>) -> G1Affine {
        let mut elements: Vec<Fr> = values.iter().map(Self::element_to_fr).collect();
        elements.push(Fr::from(values.len() as u64));
        elements.push(Self::key_to_fr(key));
        if let Some(prev_key) = prev_key {
            elements.push(Self::key_to_fr(prev_key));
        }
        Acc::cal_acc_g1(&MultiSet::from_vec(elements))
    }

    fn key_position(&self, key: &[u8]) -> Result<usize, usize> {
        let target = self.cached_key_digest(key);
        self.sorted_key_digests
            .binary_search_by(|candidate| Self::compare_digest_order(candidate, &target))
    }

    fn neighbors_around(&self, key: &[u8]) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        match self.key_position(key) {
            Ok(index) => (
                index
                    .checked_sub(1)
                    .map(|idx| self.sorted_keys[idx].clone()),
                self.sorted_keys.get(index + 1).cloned(),
            ),
            Err(index) => (
                index
                    .checked_sub(1)
                    .map(|idx| self.sorted_keys[idx].clone()),
                self.sorted_keys.get(index).cloned(),
            ),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn ordered_leaf_accs(&self) -> Vec<G1Affine> {
        self.sorted_keys
            .iter()
            .filter_map(|key| self.leaf_acc_for_key(key))
            .collect()
    }

    fn leaf_ref_for_key(&self, key: &[u8]) -> Option<NodeRef> {
        self.key_to_leaf.get(key).cloned()
    }

    fn leaf_acc_for_key(&self, key: &[u8]) -> Option<G1Affine> {
        let leaf_ref = self.key_to_leaf.get(key)?;
        let guard = leaf_ref.read().unwrap();
        match &*guard {
            Node::Leaf(leaf) => Some(leaf.acc),
            _ => None,
        }
    }

    fn root_entry_index(entries: &[RootEntry], prefix: &[u8]) -> Result<usize, usize> {
        entries.binary_search_by(|entry| entry.prefix.as_slice().cmp(prefix))
    }

    fn root_entry_acc_for_key(&self, key: &[u8]) -> Option<G1Affine> {
        let prefix = Self::root_prefix_for_key(key);
        let guard = self.root.read().unwrap();
        let root = match &*guard {
            Node::Root(root) => root,
            _ => return None,
        };

        Self::root_entry_index(&root.entries, &prefix)
            .ok()
            .map(|index| root.entries[index].acc)
    }

    pub fn root_accumulator(&self) -> Option<G1Affine> {
        let guard = self.root.read().unwrap();
        match &*guard {
            Node::Root(root) if !root.entries.is_empty() => Some(root.acc),
            Node::Root(_) => None,
            _ => None,
        }
    }

    pub fn root_accumulator_bytes(&self) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let Some(root_acc) = self.root_accumulator() else {
            return Vec::new();
        };

        let mut bytes = Vec::new();
        root_acc
            .serialize_uncompressed(&mut bytes)
            .expect("serialize root accumulator");
        bytes
    }

    fn serialize_root_entry(entry: &RootEntry) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(entry.prefix.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&entry.prefix);

        let mut acc_bytes = Vec::new();
        entry
            .acc
            .serialize_uncompressed(&mut acc_bytes)
            .expect("serialize root entry accumulator");
        bytes.extend_from_slice(&(acc_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&acc_bytes);

        let child_tag = match &*entry.child.read().unwrap() {
            Node::Leaf(_) => 0u8,
            Node::Extension(_) => 1u8,
            Node::Root(_) => 2u8,
        };
        bytes.push(child_tag);

        bytes
    }

    fn count_element(count: usize) -> Fr {
        Fr::from(count as u64)
    }

    fn new_leaf_node(
        key: &[u8],
        values: Vec<String>,
        prev: Option<NodeRef>,
        next: Option<NodeRef>,
        prev_key: Option<&[u8]>,
    ) -> NodeRef {
        Arc::new(RwLock::new(Node::Leaf(Leaf {
            suffix: Vec::new(),
            acc: Self::leaf_accumulator(key, &values, prev_key),
            values,
            prev,
            next,
        })))
    }

    fn add_value_to_leaf(leaf_ref: &NodeRef, value: String) -> Result<G1Affine, String> {
        let mut guard = leaf_ref.write().unwrap();
        match &mut *guard {
            Node::Leaf(leaf) => {
                let old_count = leaf.values.len();
                let mut acc = leaf.acc;
                leaf.values.push(value.clone());
                acc = Acc::add_element(&acc, &value).0;
                acc = Acc::update_digest_element(
                    &acc,
                    &Self::count_element(old_count),
                    &Self::count_element(old_count + 1),
                )
                .0;
                leaf.acc = acc;
                Ok(acc)
            }
            _ => Err("expected leaf node".to_string()),
        }
    }

    fn remove_value_from_leaf(leaf_ref: &NodeRef, value: &str) -> Result<G1Affine, String> {
        let mut guard = leaf_ref.write().unwrap();
        match &mut *guard {
            Node::Leaf(leaf) => {
                let removed = leaf
                    .values
                    .iter()
                    .filter(|existing| existing.as_str() == value)
                    .count();
                if removed == 0 {
                    return Err("value not found in leaf".to_string());
                }

                let old_count = leaf.values.len();
                leaf.values.retain(|existing| existing != value);

                let mut acc = leaf.acc;
                let value_element = value.to_string();
                for _ in 0..removed {
                    acc = Acc::remove_element(&acc, &value_element).0;
                }
                acc = Acc::update_digest_element(
                    &acc,
                    &Self::count_element(old_count),
                    &Self::count_element(leaf.values.len()),
                )
                .0;
                leaf.acc = acc;
                Ok(acc)
            }
            _ => Err("expected leaf node".to_string()),
        }
    }

    fn update_leaf_prev_key(
        leaf_ref: &NodeRef,
        old_prev_key: Option<&[u8]>,
        new_prev_key: Option<&[u8]>,
        prev: Option<NodeRef>,
        next: Option<NodeRef>,
    ) -> Result<G1Affine, String> {
        let mut guard = leaf_ref.write().unwrap();
        match &mut *guard {
            Node::Leaf(leaf) => {
                let mut acc = leaf.acc;
                match (old_prev_key, new_prev_key) {
                    (Some(old_prev_key), Some(new_prev_key)) if old_prev_key != new_prev_key => {
                        acc = Acc::update_element(
                            &acc,
                            &old_prev_key.to_vec(),
                            &new_prev_key.to_vec(),
                        )
                        .0;
                    }
                    (Some(old_prev_key), None) => {
                        acc = Acc::remove_element(&acc, &old_prev_key.to_vec()).0;
                    }
                    (None, Some(new_prev_key)) => {
                        acc = Acc::add_element(&acc, &new_prev_key.to_vec()).0;
                    }
                    _ => {}
                }

                leaf.prev = prev;
                leaf.next = next;
                leaf.acc = acc;
                Ok(acc)
            }
            _ => Err("expected leaf node".to_string()),
        }
    }

    fn keyed_leaves_for_prefix(&self, prefix: &[u8]) -> Vec<KeyedLeaf> {
        self.sorted_keys
            .iter()
            .filter_map(|key| {
                let path = Self::key_path(key);
                if path.starts_with(prefix) {
                    self.leaf_ref_for_key(key)
                        .map(|leaf| KeyedLeaf { path, leaf })
                } else {
                    None
                }
            })
            .collect()
    }

    fn add_root_entry_leaf_acc(&mut self, prefix: &[u8], leaf_acc: &G1Affine) -> bool {
        let Some((index, updated_entry)) = ({
            let mut guard = self.root.write().unwrap();
            let root = match &mut *guard {
                Node::Root(root) => root,
                _ => return false,
            };

            let Ok(index) = Self::root_entry_index(&root.entries, prefix) else {
                return false;
            };
            root.entries[index].acc = Acc::add_element(&root.entries[index].acc, leaf_acc).0;
            Some((index, Self::serialize_root_entry(&root.entries[index])))
        }) else {
            return false;
        };

        self.root_snapshot[index] = updated_entry;
        self.refresh_root_hash();
        true
    }

    fn add_root_leaf_acc(&mut self, leaf_acc: &G1Affine) -> bool {
        let mut guard = self.root.write().unwrap();
        let root = match &mut *guard {
            Node::Root(root) => root,
            _ => return false,
        };
        root.acc = Acc::add_element(&root.acc, leaf_acc).0;
        true
    }

    fn replace_root_entry_leaf_acc(
        &mut self,
        prefix: &[u8],
        old_leaf_acc: &G1Affine,
        new_leaf_acc: &G1Affine,
    ) -> bool {
        if old_leaf_acc == new_leaf_acc {
            return true;
        }

        let Some((index, updated_entry)) = ({
            let mut guard = self.root.write().unwrap();
            let root = match &mut *guard {
                Node::Root(root) => root,
                _ => return false,
            };

            let Ok(index) = Self::root_entry_index(&root.entries, prefix) else {
                return false;
            };
            root.entries[index].acc =
                Acc::update_element(&root.entries[index].acc, old_leaf_acc, new_leaf_acc).0;
            Some((index, Self::serialize_root_entry(&root.entries[index])))
        }) else {
            return false;
        };

        self.root_snapshot[index] = updated_entry;
        self.refresh_root_hash();
        true
    }

    fn replace_root_leaf_acc(&mut self, old_leaf_acc: &G1Affine, new_leaf_acc: &G1Affine) -> bool {
        if old_leaf_acc == new_leaf_acc {
            return true;
        }

        let mut guard = self.root.write().unwrap();
        let root = match &mut *guard {
            Node::Root(root) => root,
            _ => return false,
        };
        root.acc = Acc::update_element(&root.acc, old_leaf_acc, new_leaf_acc).0;
        true
    }

    fn remove_root_entry_leaf_acc(&mut self, prefix: &[u8], leaf_acc: &G1Affine) -> bool {
        let Some((index, updated_entry)) = ({
            let mut guard = self.root.write().unwrap();
            let root = match &mut *guard {
                Node::Root(root) => root,
                _ => return false,
            };

            let Ok(index) = Self::root_entry_index(&root.entries, prefix) else {
                return false;
            };
            root.entries[index].acc = Acc::remove_element(&root.entries[index].acc, leaf_acc).0;
            Some((index, Self::serialize_root_entry(&root.entries[index])))
        }) else {
            return false;
        };

        self.root_snapshot[index] = updated_entry;
        self.refresh_root_hash();
        true
    }

    fn remove_root_leaf_acc(&mut self, leaf_acc: &G1Affine) -> bool {
        let mut guard = self.root.write().unwrap();
        let root = match &mut *guard {
            Node::Root(root) => root,
            _ => return false,
        };
        root.acc = Acc::remove_element(&root.acc, leaf_acc).0;
        true
    }

    fn sync_root_entry(&mut self, prefix: &[u8]) {
        let keyed = self.keyed_leaves_for_prefix(prefix);
        enum SyncUpdate {
            Remove(usize),
            Replace(usize, Vec<u8>),
            Insert(usize, Vec<u8>),
            Noop,
        }

        let update = {
            let mut guard = self.root.write().unwrap();
            let root = match &mut *guard {
                Node::Root(root) => root,
                _ => return,
            };

            match Self::root_entry_index(&root.entries, prefix) {
                Ok(index) => {
                    if keyed.is_empty() {
                        root.entries.remove(index);
                        SyncUpdate::Remove(index)
                    } else {
                        root.entries[index].acc = Self::subtree_acc(&keyed);
                        root.entries[index].child = Self::build_root_child(&keyed, prefix.len());
                        SyncUpdate::Replace(index, Self::serialize_root_entry(&root.entries[index]))
                    }
                }
                Err(index) => {
                    if keyed.is_empty() {
                        SyncUpdate::Noop
                    } else {
                        root.entries.insert(
                            index,
                            RootEntry {
                                prefix: prefix.to_vec(),
                                acc: Self::subtree_acc(&keyed),
                                child: Self::build_root_child(&keyed, prefix.len()),
                            },
                        );
                        SyncUpdate::Insert(index, Self::serialize_root_entry(&root.entries[index]))
                    }
                }
            }
        };

        match update {
            SyncUpdate::Remove(index) => {
                self.root_snapshot.remove(index);
                self.refresh_root_hash();
            }
            SyncUpdate::Replace(index, bytes) => {
                self.root_snapshot[index] = bytes;
                self.refresh_root_hash();
            }
            SyncUpdate::Insert(index, bytes) => {
                self.root_snapshot.insert(index, bytes);
                self.refresh_root_hash();
            }
            SyncUpdate::Noop => {}
        }
    }

    #[allow(dead_code)]
    fn rebuild_root_entries(&mut self) {
        if self.sorted_keys.is_empty() {
            self.root = Arc::new(RwLock::new(Node::Root(RootNode {
                entries: Vec::new(),
                acc: Self::empty_accumulator(),
            })));
            self.root_snapshot.clear();
            self.refresh_root_hash();
            self.first_leaf = None;
            return;
        }

        let mut keyed = Vec::with_capacity(self.sorted_keys.len());
        for key in &self.sorted_keys {
            let leaf = self
                .leaf_ref_for_key(key)
                .expect("sorted key must point to an existing leaf");
            keyed.push(KeyedLeaf {
                path: Self::key_path(key),
                leaf,
            });
        }

        self.first_leaf = keyed.first().map(|item| item.leaf.clone());
        let entries = Self::build_root_entries(&keyed);
        self.root_snapshot = entries.iter().map(Self::serialize_root_entry).collect();
        self.refresh_root_hash();
        self.root = Arc::new(RwLock::new(Node::Root(RootNode {
            acc: Self::subtree_acc(&keyed),
            entries,
        })));
    }

    fn build_non_membership_proof(&self, key: &[u8]) -> QueryNotExistsProof {
        if self.sorted_keys.is_empty() {
            return QueryNotExistsProof {
                key: key.to_vec(),
                key_prev: None,
                key_next: None,
                ln_next_acc: None,
                prev_in_next_proof: None,
                next_in_next_proof: None,
                root_acc: None,
                ln_next_acc_in_root_proof: None,
            };
        }

        let (key_prev, key_next) = self.neighbors_around(key);
        if let Some(ref next_key) = key_next {
            let ln_next_acc = self
                .leaf_acc_for_key(next_key)
                .expect("next leaf accumulator must exist");
            let root_acc = self.root_entry_acc_for_key(next_key);
            let prev_in_next_proof = key_prev
                .as_ref()
                .map(|prev_key| Self::membership_proof_for_key(&ln_next_acc, prev_key));
            let next_in_next_proof = Some(Self::membership_proof_for_key(&ln_next_acc, next_key));
            let ln_next_acc_in_root_proof = root_acc
                .as_ref()
                .map(|root| Self::membership_proof_for_acc(root, &ln_next_acc));

            QueryNotExistsProof {
                key: key.to_vec(),
                key_prev,
                key_next,
                ln_next_acc: Some(ln_next_acc),
                prev_in_next_proof,
                next_in_next_proof,
                root_acc,
                ln_next_acc_in_root_proof,
            }
        } else {
            QueryNotExistsProof {
                key: key.to_vec(),
                key_prev,
                key_next: None,
                ln_next_acc: None,
                prev_in_next_proof: None,
                next_in_next_proof: None,
                root_acc: None,
                ln_next_acc_in_root_proof: None,
            }
        }
    }

    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            root: Arc::new(RwLock::new(Node::Root(RootNode {
                entries: Vec::new(),
                acc: Self::empty_accumulator(),
            }))),
            first_leaf: None,
            key_to_leaf: HashMap::new(),
            key_to_digest: HashMap::new(),
            key_to_root_prefix: HashMap::new(),
            sorted_keys: Vec::new(),
            sorted_key_digests: Vec::new(),
            root_snapshot: Vec::new(),
            root_hash: Self::hash_root_snapshot(&[]),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.map.values().all(Vec::is_empty)
    }

    pub fn records(&self) -> Vec<PersistedRecord> {
        self.sorted_keys
            .iter()
            .filter_map(|key| {
                self.map.get(key).map(|values| PersistedRecord {
                    key: key.clone(),
                    values: values.clone(),
                })
            })
            .collect()
    }

    pub fn records_for_root_prefix(&self, root_prefix: &[u8]) -> Vec<PersistedRecord> {
        self.sorted_keys
            .iter()
            .filter(|key| self.cached_root_prefix_for_key(key) == root_prefix)
            .filter_map(|key| {
                self.map.get(key).map(|values| PersistedRecord {
                    key: key.clone(),
                    values: values.clone(),
                })
            })
            .collect()
    }

    pub fn records_for_hashed_prefix(&self, prefix_hex: &str) -> Vec<PersistedRecord> {
        self.records()
            .into_iter()
            .filter(|record| Self::key_matches_hashed_prefix(&record.key, prefix_hex))
            .collect()
    }

    pub fn restore_from_records(&mut self, records: Vec<PersistedRecord>) -> Result<(), String> {
        let mut prepared_records = records
            .into_iter()
            .filter(|record| !record.values.is_empty())
            .map(|record| {
                let digest = Self::key_digest(&record.key);
                let prefix = Self::root_prefix_from_digest(&digest);
                let path = Self::key_path_from_digest(&digest);
                (record, digest, prefix, path)
            })
            .collect::<Vec<_>>();
        prepared_records.sort_by(|left, right| Self::compare_digest_order(&left.1, &right.1));

        self.map.clear();
        self.key_to_leaf.clear();
        self.key_to_digest.clear();
        self.key_to_root_prefix.clear();
        self.sorted_keys.clear();
        self.sorted_key_digests.clear();
        self.first_leaf = None;

        self.map.reserve(prepared_records.len());
        self.key_to_leaf.reserve(prepared_records.len());
        self.key_to_digest.reserve(prepared_records.len());
        self.key_to_root_prefix.reserve(prepared_records.len());
        self.sorted_keys.reserve(prepared_records.len());
        self.sorted_key_digests.reserve(prepared_records.len());

        if prepared_records.is_empty() {
            self.root = Arc::new(RwLock::new(Node::Root(RootNode {
                entries: Vec::new(),
                acc: Self::empty_accumulator(),
            })));
            self.root_snapshot.clear();
            self.refresh_root_hash();
            return Ok(());
        }

        let mut keyed = Vec::with_capacity(prepared_records.len());
        let mut previous_leaf: Option<NodeRef> = None;
        let mut previous_key: Option<Vec<u8>> = None;

        for (record, digest, prefix, path) in prepared_records {
            self.map.insert(record.key.clone(), record.values.clone());
            self.sorted_keys.push(record.key.clone());
            self.sorted_key_digests.push(digest);
            self.key_to_digest.insert(record.key.clone(), digest);
            self.key_to_root_prefix.insert(record.key.clone(), prefix);

            let leaf = Self::new_leaf_node(
                &record.key,
                record.values.clone(),
                previous_leaf.clone(),
                None,
                previous_key.as_deref(),
            );

            if let Some(prev_leaf) = previous_leaf.as_ref() {
                let mut guard = prev_leaf.write().unwrap();
                match &mut *guard {
                    Node::Leaf(prev) => prev.next = Some(leaf.clone()),
                    _ => unreachable!("expected leaf node"),
                }
            } else {
                self.first_leaf = Some(leaf.clone());
            }

            self.key_to_leaf.insert(record.key.clone(), leaf.clone());
            keyed.push(KeyedLeaf {
                path,
                leaf: leaf.clone(),
            });

            previous_leaf = Some(leaf);
            previous_key = Some(record.key.clone());
        }

        let entries = Self::build_root_entries(&keyed);
        self.root_snapshot = entries.iter().map(Self::serialize_root_entry).collect();
        self.refresh_root_hash();
        self.root = Arc::new(RwLock::new(Node::Root(RootNode {
            acc: Self::subtree_acc(&keyed),
            entries,
        })));

        Ok(())
    }

    pub fn replace_root_prefix_records(
        &mut self,
        root_prefix: &[u8],
        mut records: Vec<PersistedRecord>,
    ) -> Result<(), String> {
        if records
            .iter()
            .any(|record| Self::root_prefix_for_key(&record.key) != root_prefix)
        {
            return Err("record does not belong to the target root prefix".to_string());
        }

        records.retain(|record| !record.values.is_empty());
        records.sort_by(|left, right| Self::compare_key_order(&left.key, &right.key));

        let mut old_keys = Vec::new();
        let mut start_index = None;
        for (index, key) in self.sorted_keys.iter().enumerate() {
            if self.cached_root_prefix_for_key(key) == root_prefix {
                if start_index.is_none() {
                    start_index = Some(index);
                }
                old_keys.push(key.clone());
            }
        }

        let old_leaf_accs: Vec<G1Affine> = old_keys
            .iter()
            .filter_map(|key| self.leaf_acc_for_key(key))
            .collect();

        let insertion_index = if let Some(index) = start_index {
            index
        } else if let Some(first_new) = records.first() {
            match self
                .sorted_keys
                .binary_search_by(|probe| Self::compare_key_order(probe, &first_new.key))
            {
                Ok(index) | Err(index) => index,
            }
        } else {
            return Ok(());
        };

        let before_key = insertion_index
            .checked_sub(1)
            .and_then(|index| self.sorted_keys.get(index).cloned());
        let after_key = self
            .sorted_keys
            .get(insertion_index + old_keys.len())
            .cloned();

        let before_leaf = before_key
            .as_ref()
            .and_then(|key| self.leaf_ref_for_key(key));
        let after_leaf = after_key
            .as_ref()
            .and_then(|key| self.leaf_ref_for_key(key));
        let after_leaf_old_acc = after_key
            .as_ref()
            .and_then(|key| self.leaf_acc_for_key(key));

        let after_leaf_next = if let Some(after_leaf) = after_leaf.as_ref() {
            let guard = after_leaf.read().unwrap();
            match &*guard {
                Node::Leaf(leaf) => leaf.next.clone(),
                _ => None,
            }
        } else {
            None
        };

        let old_after_prev_key = if after_key.is_some() {
            old_keys.last().cloned().or_else(|| before_key.clone())
        } else {
            None
        };

        for leaf_acc in &old_leaf_accs {
            let _ = self.remove_root_leaf_acc(leaf_acc);
        }

        for key in &old_keys {
            self.map.remove(key);
            self.key_to_leaf.remove(key);
            self.remove_key_metadata(key);
        }

        let retained_keys = self
            .sorted_keys
            .iter()
            .filter(|key| self.cached_root_prefix_for_key(key) != root_prefix)
            .cloned()
            .collect::<Vec<_>>();
        self.sorted_keys = retained_keys;
        self.sorted_key_digests = self
            .sorted_keys
            .iter()
            .map(|key| self.cached_key_digest(key))
            .collect();

        let mut new_leaves = Vec::with_capacity(records.len());
        for (index, record) in records.iter().enumerate() {
            self.map.insert(record.key.clone(), record.values.clone());
            let _ = self.ensure_key_metadata(&record.key);
            let prev_key = if index == 0 {
                before_key.as_deref()
            } else {
                Some(records[index - 1].key.as_slice())
            };
            let leaf =
                Self::new_leaf_node(&record.key, record.values.clone(), None, None, prev_key);
            let leaf_acc = match &*leaf.read().unwrap() {
                Node::Leaf(leaf) => leaf.acc,
                _ => unreachable!("expected leaf node"),
            };
            let _ = self.add_root_leaf_acc(&leaf_acc);
            self.key_to_leaf.insert(record.key.clone(), leaf.clone());
            new_leaves.push((record.key.clone(), leaf));
        }

        for index in 0..new_leaves.len() {
            let prev = if index == 0 {
                before_leaf.clone()
            } else {
                Some(new_leaves[index - 1].1.clone())
            };
            let next = if index + 1 < new_leaves.len() {
                Some(new_leaves[index + 1].1.clone())
            } else {
                after_leaf.clone()
            };

            let mut guard = new_leaves[index].1.write().unwrap();
            match &mut *guard {
                Node::Leaf(leaf) => {
                    leaf.prev = prev;
                    leaf.next = next;
                }
                _ => unreachable!("expected leaf node"),
            }
        }

        if let Some(before_leaf) = before_leaf.as_ref() {
            let mut guard = before_leaf.write().unwrap();
            match &mut *guard {
                Node::Leaf(leaf) => {
                    leaf.next = new_leaves
                        .first()
                        .map(|(_, leaf)| leaf.clone())
                        .or_else(|| after_leaf.clone());
                }
                _ => unreachable!("expected leaf node"),
            }
        } else {
            self.first_leaf = new_leaves
                .first()
                .map(|(_, leaf)| leaf.clone())
                .or_else(|| after_leaf.clone());
        }

        if let Some(after_leaf) = after_leaf.as_ref() {
            let new_prev_key = new_leaves
                .last()
                .map(|(key, _)| key.clone())
                .or_else(|| before_key.clone());
            let new_prev_leaf = new_leaves
                .last()
                .map(|(_, leaf)| leaf.clone())
                .or_else(|| before_leaf.clone());
            let new_after_acc = Self::update_leaf_prev_key(
                after_leaf,
                old_after_prev_key.as_deref(),
                new_prev_key.as_deref(),
                new_prev_leaf,
                after_leaf_next,
            )?;

            if let (Some(after_key), Some(old_after_acc)) =
                (after_key.as_ref(), after_leaf_old_acc.as_ref())
            {
                let after_prefix = Self::root_prefix_for_key(after_key);
                let _ =
                    self.replace_root_entry_leaf_acc(&after_prefix, old_after_acc, &new_after_acc);
                let _ = self.replace_root_leaf_acc(old_after_acc, &new_after_acc);
            }
        }

        let insert_pos = if let Some((first_key, _)) = new_leaves.first() {
            match self
                .sorted_keys
                .binary_search_by(|probe| Self::compare_key_order(probe, first_key))
            {
                Ok(index) | Err(index) => index,
            }
        } else {
            insertion_index.min(self.sorted_keys.len())
        };

        for (offset, (key, _)) in new_leaves.iter().enumerate() {
            self.sorted_keys.insert(insert_pos + offset, key.clone());
            self.sorted_key_digests
                .insert(insert_pos + offset, self.cached_key_digest(key));
        }

        self.sync_root_entry(root_prefix);
        Ok(())
    }

    pub fn accumulator_snapshot(&self) -> Vec<Vec<u8>> {
        self.root_snapshot.clone()
    }

    pub fn root_hash(&self) -> Vec<u8> {
        self.root_hash.clone()
    }

    pub fn insert(&mut self, key: Vec<u8>, value: String) -> Result<InsertionProof, String> {
        let _ = self.ensure_key_metadata(&key);
        let old_leaf_acc = self
            .leaf_acc_for_key(&key)
            .unwrap_or_else(Self::empty_accumulator);
        let old_neighbors = self.neighbors_around(&key);
        let old_next_acc = old_neighbors
            .1
            .as_ref()
            .and_then(|next_key| self.leaf_acc_for_key(next_key));
        let key_prefix = Self::root_prefix_for_key(&key);
        let had_root_entry = self.root_entry_acc_for_key(&key).is_some();

        match self.key_position(&key) {
            Ok(_) => {
                self.map.entry(key.clone()).or_default().push(value.clone());

                let leaf_ref = self
                    .leaf_ref_for_key(&key)
                    .ok_or_else(|| "existing leaf missing".to_string())?;
                let new_leaf_acc = Self::add_value_to_leaf(&leaf_ref, value.clone())?;
                let _ = self.replace_root_entry_leaf_acc(&key_prefix, &old_leaf_acc, &new_leaf_acc);
                let _ = self.replace_root_leaf_acc(&old_leaf_acc, &new_leaf_acc);
            }
            Err(index) => {
                let prev_key = index
                    .checked_sub(1)
                    .map(|idx| self.sorted_keys[idx].clone());
                let next_key = self.sorted_keys.get(index).cloned();
                let prev_leaf = prev_key
                    .as_ref()
                    .and_then(|prev_key| self.leaf_ref_for_key(prev_key));
                let next_leaf = next_key
                    .as_ref()
                    .and_then(|next_key| self.leaf_ref_for_key(next_key));

                let values = self.map.entry(key.clone()).or_default();
                values.push(value.clone());
                let new_leaf = Self::new_leaf_node(
                    &key,
                    values.clone(),
                    prev_leaf.clone(),
                    next_leaf.clone(),
                    prev_key.as_deref(),
                );
                let new_leaf_acc = {
                    let guard = new_leaf.read().unwrap();
                    match &*guard {
                        Node::Leaf(leaf) => leaf.acc,
                        _ => unreachable!("expected leaf"),
                    }
                };

                if let Some(ref prev_leaf) = prev_leaf {
                    let mut guard = prev_leaf.write().unwrap();
                    match &mut *guard {
                        Node::Leaf(leaf) => leaf.next = Some(new_leaf.clone()),
                        _ => unreachable!("expected leaf"),
                    }
                }

                if let Some(ref next_leaf) = next_leaf {
                    self.map
                        .get(next_key.as_ref().expect("next key must exist"))
                        .cloned()
                        .ok_or_else(|| "next leaf values missing".to_string())?;
                    let next_next = {
                        let guard = next_leaf.read().unwrap();
                        match &*guard {
                            Node::Leaf(leaf) => leaf.next.clone(),
                            _ => unreachable!("expected leaf"),
                        }
                    };
                    let next_leaf_acc = Self::update_leaf_prev_key(
                        next_leaf,
                        prev_key.as_deref(),
                        Some(&key),
                        Some(new_leaf.clone()),
                        next_next,
                    )?;
                    let next_prefix =
                        Self::root_prefix_for_key(next_key.as_ref().expect("next key must exist"));
                    if let Some(old_next_acc) = old_next_acc {
                        let _ = self.replace_root_entry_leaf_acc(
                            &next_prefix,
                            &old_next_acc,
                            &next_leaf_acc,
                        );
                        let _ = self.replace_root_leaf_acc(&old_next_acc, &next_leaf_acc);
                    }
                }

                self.sorted_keys.insert(index, key.clone());
                self.sorted_key_digests
                    .insert(index, self.cached_key_digest(&key));
                self.key_to_leaf.insert(key.clone(), new_leaf.clone());
                self.first_leaf = self
                    .sorted_keys
                    .first()
                    .and_then(|first_key| self.leaf_ref_for_key(first_key));

                if had_root_entry {
                    let _ = self.add_root_entry_leaf_acc(&key_prefix, &new_leaf_acc);
                }
                let _ = self.add_root_leaf_acc(&new_leaf_acc);
                self.sync_root_entry(&key_prefix);
            }
        }

        let (key_prev, key_next) = self.neighbors_around(&key);
        let ln_acc_new = self
            .leaf_acc_for_key(&key)
            .ok_or_else(|| "inserted leaf accumulator missing".to_string())?;
        let ln_prev_acc = key_prev
            .as_ref()
            .and_then(|prev_key| self.leaf_acc_for_key(prev_key));
        let ln_next_acc_new = key_next
            .as_ref()
            .and_then(|next_key| self.leaf_acc_for_key(next_key));

        let keyp_in_ln_next_old_proof = match (&key_prev, old_next_acc) {
            (Some(prev_key), Some(next_acc)) => {
                Some(Self::membership_proof_for_key(&next_acc, prev_key))
            }
            _ => None,
        };
        let keyp_in_ln_proof = key_prev
            .as_ref()
            .map(|prev_key| Self::membership_proof_for_key(&ln_acc_new, prev_key));
        let no_prev_in_ln_proof = if key_prev.is_none() {
            Some(Self::membership_proof_for_count(
                &ln_acc_new,
                self.map.get(&key).map(|values| values.len()).unwrap_or(0),
            ))
        } else {
            None
        };
        let key_in_ln_next_new_proof = ln_next_acc_new
            .as_ref()
            .map(|next_acc| Self::membership_proof_for_key(next_acc, &key));
        let keyp_in_ln_next_new_proof = match (&key_next, ln_next_acc_new.as_ref()) {
            (Some(next_key), Some(next_acc)) => {
                Some(Self::membership_proof_for_key(next_acc, next_key))
            }
            _ => None,
        };

        Ok(InsertionProof {
            key,
            value: value.clone(),
            key_prev,
            key_next,
            ln_acc_old: old_leaf_acc,
            ln_acc_new,
            ln_prev_acc,
            ln_next_acc_old: old_next_acc,
            ln_next_acc_new,
            keyp_in_ln_next_old_proof,
            keyp_in_ln_proof,
            no_prev_in_ln_proof,
            key_in_ln_next_new_proof,
            keyp_in_ln_next_new_proof,
            value_in_ln_proof: Some(Self::membership_proof_for_value(&ln_acc_new, &value)),
        })
    }

    pub fn delete(
        &mut self,
        key: &Vec<u8>,
        value: Option<String>,
    ) -> Result<DeletionProof, String> {
        let old_values = self
            .map
            .get(key)
            .cloned()
            .ok_or_else(|| "key not found".to_string())?;
        if old_values.is_empty() {
            return Err("key not found".to_string());
        }

        let position = self
            .key_position(key)
            .map_err(|_| "key not found".to_string())?;
        let (key_prev, key_next) = self.neighbors_around(key);
        let ln_acc_old = self
            .leaf_acc_for_key(key)
            .ok_or_else(|| "old leaf accumulator missing".to_string())?;
        let ln_next_acc_old = key_next
            .as_ref()
            .and_then(|next_key| self.leaf_acc_for_key(next_key));
        let key_prefix = Self::root_prefix_for_key(key);

        let delete_entire_leaf;
        if let Some(ref target_value) = value {
            if !old_values.contains(target_value) {
                return Err("value not found".to_string());
            }
            let Some(values) = self.map.get_mut(key) else {
                return Err("key not found".to_string());
            };
            values.retain(|existing| existing != target_value);
            delete_entire_leaf = values.is_empty();
            if delete_entire_leaf {
                self.map.remove(key);
            }
        } else {
            self.map.remove(key);
            delete_entire_leaf = true;
        }

        if delete_entire_leaf {
            let prev_leaf = key_prev
                .as_ref()
                .and_then(|prev_key| self.leaf_ref_for_key(prev_key));
            let next_leaf = key_next
                .as_ref()
                .and_then(|next_key| self.leaf_ref_for_key(next_key));

            if let Some(ref prev_leaf) = prev_leaf {
                let mut guard = prev_leaf.write().unwrap();
                match &mut *guard {
                    Node::Leaf(leaf) => leaf.next = next_leaf.clone(),
                    _ => unreachable!("expected leaf"),
                }
            }

            if let Some(ref next_leaf) = next_leaf {
                self.map
                    .get(key_next.as_ref().expect("next key must exist"))
                    .cloned()
                    .ok_or_else(|| "next leaf values missing".to_string())?;
                let next_next = {
                    let guard = next_leaf.read().unwrap();
                    match &*guard {
                        Node::Leaf(leaf) => leaf.next.clone(),
                        _ => unreachable!("expected leaf"),
                    }
                };
                let next_leaf_acc = Self::update_leaf_prev_key(
                    next_leaf,
                    Some(key.as_slice()),
                    key_prev.as_deref(),
                    prev_leaf.clone(),
                    next_next,
                )?;
                let next_prefix =
                    Self::root_prefix_for_key(key_next.as_ref().expect("next key must exist"));
                if let Some(old_next_acc) = ln_next_acc_old {
                    let _ = self.replace_root_entry_leaf_acc(
                        &next_prefix,
                        &old_next_acc,
                        &next_leaf_acc,
                    );
                    let _ = self.replace_root_leaf_acc(&old_next_acc, &next_leaf_acc);
                }
            }

            self.key_to_leaf.remove(key);
            self.sorted_keys.remove(position);
            self.sorted_key_digests.remove(position);
            self.remove_key_metadata(key);
            self.first_leaf = self
                .sorted_keys
                .first()
                .and_then(|first_key| self.leaf_ref_for_key(first_key));
            let _ = self.remove_root_entry_leaf_acc(&key_prefix, &ln_acc_old);
            let _ = self.remove_root_leaf_acc(&ln_acc_old);
            self.sync_root_entry(&key_prefix);
        } else {
            let leaf_ref = self
                .leaf_ref_for_key(key)
                .ok_or_else(|| "updated leaf missing".to_string())?;
            let target_value = value
                .as_deref()
                .expect("partial deletion must target a value");
            let new_leaf_acc = Self::remove_value_from_leaf(&leaf_ref, target_value)?;
            let _ = self.replace_root_entry_leaf_acc(&key_prefix, &ln_acc_old, &new_leaf_acc);
            let _ = self.replace_root_leaf_acc(&ln_acc_old, &new_leaf_acc);
        }

        let ln_acc_new = self.leaf_acc_for_key(key);
        let ln_next_acc_new = key_next
            .as_ref()
            .and_then(|next_key| self.leaf_acc_for_key(next_key));

        let value_in_ln_old_proof = value
            .as_ref()
            .map(|target_value| Self::membership_proof_for_value(&ln_acc_old, target_value));
        let keyp_in_ln_proof = if let Some(ref prev_key) = key_prev {
            Some(Self::membership_proof_for_key(&ln_acc_old, prev_key))
        } else {
            Some(Self::membership_proof_for_count(
                &ln_acc_old,
                old_values.len(),
            ))
        };
        let key_in_ln_next_old_proof = match (&key_next, ln_next_acc_old.as_ref()) {
            (Some(_), Some(next_acc)) if delete_entire_leaf => {
                Some(Self::membership_proof_for_key(next_acc, key))
            }
            _ => None,
        };
        let keyp_in_ln_next_new_proof = match (&key_prev, &key_next, ln_next_acc_new.as_ref()) {
            (Some(prev_key), Some(_), Some(next_acc)) if delete_entire_leaf => {
                Some(Self::membership_proof_for_key(next_acc, prev_key))
            }
            (None, Some(next_key), Some(next_acc)) if delete_entire_leaf => {
                Some(Self::membership_proof_for_key(next_acc, next_key))
            }
            _ => None,
        };

        Ok(DeletionProof {
            key: key.clone(),
            delete_entire_leaf,
            value: value.clone(),
            key_prev,
            key_next,
            ln_acc_old,
            ln_acc_new,
            ln_next_acc_old,
            ln_next_acc_new,
            value_in_ln_old_proof,
            keyp_in_ln_proof,
            key_in_ln_next_old_proof,
            keyp_in_ln_next_new_proof,
        })
    }

    pub fn query(&self, key: &[u8], value: &str) -> Result<QueryResult, String> {
        if let Some(values) = self.map.get(key) {
            if values.iter().any(|existing| existing == value) {
                let ln_acc = self
                    .leaf_acc_for_key(key)
                    .ok_or_else(|| "leaf accumulator missing".to_string())?;
                let root_acc = self.root_entry_acc_for_key(key);
                let ln_acc_in_root_proof = root_acc
                    .as_ref()
                    .map(|root| Self::membership_proof_for_acc(root, &ln_acc));
                Ok(QueryResult::Exists(QueryExistsProof {
                    key: key.to_vec(),
                    value: value.to_string(),
                    value_count: values.len() as u64,
                    ln_acc,
                    membership_proof: Some(Self::membership_proof_for_value(&ln_acc, value)),
                    count_membership_proof: Some(Self::membership_proof_for_count(
                        &ln_acc,
                        values.len(),
                    )),
                    root_acc,
                    ln_acc_in_root_proof,
                }))
            } else {
                Err("value not found for existing key".to_string())
            }
        } else {
            Ok(QueryResult::NotExists(self.build_non_membership_proof(key)))
        }
    }
}

impl QueryNotExistsProof {
    pub fn verify(&self) -> bool {
        match (&self.key_prev, &self.key_next) {
            (Some(prev), Some(next)) => {
                if !matches!(
                    (
                        AccTrie::compare_key_order(prev, &self.key),
                        AccTrie::compare_key_order(&self.key, next)
                    ),
                    (Ordering::Less, Ordering::Less)
                ) {
                    return false;
                }
            }
            (Some(prev), None) => {
                if AccTrie::compare_key_order(prev, &self.key) != Ordering::Less {
                    return false;
                }
            }
            (None, Some(next)) => {
                if AccTrie::compare_key_order(&self.key, next) != Ordering::Less {
                    return false;
                }
            }
            (None, None) => {
                return self.ln_next_acc.is_none()
                    && self.prev_in_next_proof.is_none()
                    && self.next_in_next_proof.is_none()
                    && self.root_acc.is_none()
                    && self.ln_next_acc_in_root_proof.is_none();
            }
        }

        if let Some(ref ln_next_acc) = self.ln_next_acc {
            if let Some(ref key_prev) = self.key_prev {
                let prev_proof = match self.prev_in_next_proof.as_ref() {
                    Some(proof) => proof,
                    None => return false,
                };
                if !Acc::verify_membership(ln_next_acc, &prev_proof.witness, key_prev) {
                    return false;
                }
            }

            let key_next = match self.key_next.as_ref() {
                Some(key_next) => key_next,
                None => return false,
            };
            let next_proof = match self.next_in_next_proof.as_ref() {
                Some(proof) => proof,
                None => return false,
            };
            if !Acc::verify_membership(ln_next_acc, &next_proof.witness, key_next) {
                return false;
            }
        }

        match (
            &self.root_acc,
            &self.ln_next_acc,
            &self.ln_next_acc_in_root_proof,
        ) {
            (Some(root_acc), Some(ln_next_acc), Some(root_proof)) => {
                if !Acc::verify_membership(root_acc, &root_proof.witness, ln_next_acc) {
                    return false;
                }
            }
            (None, None, None) => {}
            (Some(_), None, None) => {}
            _ => return false,
        }

        true
    }
}

impl QueryExistsProof {
    pub fn verify(&self) -> bool {
        let membership_proof = match self.membership_proof.as_ref() {
            Some(proof) => proof,
            None => return false,
        };
        if !membership_proof.verify(self.ln_acc) {
            return false;
        }

        let count_membership_proof = match self.count_membership_proof.as_ref() {
            Some(proof) => proof,
            None => return false,
        };
        if !count_membership_proof.verify(self.ln_acc) {
            return false;
        }

        match (&self.root_acc, &self.ln_acc_in_root_proof) {
            (Some(root_acc), Some(root_proof)) => {
                if !Acc::verify_membership(root_acc, &root_proof.witness, &self.ln_acc) {
                    return false;
                }
            }
            (None, None) => {}
            _ => return false,
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as StdHashMap;

    fn sorted_keys_by_hash(keys: &[&[u8]]) -> Vec<Vec<u8>> {
        let mut ordered = keys.iter().map(|key| key.to_vec()).collect::<Vec<_>>();
        ordered.sort_by(|left, right| AccTrie::compare_key_order(left, right));
        ordered
    }

    fn leaf_acc(node: &NodeRef) -> G1Affine {
        let guard = node.read().unwrap();
        match &*guard {
            Node::Leaf(leaf) => leaf.acc,
            _ => panic!("expected leaf"),
        }
    }

    fn byte_keys_sharing_root_prefix(count: usize) -> Vec<Vec<u8>> {
        let mut groups: StdHashMap<Vec<u8>, Vec<Vec<u8>>> = StdHashMap::new();
        for byte in 0u16..=255 {
            let key = vec![byte as u8];
            let prefix = AccTrie::root_prefix_for_key(&key);
            let group = groups.entry(prefix).or_default();
            group.push(key.clone());
            if group.len() == count {
                return group.clone();
            }
        }

        panic!("failed to find enough keys with the same root prefix");
    }

    fn root_entry_child(trie: &AccTrie, prefix: &[u8]) -> NodeRef {
        let guard = trie.root.read().unwrap();
        let root = match &*guard {
            Node::Root(root) => root,
            _ => panic!("expected root node"),
        };
        let index = AccTrie::root_entry_index(&root.entries, prefix).expect("root entry");
        root.entries[index].child.clone()
    }

    #[test]
    fn rebuilds_root_extension_and_leaf_layers() {
        let mut trie = AccTrie::new();
        let keys = byte_keys_sharing_root_prefix(3);
        trie.insert(keys[0].clone(), "1".to_string()).unwrap();
        trie.insert(keys[1].clone(), "2".to_string()).unwrap();
        trie.insert(keys[2].clone(), "3".to_string()).unwrap();

        let root_guard = trie.root.read().unwrap();
        let entries = match &*root_guard {
            Node::Root(root) => &root.entries,
            _ => panic!("expected root node"),
        };
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|entry| {
            matches!(
                &*entry.child.read().unwrap(),
                Node::Leaf(_) | Node::Extension(_)
            )
        }));
        assert!(entries
            .iter()
            .any(|entry| matches!(&*entry.child.read().unwrap(), Node::Extension(_))));

        let first_leaf = trie.first_leaf.as_ref().expect("first leaf").clone();
        let second_leaf = {
            let guard = first_leaf.read().unwrap();
            match &*guard {
                Node::Leaf(leaf) => leaf.next.clone().expect("second leaf"),
                _ => panic!("expected leaf"),
            }
        };
        let third_leaf = {
            let guard = second_leaf.read().unwrap();
            match &*guard {
                Node::Leaf(leaf) => leaf.next.clone().expect("third leaf"),
                _ => panic!("expected leaf"),
            }
        };

        let first_guard = first_leaf.read().unwrap();
        let second_guard = second_leaf.read().unwrap();
        let third_guard = third_leaf.read().unwrap();
        match (&*first_guard, &*second_guard, &*third_guard) {
            (Node::Leaf(first), Node::Leaf(second), Node::Leaf(third)) => {
                assert!(first.prev.is_none());
                assert!(second.prev.is_some());
                assert!(second.next.is_some());
                assert!(third.next.is_none());
            }
            _ => panic!("expected leaf sequence"),
        }
    }

    #[test]
    fn updating_existing_key_keeps_root_entry_child_stable() {
        let mut trie = AccTrie::new();
        let keys = byte_keys_sharing_root_prefix(2);
        trie.insert(keys[0].clone(), "1".to_string()).unwrap();
        trie.insert(keys[1].clone(), "2".to_string()).unwrap();

        let prefix = AccTrie::root_prefix_for_key(&keys[0]);
        let child_before = root_entry_child(&trie, &prefix);
        let acc_before = trie
            .root_entry_acc_for_key(&keys[0])
            .expect("root entry acc");

        trie.insert(keys[0].clone(), "9".to_string()).unwrap();

        let child_after = root_entry_child(&trie, &prefix);
        let acc_after = trie
            .root_entry_acc_for_key(&keys[0])
            .expect("root entry acc");
        assert!(Arc::ptr_eq(&child_before, &child_after));
        assert_ne!(acc_before, acc_after);
    }

    #[test]
    fn structural_insert_only_rebuilds_affected_root_entry() {
        let mut trie = AccTrie::new();
        let shared = byte_keys_sharing_root_prefix(3);
        let mut distinct = None;
        for byte in 0u16..=255 {
            let key = vec![byte as u8];
            if AccTrie::root_prefix_for_key(&key) != AccTrie::root_prefix_for_key(&shared[0]) {
                distinct = Some(key);
                break;
            }
        }
        let distinct = distinct.expect("key with different root prefix");

        trie.insert(shared[0].clone(), "1".to_string()).unwrap();
        trie.insert(shared[1].clone(), "2".to_string()).unwrap();
        trie.insert(distinct.clone(), "3".to_string()).unwrap();

        let shared_prefix = AccTrie::root_prefix_for_key(&shared[0]);
        let distinct_prefix = AccTrie::root_prefix_for_key(&distinct);
        let shared_child_before = root_entry_child(&trie, &shared_prefix);
        let distinct_child_before = root_entry_child(&trie, &distinct_prefix);

        trie.insert(shared[2].clone(), "4".to_string()).unwrap();

        let shared_child_after = root_entry_child(&trie, &shared_prefix);
        let distinct_child_after = root_entry_child(&trie, &distinct_prefix);
        assert!(!Arc::ptr_eq(&shared_child_before, &shared_child_after));
        assert!(Arc::ptr_eq(&distinct_child_before, &distinct_child_after));
    }

    #[test]
    fn query_missing_key_returns_verifiable_non_membership_proof() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), "1".to_string()).unwrap();
        trie.insert(b"gamma".to_vec(), "2".to_string()).unwrap();

        let result = trie.query(&b"theta".to_vec(), "").unwrap();
        match result {
            QueryResult::NotExists(proof) => {
                let expected = sorted_keys_by_hash(&[b"alpha", b"theta", b"gamma"]);
                assert_eq!(proof.key_prev, Some(b"alpha".to_vec()));
                assert_eq!(proof.key_next, Some(b"gamma".to_vec()));
                assert_eq!(expected[0], b"alpha".to_vec());
                assert_eq!(expected[1], b"theta".to_vec());
                assert_eq!(expected[2], b"gamma".to_vec());
                assert!(proof.prev_in_next_proof.is_some());
                assert!(proof.next_in_next_proof.is_some());
                assert!(proof.root_acc.is_some());
                assert!(proof.ln_next_acc_in_root_proof.is_some());
                assert!(proof.verify());
            }
            QueryResult::Exists(_) => panic!("expected non-membership proof"),
        }
    }

    #[test]
    fn query_existing_key_returns_membership_proof() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), "7".to_string()).unwrap();

        let result = trie.query(&b"alpha".to_vec(), "7").unwrap();
        match result {
            QueryResult::Exists(proof) => {
                let membership_proof = proof.membership_proof.clone().expect("membership proof");
                assert!(membership_proof.verify(proof.ln_acc));
                let count_membership_proof = proof
                    .count_membership_proof
                    .clone()
                    .expect("count membership proof");
                assert_eq!(proof.value_count, 1);
                assert!(count_membership_proof.verify(proof.ln_acc));
                assert!(proof.root_acc.is_some());
                assert!(proof.ln_acc_in_root_proof.is_some());
                assert!(proof.verify());
            }
            QueryResult::NotExists(_) => panic!("expected membership proof"),
        }
    }

    #[test]
    fn query_existing_key_proves_value_count() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), "7".to_string()).unwrap();
        trie.insert(b"alpha".to_vec(), "9".to_string()).unwrap();

        let result = trie.query(&b"alpha".to_vec(), "7").unwrap();
        match result {
            QueryResult::Exists(proof) => {
                assert_eq!(proof.value_count, 2);
                let count_membership_proof = proof
                    .count_membership_proof
                    .clone()
                    .expect("count membership proof");
                assert!(count_membership_proof.verify(proof.ln_acc));
                assert!(proof.verify());
            }
            QueryResult::NotExists(_) => panic!("expected membership proof"),
        }
    }

    #[test]
    fn insertion_and_deletion_update_neighbor_leaf_accumulators() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), "1".to_string()).unwrap();
        trie.insert(b"beta".to_vec(), "3".to_string()).unwrap();

        let insertion = trie.insert(b"gamma".to_vec(), "2".to_string()).unwrap();
        assert_eq!(insertion.key_prev, Some(b"alpha".to_vec()));
        assert_eq!(insertion.key_next, Some(b"beta".to_vec()));
        assert!(insertion.value_in_ln_proof.is_some());
        assert!(insertion.key_in_ln_next_new_proof.is_some());

        let beta_before_delete = trie.leaf_acc_for_key(b"beta").expect("beta leaf acc");
        let deletion = trie
            .delete(&b"gamma".to_vec(), Some("2".to_string()))
            .unwrap();
        assert!(deletion.delete_entire_leaf);
        assert!(deletion.value_in_ln_old_proof.is_some());
        assert_eq!(deletion.ln_next_acc_old, Some(beta_before_delete));
        assert_ne!(deletion.ln_next_acc_new, Some(beta_before_delete));
    }

    #[test]
    fn snapshot_tracks_root_entries() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), "1".to_string()).unwrap();
        trie.insert(b"beta".to_vec(), "2".to_string()).unwrap();

        let snapshot = trie.accumulator_snapshot();
        assert!(!snapshot.is_empty());

        let root_guard = trie.root.read().unwrap();
        let entries = match &*root_guard {
            Node::Root(root) => root.entries.len(),
            _ => 0,
        };
        assert_eq!(snapshot.len(), entries);
        assert!(snapshot.iter().all(|entry| !entry.is_empty()));
    }

    #[test]
    fn linked_leaves_expose_leaf_accumulator() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), "11".to_string()).unwrap();
        trie.insert(b"beta".to_vec(), "22".to_string()).unwrap();

        let first_leaf = trie.first_leaf.as_ref().expect("first leaf");
        let first_acc = leaf_acc(first_leaf);
        let first_key = sorted_keys_by_hash(&[b"alpha", b"beta"])[0].clone();
        assert_eq!(first_acc, trie.leaf_acc_for_key(&first_key).unwrap());
    }

    #[test]
    fn query_proof_uses_matching_root_entry_accumulator() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), "1".to_string()).unwrap();
        trie.insert(b"beta".to_vec(), "2".to_string()).unwrap();
        trie.insert(b"gamma".to_vec(), "3".to_string()).unwrap();

        let result = trie.query(&b"alpha".to_vec(), "1").unwrap();
        match result {
            QueryResult::Exists(proof) => {
                let expected_root_acc = trie
                    .root_entry_acc_for_key(b"alpha")
                    .expect("root entry acc for alpha");
                assert_eq!(proof.root_acc, Some(expected_root_acc));

                if let Some(global_root_acc) = trie.root_accumulator() {
                    let root_guard = trie.root.read().unwrap();
                    let entry_count = match &*root_guard {
                        Node::Root(root) => root.entries.len(),
                        _ => 0,
                    };
                    if entry_count > 1 {
                        assert_ne!(proof.root_acc, Some(global_root_acc));
                    }
                }
            }
            QueryResult::NotExists(_) => panic!("expected membership proof"),
        }
    }

    #[test]
    fn inserting_new_key_reuses_existing_leaf_nodes() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), "1".to_string()).unwrap();
        trie.insert(b"gamma".to_vec(), "3".to_string()).unwrap();

        let alpha_leaf = trie.leaf_ref_for_key(b"alpha").expect("alpha leaf");
        let gamma_leaf = trie.leaf_ref_for_key(b"gamma").expect("gamma leaf");

        trie.insert(b"beta".to_vec(), "2".to_string()).unwrap();

        let alpha_after = trie
            .leaf_ref_for_key(b"alpha")
            .expect("alpha leaf after insert");
        let gamma_after = trie
            .leaf_ref_for_key(b"gamma")
            .expect("gamma leaf after insert");

        assert!(Arc::ptr_eq(&alpha_leaf, &alpha_after));
        assert!(Arc::ptr_eq(&gamma_leaf, &gamma_after));
    }

    #[test]
    fn inserting_existing_key_updates_leaf_in_place() {
        let mut trie = AccTrie::new();
        trie.insert(b"alpha".to_vec(), "1".to_string()).unwrap();

        let alpha_leaf = trie.leaf_ref_for_key(b"alpha").expect("alpha leaf");
        let acc_before = trie.leaf_acc_for_key(b"alpha").expect("alpha acc before");

        trie.insert(b"alpha".to_vec(), "2".to_string()).unwrap();

        let alpha_after = trie
            .leaf_ref_for_key(b"alpha")
            .expect("alpha leaf after update");
        let acc_after = trie.leaf_acc_for_key(b"alpha").expect("alpha acc after");

        assert!(Arc::ptr_eq(&alpha_leaf, &alpha_after));
        assert_ne!(acc_before, acc_after);
    }
}
