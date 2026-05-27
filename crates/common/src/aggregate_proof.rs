use crate::accumulator_set_proof::{
    decode_accumulator_set_operation_proof, is_accumulator_set_operation_proof,
};
use crate::poly_set_proof::{
    decode_polynomial_intersection_proof, is_polynomial_intersection_proof,
};
use crate::{acctree_proof_fids, is_acctree_proof, ProofVerifier, SetProofLeaf};
use ads_rust::acctrie::acc::{Acc, AccProof, Accumulator, MultiSet};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub const BOOLEAN_QUERY_PROOF_MAGIC: &[u8] = b"BOOLP1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BooleanQueryAggregateProof {
    pub expr: String,
    pub result_fids: Vec<String>,
    pub node_root_hashes: HashMap<String, Vec<u8>>,
    pub tree: BooleanProofNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BooleanProofNode {
    Leaf(LeafKeywordProof),
    And(Box<BooleanBinaryProof>),
    Or(Box<BooleanBinaryProof>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeafKeywordProof {
    pub leaf: SetProofLeaf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BooleanBinaryProof {
    pub left: BooleanProofNode,
    pub right: BooleanProofNode,
    pub decomposition: SetOperationAggregateProof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetOperationAggregateProof {
    pub intersection_fids: Vec<String>,
    pub left_only_fids: Vec<String>,
    pub right_only_fids: Vec<String>,
    pub intersection_left_only_proof: Vec<u8>,
    pub intersection_right_only_proof: Vec<u8>,
    pub left_only_right_only_proof: Vec<u8>,
}

pub fn encode_boolean_query_aggregate_proof(
    proof: &BooleanQueryAggregateProof,
) -> Result<Vec<u8>, String> {
    let payload = bincode::serialize(proof)
        .map_err(|e| format!("serialize aggregate proof failed: {}", e))?;
    let mut bytes = Vec::with_capacity(BOOLEAN_QUERY_PROOF_MAGIC.len() + payload.len());
    bytes.extend_from_slice(BOOLEAN_QUERY_PROOF_MAGIC);
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

pub fn is_boolean_query_aggregate_proof(proof: &[u8]) -> bool {
    proof.starts_with(BOOLEAN_QUERY_PROOF_MAGIC)
}

pub fn decode_boolean_query_aggregate_proof(
    proof: &[u8],
) -> Result<BooleanQueryAggregateProof, String> {
    if !is_boolean_query_aggregate_proof(proof) {
        return Err("invalid boolean aggregate proof magic".to_string());
    }

    bincode::deserialize(&proof[BOOLEAN_QUERY_PROOF_MAGIC.len()..])
        .map_err(|e| format!("deserialize aggregate proof failed: {}", e))
}

pub fn boolean_query_aggregate_root_hash(proof: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(proof);
    hasher.finalize().to_vec()
}

fn sorted_vec_from_set(set: &HashSet<String>) -> Vec<String> {
    let mut items: Vec<String> = set.iter().cloned().collect();
    items.sort();
    items
}

fn sorted_vec_from_slice(values: &[String]) -> Vec<String> {
    let mut items = values.to_vec();
    items.sort();
    items
}

fn set_from_slice(values: &[String]) -> HashSet<String> {
    values.iter().cloned().collect()
}

fn verify_disjointness_proof(left: &[String], right: &[String], proof_bytes: &[u8]) -> bool {
    if left.is_empty() || right.is_empty() {
        return proof_bytes.is_empty();
    }

    let proof: AccProof = match bincode::deserialize(proof_bytes) {
        Ok(proof) => proof,
        Err(_) => return false,
    };

    let left_set = MultiSet::from_vec(left.to_vec());
    let right_set = MultiSet::from_vec(right.to_vec());
    let left_acc = Acc::cal_acc_g1(&left_set);
    let right_acc = Acc::cal_acc_g2(&right_set);
    proof.verify(&left_acc, &right_acc)
}

fn verify_pairwise_disjoint(op: &SetOperationAggregateProof) -> bool {
    verify_disjointness_proof(
        &op.intersection_fids,
        &op.left_only_fids,
        &op.intersection_left_only_proof,
    ) && verify_disjointness_proof(
        &op.intersection_fids,
        &op.right_only_fids,
        &op.intersection_right_only_proof,
    ) && verify_disjointness_proof(
        &op.left_only_fids,
        &op.right_only_fids,
        &op.left_only_right_only_proof,
    )
}

impl ProofVerifier {
    pub fn verify_query_result_fids(&self, proof: &[u8], fids: &[String]) -> bool {
        if is_accumulator_set_operation_proof(proof) {
            return match decode_accumulator_set_operation_proof(proof) {
                Ok(aggregate) => {
                    sorted_vec_from_slice(&aggregate.result_fids) == sorted_vec_from_slice(fids)
                }
                Err(_) => false,
            };
        }

        if is_polynomial_intersection_proof(proof) {
            return match decode_polynomial_intersection_proof(proof) {
                Ok(aggregate) => {
                    sorted_vec_from_slice(&aggregate.result_fids) == sorted_vec_from_slice(fids)
                }
                Err(_) => false,
            };
        }

        if is_acctree_proof(proof) {
            return match acctree_proof_fids(proof) {
                Some(proof_fids) => {
                    sorted_vec_from_slice(&proof_fids) == sorted_vec_from_slice(fids)
                }
                None => false,
            };
        }

        if !is_boolean_query_aggregate_proof(proof) {
            return true;
        }

        match decode_boolean_query_aggregate_proof(proof) {
            Ok(aggregate) => {
                sorted_vec_from_slice(&aggregate.result_fids) == sorted_vec_from_slice(fids)
            }
            Err(_) => false,
        }
    }

    pub(crate) fn verify_boolean_query_aggregate(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if boolean_query_aggregate_root_hash(proof) != root_hash {
            return false;
        }

        let aggregate = match decode_boolean_query_aggregate_proof(proof) {
            Ok(aggregate) => aggregate,
            Err(_) => return false,
        };

        match self.verify_boolean_node(&aggregate.tree, &aggregate.node_root_hashes) {
            Some(result_set) => {
                let expected = sorted_vec_from_set(&result_set);
                expected == aggregate.result_fids
            }
            None => false,
        }
    }

    fn verify_boolean_node(
        &self,
        node: &BooleanProofNode,
        node_root_hashes: &HashMap<String, Vec<u8>>,
    ) -> Option<HashSet<String>> {
        match node {
            BooleanProofNode::Leaf(leaf) => {
                let expected_root = node_root_hashes.get(&leaf.leaf.node_name)?;
                if expected_root != &leaf.leaf.root_hash {
                    return None;
                }
                leaf.leaf.verify_and_collect_fids(self)
            }
            BooleanProofNode::And(binary) => {
                let left_set = self.verify_boolean_node(&binary.left, node_root_hashes)?;
                let right_set = self.verify_boolean_node(&binary.right, node_root_hashes)?;

                if !verify_pairwise_disjoint(&binary.decomposition) {
                    return None;
                }

                let intersection = set_from_slice(&binary.decomposition.intersection_fids);
                let left_only = set_from_slice(&binary.decomposition.left_only_fids);
                let right_only = set_from_slice(&binary.decomposition.right_only_fids);

                let expected_left: HashSet<String> =
                    intersection.union(&left_only).cloned().collect();
                let expected_right: HashSet<String> =
                    intersection.union(&right_only).cloned().collect();

                if expected_left != left_set || expected_right != right_set {
                    return None;
                }

                Some(intersection)
            }
            BooleanProofNode::Or(binary) => {
                let left_set = self.verify_boolean_node(&binary.left, node_root_hashes)?;
                let right_set = self.verify_boolean_node(&binary.right, node_root_hashes)?;

                if !verify_pairwise_disjoint(&binary.decomposition) {
                    return None;
                }

                let intersection = set_from_slice(&binary.decomposition.intersection_fids);
                let left_only = set_from_slice(&binary.decomposition.left_only_fids);
                let right_only = set_from_slice(&binary.decomposition.right_only_fids);

                let expected_left: HashSet<String> =
                    intersection.union(&left_only).cloned().collect();
                let expected_right: HashSet<String> =
                    intersection.union(&right_only).cloned().collect();

                if expected_left != left_set || expected_right != right_set {
                    return None;
                }

                let result: HashSet<String> = expected_left.union(&right_only).cloned().collect();
                Some(result)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdsMode, ProofVerifier};
    use std::collections::HashMap;

    #[test]
    fn test_boolean_leaf_verification_uses_leaf_ads_mode() {
        let leaf = SetProofLeaf::new(
            "A".to_string(),
            "node-a".to_string(),
            AdsMode::Mest,
            vec![5u8; 32],
            vec![5u8; 32],
            vec!["f1".to_string()],
        );
        let mut node_root_hashes = HashMap::new();
        node_root_hashes.insert("node-a".to_string(), vec![5u8; 32]);

        let aggregate = BooleanQueryAggregateProof {
            expr: "A".to_string(),
            result_fids: vec!["f1".to_string()],
            node_root_hashes,
            tree: BooleanProofNode::Leaf(LeafKeywordProof { leaf }),
        };
        let encoded = encode_boolean_query_aggregate_proof(&aggregate).unwrap();

        assert!(ProofVerifier::new(AdsMode::Mpt)
            .verify(&encoded, &boolean_query_aggregate_root_hash(&encoded)));
    }
}
