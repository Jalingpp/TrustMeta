use accumulator_tree::{DeleteResult, InsertResult, SelectResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ACCTREE_PROOF_MAGIC: &[u8] = b"ACTR1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccTreeProofKind {
    Add {
        keyword: String,
        fid: String,
        result: InsertResult,
    },
    Query {
        keyword: String,
        result: SelectResult,
    },
    Delete {
        keyword: String,
        fid: String,
        result: DeleteResult,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccTreeProofEnvelope {
    pub root_hash: Vec<u8>,
    pub proof: AccTreeProofKind,
}

pub fn encode_acctree_proof(envelope: &AccTreeProofEnvelope) -> Result<Vec<u8>, String> {
    let payload = bincode::serialize(envelope)
        .map_err(|e| format!("serialize acctree proof failed: {}", e))?;
    let mut bytes = Vec::with_capacity(ACCTREE_PROOF_MAGIC.len() + payload.len());
    bytes.extend_from_slice(ACCTREE_PROOF_MAGIC);
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

pub fn is_acctree_proof(proof: &[u8]) -> bool {
    proof.starts_with(ACCTREE_PROOF_MAGIC)
}

pub fn decode_acctree_proof(proof: &[u8]) -> Result<AccTreeProofEnvelope, String> {
    if !is_acctree_proof(proof) {
        return Err("invalid acctree proof magic".to_string());
    }
    bincode::deserialize(&proof[ACCTREE_PROOF_MAGIC.len()..])
        .map_err(|e| format!("deserialize acctree proof failed: {}", e))
}

pub fn acctree_proof_root_hash(proof: &[u8]) -> Vec<u8> {
    match decode_acctree_proof(proof) {
        Ok(envelope) => envelope.root_hash,
        Err(_) => Vec::new(),
    }
}

pub fn acctree_proof_fids(proof: &[u8]) -> Option<Vec<String>> {
    match decode_acctree_proof(proof).ok()?.proof {
        AccTreeProofKind::Query { result, .. } => Some(result.fids()),
        AccTreeProofKind::Delete { result, .. } => Some(match result.new_fid {
            Some(fid) => vec![fid],
            None => Vec::new(),
        }),
        AccTreeProofKind::Add { fid, .. } => Some(vec![fid]),
    }
}

pub fn acctree_proof_digest(proof: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(proof);
    hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdsMode, ProofVerifier};
    use accumulator_ads::acc::setup::PRI_S;
    use accumulator_ads::acc::{init_public_parameters_direct, PublicParameters};
    use accumulator_tree::AccumulatorTree;
    use std::sync::OnceLock;

    static ACCTREE_TEST_PARAMS_INIT: OnceLock<()> = OnceLock::new();

    fn init_acctree_params() {
        ACCTREE_TEST_PARAMS_INIT.get_or_init(|| {
            let params = PublicParameters::generate_for_testing(*PRI_S, 64);
            let _ = init_public_parameters_direct(params);
        });
    }

    fn bytes_to_string(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn test_acctree_query_proof_roundtrip_and_tamper_detection() {
        init_acctree_params();

        let alpha = bytes_to_string(&[97, 108, 112, 104, 97]);
        let beta = bytes_to_string(&[98, 101, 116, 97]);
        let file_1 = bytes_to_string(&[102, 105, 108, 101, 45, 49]);
        let file_2 = bytes_to_string(&[102, 105, 108, 101, 45, 50]);
        let file_3 = bytes_to_string(&[102, 105, 108, 101, 45, 51]);
        let file_x = bytes_to_string(&[102, 105, 108, 101, 45, 120]);

        let mut tree = AccumulatorTree::new();
        tree.insert_with_proof(alpha.clone(), file_1.clone());
        tree.insert_with_proof(alpha.clone(), file_2.clone());
        tree.insert_with_proof(beta, file_3);

        let result = tree.select_all_with_proof(&alpha);
        let expected_fids = result.fids();
        let root_hash = tree.global_state_hash().to_vec();
        let proof = encode_acctree_proof(&AccTreeProofEnvelope {
            root_hash: root_hash.clone(),
            proof: AccTreeProofKind::Query {
                keyword: alpha.clone(),
                result: result.clone(),
            },
        })
        .unwrap();

        assert!(is_acctree_proof(&proof));
        assert_eq!(acctree_proof_root_hash(&proof), root_hash);
        assert_eq!(acctree_proof_fids(&proof), Some(expected_fids.clone()));

        let decoded = decode_acctree_proof(&proof).unwrap();
        match decoded.proof {
            AccTreeProofKind::Query { keyword, result } => {
                assert_eq!(keyword, alpha);
                assert_eq!(result.results.len(), 2);
                for entry in result.results {
                    assert_eq!(
                        entry.tree_proof.sibling_proofs.len(),
                        entry.tree_proof.leaf_merkle_proof.path.len()
                    );
                    assert_eq!(
                        entry.tree_proof.tree_root_hash,
                        entry.tree_proof.leaf_merkle_proof.root_hash
                    );
                }
            }
            _ => panic!(),
        }

        let verifier = ProofVerifier::new(AdsMode::AccTree);
        assert!(verifier.verify(&proof, &tree.global_state_hash().to_vec()));
        assert!(verifier.verify_query_result_fids(&proof, &expected_fids));

        let mut tampered = decode_acctree_proof(&proof).unwrap();
        match &mut tampered.proof {
            AccTreeProofKind::Query { result, .. } => {
                result.results[0].fid = file_x;
            }
            _ => unreachable!(),
        }

        let tampered_proof = encode_acctree_proof(&tampered).unwrap();
        assert!(!verifier.verify(&tampered_proof, &tree.global_state_hash().to_vec()));
        assert!(!verifier.verify_query_result_fids(&tampered_proof, &expected_fids));
    }
}
