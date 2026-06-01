use crate::acctree::result::PathElement;
use crate::acctree::{Hash, nonleaf_hash};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    pub root_hash: Hash,
    pub path: Vec<PathElement>,
}

impl MerkleProof {
    pub fn new(root_hash: Hash, path: Vec<PathElement>) -> Self {
        Self { root_hash, path }
    }

    pub fn verify(&self, leaf_hash: Hash) -> bool {
        let mut current = leaf_hash;
        for element in &self.path {
            current = if element.is_left_sibling {
                nonleaf_hash(
                    element.sibling_hash,
                    current,
                    &element.parent_keys,
                    &element.parent_acc,
                )
            } else {
                nonleaf_hash(
                    current,
                    element.sibling_hash,
                    &element.parent_keys,
                    &element.parent_acc,
                )
            };
        }
        current == self.root_hash
    }
}
