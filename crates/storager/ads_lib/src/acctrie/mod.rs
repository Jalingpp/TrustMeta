use ark_serialize::{CanonicalDeserialize, CanonicalSerialize, SerializationError};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// Re-export the inner `acc` crate so other workspace crates can access it
pub use acc;

// Use real accumulator types from `acc` implementation
use acc::acc_mod::{Acc, Fr, G1Affine};
use acc::set::MultiSet;
use acc::Accumulator;

// -----------------------------
// Unified ADS adapter for AccTrie
// -----------------------------
use crate::unified_ads::{AuthenticatedDataStructure, UnifiedKey, UnifiedValue};
use anyhow::{anyhow, Result};

#[derive(Clone, Debug)]
pub struct AccTrieProof(pub Vec<u8>);

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
            UnifiedValue::Integer(v) => v,
            UnifiedValue::String(s) => s.parse::<i64>().map_err(|e| anyhow!("parse int: {}", e))?,
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
                // return the first value as the canonical value
                return Ok(Some((
                    UnifiedValue::Integer(vec[0]),
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
        // delete entire leaf
        match self.trie.write().unwrap().delete(&keyv, None) {
            Ok(_) => Ok(None),
            Err(_) => Ok(None),
        }
    }

    fn verify(&self, _proof: &Self::Proof) -> bool {
        // lightweight stub: real verification would inspect accumulated bytes
        true
    }

    fn ads_type(&self) -> &'static str {
        "AccTrie"
    }

    fn estimate_proof_size(_proof: &Self::Proof) -> usize {
        0
    }
}

// Use the dynamic membership proof types from acc crate
pub use acc::dynamic_accumulator::MembershipProof;

// Node and Leaf types for iteration used by acctrie_ads
#[derive(Debug)]
pub struct Leaf {
    pub acc: Option<G1Affine>,
    pub next: Option<Arc<RwLock<Node>>>,
}

#[derive(Debug)]
pub enum Node {
    Leaf(Leaf),
    Internal,
}

// Proof structs expected by acctrie_ads.rs
#[derive(Clone, Debug)]
pub struct InsertionProof {
    pub key: Vec<u8>,
    pub value: i64,
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
    pub value: Option<i64>,
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
    pub value: i64,
    pub ln_acc: G1Affine,
    pub membership_proof: Option<MembershipProof>,
}

#[derive(Clone, Debug)]
pub struct QueryNotExistsProof {
    pub key: Vec<u8>,
    pub key_prev: Option<Vec<u8>>,
    pub key_next: Option<Vec<u8>>,
    pub ln_next_acc: Option<G1Affine>,
    pub prev_in_next_proof: Option<MembershipProof>,
}

#[derive(Clone, Debug)]
pub enum QueryResult {
    Exists(QueryExistsProof),
    NotExists(QueryNotExistsProof),
}

// Simple in-memory AccTrie used for tests: maps key -> Vec<i64>
#[derive(Debug)]
pub struct AccTrie {
    pub map: HashMap<Vec<u8>, Vec<i64>>,
    pub first_leaf: Option<Arc<RwLock<Node>>>,
}

impl AccTrie {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            first_leaf: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    // Insert returns a synthetic InsertionProof
    pub fn insert(&mut self, key: Vec<u8>, value: i64) -> Result<InsertionProof, String> {
        let entry = self.map.entry(key.clone()).or_default();
        // compute old accumulator over current elements
        let old_set = MultiSet::from_vec(entry.clone());
        let ln_acc_old = Acc::cal_acc_g1(&old_set);
        entry.push(value);
        // compute new accumulator after insertion
        let new_set = MultiSet::from_vec(entry.clone());
        let ln_acc_new = Acc::cal_acc_g1(&new_set);

        // maintain a simple linked list of leaves via first_leaf if absent
        if self.first_leaf.is_none() {
            let leaf = Leaf {
                acc: Some(ln_acc_new.clone()),
                next: None,
            };
            self.first_leaf = Some(Arc::new(RwLock::new(Node::Leaf(leaf))));
        }

        Ok(InsertionProof {
            key,
            value,
            key_prev: None,
            key_next: None,
            ln_acc_old,
            ln_acc_new,
            ln_prev_acc: None,
            ln_next_acc_old: None,
            ln_next_acc_new: None,
            keyp_in_ln_next_old_proof: None,
            keyp_in_ln_proof: None,
            no_prev_in_ln_proof: None,
            key_in_ln_next_new_proof: None,
            keyp_in_ln_next_new_proof: None,
            value_in_ln_proof: None,
        })
    }

    pub fn delete(&mut self, key: &Vec<u8>, value: Option<i64>) -> Result<DeletionProof, String> {
        let existed = self.map.get_mut(key);
        let old_set = if let Some(v) = existed.as_ref() {
            MultiSet::from_vec((*v).clone())
        } else {
            MultiSet::new()
        };
        let ln_acc_old = Acc::cal_acc_g1(&old_set);
        if let Some(vec) = existed {
            if let Some(v) = value {
                vec.retain(|x| *x != v);
                let ln_acc_new = if vec.is_empty() {
                    None
                } else {
                    Some(Acc::cal_acc_g1(&MultiSet::from_vec(vec.clone())))
                };
                return Ok(DeletionProof {
                    key: key.clone(),
                    delete_entire_leaf: vec.is_empty(),
                    value: value,
                    key_prev: None,
                    key_next: None,
                    ln_acc_old,
                    ln_acc_new,
                    ln_next_acc_old: None,
                    ln_next_acc_new: None,
                    value_in_ln_old_proof: None,
                    keyp_in_ln_proof: None,
                    key_in_ln_next_old_proof: None,
                    keyp_in_ln_next_new_proof: None,
                });
            } else {
                // remove entire key
                self.map.remove(key);
                return Ok(DeletionProof {
                    key: key.clone(),
                    delete_entire_leaf: true,
                    value: None,
                    key_prev: None,
                    key_next: None,
                    ln_acc_old,
                    ln_acc_new: None,
                    ln_next_acc_old: None,
                    ln_next_acc_new: None,
                    value_in_ln_old_proof: None,
                    keyp_in_ln_proof: None,
                    key_in_ln_next_old_proof: None,
                    keyp_in_ln_next_new_proof: None,
                });
            }
        }
        Err("key not found".to_string())
    }

    pub fn query(&self, key: &Vec<u8>, value: i64) -> Result<QueryResult, String> {
        if let Some(vec) = self.map.get(key) {
            if vec.contains(&value) {
                let set = MultiSet::from_vec(vec.clone());
                let ln_acc = Acc::cal_acc_g1(&set);
                Ok(QueryResult::Exists(QueryExistsProof {
                    key: key.clone(),
                    value,
                    ln_acc,
                    membership_proof: None,
                }))
            } else {
                Ok(QueryResult::NotExists(QueryNotExistsProof {
                    key: key.clone(),
                    key_prev: None,
                    key_next: None,
                    ln_next_acc: None,
                    prev_in_next_proof: None,
                }))
            }
        } else {
            Ok(QueryResult::NotExists(QueryNotExistsProof {
                key: key.clone(),
                key_prev: None,
                key_next: None,
                ln_next_acc: None,
                prev_in_next_proof: None,
            }))
        }
    }
}
