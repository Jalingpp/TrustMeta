use accumulator_ads::acc::serde_impl;
use accumulator_ads::acc::utils::digest_to_prime_field;
use accumulator_ads::digest::Digestible;
use accumulator_ads::{G1Affine, Set, digest_set_from_set};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccProof {
    Membership(MembershipProof),
    NonMembership(NonMembershipProof),
}

impl AccProof {
    pub fn new_membership(proof: MembershipProof) -> Self {
        Self::Membership(proof)
    }

    pub fn new_non_membership(proof: NonMembershipProof) -> Self {
        Self::NonMembership(proof)
    }

    pub fn verify(&self, current_acc: &G1Affine, expected_key: &str) -> bool {
        match self {
            Self::Membership(p) => p.verify(current_acc, expected_key),
            Self::NonMembership(p) => p.verify(current_acc, expected_key),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipProof {
    pub key: String,
    #[serde(with = "serde_impl")]
    pub accumulator: G1Affine,
    pub witness: accumulator_ads::MembershipProof,
}

impl MembershipProof {
    pub fn new(
        key: String,
        accumulator: G1Affine,
        witness: accumulator_ads::MembershipProof,
    ) -> Self {
        Self {
            key,
            accumulator,
            witness,
        }
    }

    pub fn verify(&self, current_acc: &G1Affine, expected_key: &str) -> bool {
        self.key == expected_key
            && self.accumulator == *current_acc
            && self.witness.verify(*current_acc)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonMembershipProof {
    pub key: String,
    #[serde(with = "serde_impl")]
    pub accumulator: G1Affine,
    pub witness: accumulator_ads::NonMembershipProof,
}

impl NonMembershipProof {
    pub fn new(key: String, accumulator: G1Affine, all_keys_set: &Set<String>) -> Option<Self> {
        let element = digest_to_prime_field(&key.to_digest());
        let digest_set = digest_set_from_set(all_keys_set);

        accumulator_ads::NonMembershipProof::new(element, &digest_set)
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
