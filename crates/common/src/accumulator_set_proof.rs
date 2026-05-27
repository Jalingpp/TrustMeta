use crate::{ProofVerifier, SetProofLeaf};
use accumulator_ads::{
    digest_set_from_set, DynamicAccumulator, G1Affine, IntersectionProof, Set, UnionProof,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub const ACCUMULATOR_SET_PROOF_MAGIC: &[u8] = b"ACCSI1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccumulatorSetOperationAggregateProof {
    pub expr: String,
    pub result_fids: Vec<String>,
    pub root: AccumulatorSetOperationProofNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccumulatorSetOperationProofNode {
    Leaf(AccumulatorSetOperationLeafProof),
    And(Box<AccumulatorIntersectionNodeProof>),
    Or(Box<AccumulatorUnionNodeProof>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccumulatorSetOperationLeafProof {
    pub leaf: SetProofLeaf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccumulatorIntersectionNodeProof {
    pub left: AccumulatorSetOperationProofNode,
    pub right: AccumulatorSetOperationProofNode,
    pub result_fids: Vec<String>,
    pub proof: IntersectionProof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccumulatorUnionNodeProof {
    pub left: AccumulatorSetOperationProofNode,
    pub right: AccumulatorSetOperationProofNode,
    pub result_fids: Vec<String>,
    pub proof: UnionProof,
}

pub fn encode_accumulator_set_operation_proof(
    proof: &AccumulatorSetOperationAggregateProof,
) -> Result<Vec<u8>, String> {
    let payload = bincode::serialize(proof)
        .map_err(|e| format!("serialize accumulator aggregate proof failed: {}", e))?;
    let mut bytes = Vec::with_capacity(ACCUMULATOR_SET_PROOF_MAGIC.len() + payload.len());
    bytes.extend_from_slice(ACCUMULATOR_SET_PROOF_MAGIC);
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

pub fn is_accumulator_set_operation_proof(proof: &[u8]) -> bool {
    proof.starts_with(ACCUMULATOR_SET_PROOF_MAGIC)
}

pub fn decode_accumulator_set_operation_proof(
    proof: &[u8],
) -> Result<AccumulatorSetOperationAggregateProof, String> {
    if !is_accumulator_set_operation_proof(proof) {
        return Err("invalid accumulator aggregate proof magic".to_string());
    }

    bincode::deserialize(&proof[ACCUMULATOR_SET_PROOF_MAGIC.len()..])
        .map_err(|e| format!("deserialize accumulator aggregate proof failed: {}", e))
}

pub fn accumulator_set_operation_root_hash(proof: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(proof);
    hasher.finalize().to_vec()
}

fn sorted_vec_from_set(values: &HashSet<String>) -> Vec<String> {
    let mut items: Vec<String> = values.iter().cloned().collect();
    items.sort();
    items
}

fn sorted_vec_from_slice(values: &[String]) -> Vec<String> {
    let mut items = values.to_vec();
    items.sort();
    items
}

fn accumulator_value_from_slice(values: &[String]) -> G1Affine {
    let set = Set::from_vec(values.to_vec());
    let digest_set = digest_set_from_set(&set);
    DynamicAccumulator::from_set(Default::default(), &digest_set).acc_value
}

fn accumulator_value_from_set(values: &HashSet<String>) -> G1Affine {
    accumulator_value_from_slice(&sorted_vec_from_set(values))
}

impl ProofVerifier {
    pub(crate) fn verify_accumulator_set_operation_aggregate(
        &self,
        proof: &[u8],
        root_hash: &[u8],
    ) -> bool {
        if accumulator_set_operation_root_hash(proof) != root_hash {
            return false;
        }

        let aggregate = match decode_accumulator_set_operation_proof(proof) {
            Ok(aggregate) => aggregate,
            Err(_) => return false,
        };

        match self.verify_accumulator_set_operation_node(&aggregate.root) {
            Some(result_set) => sorted_vec_from_set(&result_set) == aggregate.result_fids,
            None => false,
        }
    }

    fn verify_accumulator_set_operation_node(
        &self,
        node: &AccumulatorSetOperationProofNode,
    ) -> Option<HashSet<String>> {
        match node {
            AccumulatorSetOperationProofNode::Leaf(leaf) => leaf.leaf.verify_and_collect_fids(self),
            AccumulatorSetOperationProofNode::And(binary) => {
                let left_set = self.verify_accumulator_set_operation_node(&binary.left)?;
                let right_set = self.verify_accumulator_set_operation_node(&binary.right)?;
                let result_set: HashSet<String> =
                    left_set.intersection(&right_set).cloned().collect();

                if sorted_vec_from_set(&result_set) != sorted_vec_from_slice(&binary.result_fids) {
                    return None;
                }

                let left_acc = accumulator_value_from_set(&left_set);
                let right_acc = accumulator_value_from_set(&right_set);
                let result_acc = accumulator_value_from_slice(&binary.result_fids);

                if !binary.proof.verify(left_acc, right_acc, result_acc) {
                    return None;
                }

                Some(result_set)
            }
            AccumulatorSetOperationProofNode::Or(binary) => {
                let left_set = self.verify_accumulator_set_operation_node(&binary.left)?;
                let right_set = self.verify_accumulator_set_operation_node(&binary.right)?;
                let result_set: HashSet<String> = left_set.union(&right_set).cloned().collect();

                if sorted_vec_from_set(&result_set) != sorted_vec_from_slice(&binary.result_fids) {
                    return None;
                }

                let left_acc = accumulator_value_from_set(&left_set);
                let right_acc = accumulator_value_from_set(&right_set);
                let result_acc = accumulator_value_from_slice(&binary.result_fids);

                if !binary.proof.verify(left_acc, right_acc, result_acc) {
                    return None;
                }

                Some(result_set)
            }
        }
    }

    pub fn verify_accumulator_query_result_fids(&self, proof: &[u8], fids: &[String]) -> bool {
        match decode_accumulator_set_operation_proof(proof) {
            Ok(aggregate) => {
                sorted_vec_from_slice(&aggregate.result_fids) == sorted_vec_from_slice(fids)
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdsMode, ProofVerifier, SetProofLeaf};

    fn leaf(keyword: &str, fids: &[&str]) -> AccumulatorSetOperationProofNode {
        AccumulatorSetOperationProofNode::Leaf(AccumulatorSetOperationLeafProof {
            leaf: SetProofLeaf::new(
                keyword.to_string(),
                format!("node-{}", keyword),
                AdsMode::Mest,
                Vec::new(),
                Vec::new(),
                fids.iter().map(|fid| (*fid).to_string()).collect(),
            ),
        })
    }

    fn digest_set(values: &[&str]) -> Vec<accumulator_ads::Fr> {
        let set = Set::from_vec(values.iter().map(|value| (*value).to_string()).collect());
        digest_set_from_set(&set)
    }

    #[test]
    fn test_accumulator_intersection_proof_roundtrip_and_verify() {
        let left = leaf("A", &["f1", "f2", "f3"]);
        let right = leaf("B", &["f2", "f3", "f4"]);
        let (_, proof) = IntersectionProof::new(
            &digest_set(&["f1", "f2", "f3"]),
            &digest_set(&["f2", "f3", "f4"]),
            &digest_set(&["f2", "f3"]),
        )
        .unwrap();

        let aggregate = AccumulatorSetOperationAggregateProof {
            expr: "(A AND B)".to_string(),
            result_fids: vec!["f2".to_string(), "f3".to_string()],
            root: AccumulatorSetOperationProofNode::And(Box::new(
                AccumulatorIntersectionNodeProof {
                    left,
                    right,
                    result_fids: vec!["f2".to_string(), "f3".to_string()],
                    proof,
                },
            )),
        };

        let encoded = encode_accumulator_set_operation_proof(&aggregate).unwrap();
        let verifier = ProofVerifier::new(AdsMode::Mest);
        assert!(verifier.verify(&encoded, &accumulator_set_operation_root_hash(&encoded)));
        assert!(verifier.verify_query_result_fids(&encoded, &["f2".to_string(), "f3".to_string()]));
    }

    #[test]
    fn test_accumulator_union_proof_roundtrip_and_verify() {
        let left = leaf("A", &["f1", "f2", "f3"]);
        let right = leaf("B", &["f2", "f3", "f4"]);
        let (intersection_acc, intersection_proof) = IntersectionProof::new(
            &digest_set(&["f1", "f2", "f3"]),
            &digest_set(&["f2", "f3", "f4"]),
            &digest_set(&["f2", "f3"]),
        )
        .unwrap();
        let (_, proof) = UnionProof::new(
            &intersection_acc,
            intersection_proof,
            &digest_set(&["f1", "f2", "f3", "f4"]),
        )
        .unwrap();

        let aggregate = AccumulatorSetOperationAggregateProof {
            expr: "(A OR B)".to_string(),
            result_fids: vec![
                "f1".to_string(),
                "f2".to_string(),
                "f3".to_string(),
                "f4".to_string(),
            ],
            root: AccumulatorSetOperationProofNode::Or(Box::new(AccumulatorUnionNodeProof {
                left,
                right,
                result_fids: vec![
                    "f1".to_string(),
                    "f2".to_string(),
                    "f3".to_string(),
                    "f4".to_string(),
                ],
                proof,
            })),
        };

        let encoded = encode_accumulator_set_operation_proof(&aggregate).unwrap();
        let verifier = ProofVerifier::new(AdsMode::Mest);
        assert!(verifier.verify(&encoded, &accumulator_set_operation_root_hash(&encoded)));
        assert!(verifier.verify_query_result_fids(
            &encoded,
            &[
                "f1".to_string(),
                "f2".to_string(),
                "f3".to_string(),
                "f4".to_string(),
            ]
        ));
    }

    #[test]
    fn test_accumulator_leaf_verification_uses_leaf_ads_mode() {
        let aggregate = AccumulatorSetOperationAggregateProof {
            expr: "A".to_string(),
            result_fids: vec!["f1".to_string()],
            root: AccumulatorSetOperationProofNode::Leaf(AccumulatorSetOperationLeafProof {
                leaf: SetProofLeaf::new(
                    "A".to_string(),
                    "node-a".to_string(),
                    AdsMode::Mest,
                    vec![9u8; 32],
                    vec![9u8; 32],
                    vec!["f1".to_string()],
                ),
            }),
        };
        let encoded = encode_accumulator_set_operation_proof(&aggregate).unwrap();

        assert!(ProofVerifier::new(AdsMode::Mpt)
            .verify(&encoded, &accumulator_set_operation_root_hash(&encoded)));
    }
}
