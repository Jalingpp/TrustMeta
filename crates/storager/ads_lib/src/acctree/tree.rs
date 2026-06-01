use crate::acctree::acc_proof::{AccProof, MembershipProof, NonMembershipProof};
use crate::acctree::accumulator_ads::{DynamicAccumulator, G1Affine, MembershipProof as RawMembershipProof, Set, digest_set_from_set};
use crate::acctree::merkle_proof::MerkleProof;
use crate::acctree::node::Node;
use crate::acctree::result::{DeleteResult, InsertResult, SelectResult, SiblingNonMembershipProof, SingleSelectResult, TreeProof};
use crate::acctree::utils::Hash;
use ark_serialize::CanonicalSerialize;

pub struct AccumulatorTree {
    pub roots: Vec<Box<Node>>,
}

impl Default for AccumulatorTree {
    fn default() -> Self { Self::new() }
}

impl AccumulatorTree {
    pub fn new() -> Self { Self { roots: Vec::new() } }

    pub fn global_state_hash(&self) -> Hash {
        let mut out = [0u8; 32];
        out
    }
}
