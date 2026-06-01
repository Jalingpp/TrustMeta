use crate::result::PathElement;
use crate::{Hash, nonleaf_hash};
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
        let mut cur = leaf_hash;
        for elem in &self.path {
            if elem.is_left_sibling {
                cur = nonleaf_hash(elem.sibling_hash, cur, &elem.parent_keys, &elem.parent_acc);
            } else {
                cur = nonleaf_hash(cur, elem.sibling_hash, &elem.parent_keys, &elem.parent_acc);
            }
        }
        cur == self.root_hash
    }
}
