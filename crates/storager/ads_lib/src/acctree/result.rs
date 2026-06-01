use crate::acctree::acc_proof::{AccProof, MembershipProof, NonMembershipProof};
use crate::acctree::accumulator_ads::Set;
use crate::acctree::accumulator_ads::acc::serde_impl;
use crate::acctree::merkle_proof::MerkleProof;
use crate::acctree::utils::{Hash, leaf_hash};
use serde::{Deserialize, Serialize};
use std::rc::Rc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiblingNonMembershipProof {
    pub sibling_hash: Hash,
    #[serde(with = serde_impl)]
    pub sibling_accumulator: crate::acctree::accumulator_ads::G1Affine,
    pub non_membership_proof: NonMembershipProof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathElement {
    pub sibling_hash: Hash,
    pub is_left_sibling: bool,
    pub sibling_keys: Rc<Set<String>>,
    #[serde(with = serde_impl)]
    pub sibling_acc: crate::acctree::accumulator_ads::G1Affine,
    pub parent_keys: Rc<Set<String>>,
    #[serde(with = serde_impl)]
    pub parent_acc: crate::acctree::accumulator_ads::G1Affine,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeProof {
    pub tree_root_hash: Hash,
    pub leaf_merkle_proof: MerkleProof,
    pub root_membership_proof: MembershipProof,
    pub sibling_proofs: Vec<SiblingNonMembershipProof>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleSelectResult {
    pub fid: String,
    pub tree_proof: TreeProof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectResult {
    pub results: Vec<SingleSelectResult>,
    pub key: String,
}

impl SelectResult {
    pub fn new(results: Vec<SingleSelectResult>, key: String) -> Self {
        Self { results, key }
    }

    pub fn fids(&self) -> Vec<String> {
        self.results.iter().map(|result| result.fid.clone()).collect()
    }

    pub fn verify(&self, expected_key: &str) -> bool {
        if self.key != expected_key {
            return false;
        }

        self.results.iter().all(|result| {
            let leaf = leaf_hash(expected_key, &result.fid);
            result.tree_proof.leaf_merkle_proof.verify(leaf)
                && result
                    .tree_proof
                    .root_membership_proof
                    .verify(&result.tree_proof.root_membership_proof.accumulator, expected_key)
                && result.tree_proof.sibling_proofs.iter().all(|proof| {
                    proof
                        .non_membership_proof
                        .verify(&proof.sibling_accumulator, expected_key)
                })
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeMatchResult {
    pub fid: String,
    pub path: Vec<PathElement>,
    pub tree_root_hash: Hash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsertResult {
    pub fid: String,
    pub post_merkle_proof: MerkleProof,
    pub post_acc_proof: MembershipProof,
    pub pre_acc_proof: Option<NonMembershipProof>,
}

impl InsertResult {
    pub fn new(
        fid: String,
        post_merkle_proof: MerkleProof,
        post_acc_proof: MembershipProof,
        pre_acc_proof: Option<NonMembershipProof>,
    ) -> Self {
        Self {
            fid,
            post_merkle_proof,
            post_acc_proof,
            pre_acc_proof,
        }
    }

    pub fn verify_insert(&self, expected_key: &str, expected_fid: &str) -> bool {
        if self.fid != expected_fid {
            return false;
        }

        if let Some(non_membership_proof) = &self.pre_acc_proof {
            if !non_membership_proof.verify(&non_membership_proof.accumulator, expected_key) {
                return false;
            }
        }

        self.post_merkle_proof.verify(leaf_hash(expected_key, &self.fid))
            && self
                .post_acc_proof
                .verify(&self.post_acc_proof.accumulator, expected_key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResult {
    pub deleted_fid: String,
    pub old_fid: Option<String>,
    pub new_fid: Option<String>,
    pub pre_merkle_proof: Option<MerkleProof>,
    pub pre_acc_proof: Option<MembershipProof>,
    pub post_merkle_proof: Option<MerkleProof>,
    pub post_acc_proof: Option<AccProof>,
}

impl DeleteResult {
    pub fn new(
        deleted_fid: String,
        old_fid: Option<String>,
        new_fid: Option<String>,
        pre_merkle_proof: Option<MerkleProof>,
        pre_acc_proof: Option<MembershipProof>,
        post_merkle_proof: Option<MerkleProof>,
        post_acc_proof: Option<AccProof>,
    ) -> Self {
        Self {
            deleted_fid,
            old_fid,
            new_fid,
            pre_merkle_proof,
            pre_acc_proof,
            post_merkle_proof,
            post_acc_proof,
        }
    }

    pub fn verify_delete(&self, expected_key: &str) -> bool {
        match &self.old_fid {
            Some(old_fid) if old_fid == &self.deleted_fid => {}
            _ => return false,
        }

        if let Some(pre_merkle) = &self.pre_merkle_proof {
            let Some(old_fid) = &self.old_fid else {
                return false;
            };
            if !pre_merkle.verify(leaf_hash(expected_key, old_fid)) {
                return false;
            }
        }

        if let Some(post_merkle) = &self.post_merkle_proof {
            let Some(new_fid) = &self.new_fid else {
                return false;
            };
            if !post_merkle.verify(leaf_hash(expected_key, new_fid)) {
                return false;
            }
        } else if self.new_fid.is_some() {
            return false;
        }

        if let Some(pre_acc) = &self.pre_acc_proof {
            if !pre_acc.verify(&pre_acc.accumulator, expected_key) {
                return false;
            }
        }

        match &self.post_acc_proof {
            Some(AccProof::Membership(proof)) => {
                self.new_fid.is_some() && proof.verify(&proof.accumulator, expected_key)
            }
            Some(AccProof::NonMembership(proof)) => {
                self.new_fid.is_none() && proof.verify(&proof.accumulator, expected_key)
            }
            None => self.new_fid.is_none(),
        }
    }
}
