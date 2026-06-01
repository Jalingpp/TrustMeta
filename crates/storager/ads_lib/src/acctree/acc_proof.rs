use crate::acctree::accumulator_ads::acc::serde_impl;
use crate::acctree::accumulator_ads::acc::utils::digest_to_prime_field;
use crate::acctree::accumulator_ads::digest::Digestible;
use crate::acctree::accumulator_ads::{
    G1Affine, MembershipProof as RawMembershipProof, NonMembershipProof as RawNonMembershipProof,
    Set, digest_set_from_set,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccProof {
    Membership(MembershipProof),
    NonMembership(NonMembershipProof),
}

impl AccProof {
    pub fn verify(&self, current_acc: &G1Affine, expected_key: &str) -> bool {
        match self {
            Self::Membership(proof) => proof.verify(current_acc, expected_key),
            Self::NonMembership(proof) => proof.verify(current_acc, expected_key),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipProof {
    pub key: String,
    #[serde(with = serde_impl)]
    pub accumulator: G1Affine,
    pub witness: RawMembershipProof,
}

impl MembershipProof {
    pub fn verify(&self, current_acc: &G1Affine, expected_key: &str) -> bool {
        self.key == expected_key
            && self.accumulator == *current_acc
            && self.witness.verify(*current_acc)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonMembershipProof {
    pub key: String,
    #[serde(with = serde_impl)]
    pub accumulator: G1Affine,
    pub witness: RawNonMembershipProof,
}

impl NonMembershipProof {
    pub fn new(key: String, accumulator: G1Affine, all_keys_set: &Set<String>) -> Option<Self> {
        let element = digest_to_prime_field(&key.to_digest());
        let digest_set = digest_set_from_set(all_keys_set);

        RawNonMembershipProof::new(element, &digest_set)
            .ok()
            .map(|witness| Self {
                key,
                accumulator,
                witness,
            })
    }

    pub fn verify(&self, current_acc: &G1Affine, expected_key: &str) -> bool {
        self.key == expected_key
            && self.accumulator == *current_acc
            && self.witness.verify(*current_acc)
    }
}
