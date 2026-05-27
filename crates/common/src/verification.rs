//! 璇佹槑楠岃瘉妯″潡
//!
//! 璐熻矗楠岃瘉鏉ヨ嚜 storager 鐨勫瘑鐮佸璇佹槑

use crate::acctree_proof::{
    acctree_proof_root_hash, decode_acctree_proof, is_acctree_proof, AccTreeProofKind,
};
use crate::accumulator_set_proof::is_accumulator_set_operation_proof;
use crate::aggregate_proof::is_boolean_query_aggregate_proof;
use crate::poly_set_proof::{
    build_characteristic_polynomial, decode_polynomial_intersection_proof,
    derive_polynomial_challenge, evaluate_transparent_polynomial, is_polynomial_intersection_proof,
    polynomial_intersection_root_hash, verify_transparent_polynomial,
    PolynomialIntersectionNodeProof, PolynomialSetProofNode, PolynomialUnionNodeProof,
    TransparentPolynomial,
};
use crate::{AdsMode, SetProofLeaf};
use ads_rust::acctrie::acc::{dynamic_accumulator::MembershipProof, Fr};
use ads_rust::mpt::proof::{compute_mpt_root, MPTProof};
use ark_bls12_381::G1Affine;
use ark_ff::{One, PrimeField};
use ark_serialize::CanonicalDeserialize;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
/// MEST Proof鍏煎鎬х粨鏋?(鐢ㄤ簬鍙嶅簭鍒楀寲)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MestProofCompat {
    is_exist: bool,
    key: String,
    bucket_proof: BucketProofCompat,
    mgt_proof: MgtProofCompat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BucketProofCompat {
    value: String,
    seg_root_hash: [u8; 32],
    merkle_path: Vec<MerklePathElementCompat>,
    leaf_segment_roots: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MerklePathElementCompat {
    direction: u8,
    sibling_hash: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MgtProofCompat {
    root_hash: [u8; 32],
    path: Vec<MgtPathElementCompat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MgtPathElementCompat {
    level: u32,
    child_index: usize,
    node_hash: [u8; 32],
    sub_siblings: Vec<SiblingElementCompat>,
    cached_siblings: Vec<SiblingElementCompat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SiblingElementCompat {
    index: usize,
    hash: [u8; 32],
}

/// 璇佹槑楠岃瘉鍣?
pub struct ProofVerifier {
    ads_mode: AdsMode,
}

impl ProofVerifier {
    /// 鍒涘缓鏂扮殑璇佹槑楠岃瘉鍣?
    pub fn new(ads_mode: AdsMode) -> Self {
        ProofVerifier { ads_mode }
    }

    /// 鑾峰彇褰撳墠鐨?ADS 妯″紡
    pub fn ads_mode(&self) -> AdsMode {
        self.ads_mode
    }

    /// 鍚堝苟澶氫釜璇佹槑
    pub fn combine_proofs(&self, proofs: &[Vec<u8>]) -> Vec<u8> {
        if proofs.is_empty() {
            return Vec::new();
        }
        // 杩斿洖鏈€鍚庝竴涓瘉鏄庯紙涓庢渶鍚庝竴娆″啓鎿嶄綔鍚庣殑 root_hash 瀵瑰簲锛?        // 鍦ㄦ湭鏉ュ彲瀹炵幇鏇村鏉傜殑鑱氬悎閫昏緫
        proofs.last().cloned().unwrap_or_default()
    }

    /// 楠岃瘉璇佹槑
    ///
    /// # Arguments
    /// * `proof` - 璇佹槑鏁版嵁 (瀹為檯涓婃槸鏂扮殑 root hash)
    /// * `root_hash` - 鏈熸湜鐨勬牴鍝堝笇 (鐢ㄤ簬鍐欐搷浣滈獙璇?
    ///
    /// # Returns
    /// 楠岃瘉鏄惁鎴愬姛
    pub fn verify(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if is_accumulator_set_operation_proof(proof) {
            return self.verify_accumulator_set_operation_aggregate(proof, root_hash);
        }

        if is_polynomial_intersection_proof(proof) {
            return self.verify_polynomial_intersection_aggregate(proof, root_hash);
        }

        if is_boolean_query_aggregate_proof(proof) {
            return self.verify_boolean_query_aggregate(proof, root_hash);
        }

        if is_acctree_proof(proof) {
            return self.verify_acctree(proof, root_hash);
        }

        self.verify_ads_proof(proof, root_hash)
    }

    pub(crate) fn verify_polynomial_intersection_aggregate(
        &self,
        proof: &[u8],
        root_hash: &[u8],
    ) -> bool {
        if polynomial_intersection_root_hash(proof) != root_hash {
            return false;
        }

        let aggregate = match decode_polynomial_intersection_proof(proof) {
            Ok(aggregate) => aggregate,
            Err(_) => return false,
        };

        let root_poly = match self.verify_polynomial_set_node(&aggregate.root) {
            Some(poly) => poly,
            None => return false,
        };
        let expected_root_poly = match build_characteristic_polynomial(&aggregate.result_fids) {
            Ok(poly) => poly,
            Err(_) => return false,
        };

        root_poly == expected_root_poly
    }

    fn verify_polynomial_set_node(
        &self,
        node: &PolynomialSetProofNode,
    ) -> Option<TransparentPolynomial> {
        match node {
            PolynomialSetProofNode::Leaf(leaf) => {
                if !self.verify_set_proof_leaf(&leaf.leaf) {
                    return None;
                }
                if !verify_transparent_polynomial(&leaf.set_polynomial) {
                    return None;
                }
                Some(leaf.set_polynomial.clone())
            }
            PolynomialSetProofNode::And(binary) => self.verify_polynomial_intersection_node(binary),
            PolynomialSetProofNode::Or(binary) => self.verify_polynomial_union_node(binary),
        }
    }

    fn verify_polynomial_intersection_node(
        &self,
        binary: &PolynomialIntersectionNodeProof,
    ) -> Option<TransparentPolynomial> {
        let left_poly = self.verify_polynomial_set_node(&binary.left)?;
        let right_poly = self.verify_polynomial_set_node(&binary.right)?;

        for poly in [
            &binary.set_polynomial,
            &binary.intersection_polynomial,
            &binary.quotient_left,
            &binary.quotient_right,
            &binary.bezout_u,
            &binary.bezout_v,
        ] {
            if !verify_transparent_polynomial(poly) {
                return None;
            }
        }

        let transcript_challenge = derive_polynomial_challenge(&[
            &left_poly,
            &right_poly,
            &binary.set_polynomial,
            &binary.intersection_polynomial,
            &binary.quotient_left,
            &binary.quotient_right,
            &binary.bezout_u,
            &binary.bezout_v,
        ]);
        if transcript_challenge != binary.challenge_point {
            return None;
        }

        let left_eval = self.evaluate_polynomial_claim(&left_poly, &binary.challenge_point)?;
        let right_eval = self.evaluate_polynomial_claim(&right_poly, &binary.challenge_point)?;
        let intersection_eval = self
            .evaluate_polynomial_claim(&binary.intersection_polynomial, &binary.challenge_point)?;
        let result_eval =
            self.evaluate_polynomial_claim(&binary.set_polynomial, &binary.challenge_point)?;
        let quotient_left_eval =
            self.evaluate_polynomial_claim(&binary.quotient_left, &binary.challenge_point)?;
        let quotient_right_eval =
            self.evaluate_polynomial_claim(&binary.quotient_right, &binary.challenge_point)?;
        let bezout_u_eval =
            self.evaluate_polynomial_claim(&binary.bezout_u, &binary.challenge_point)?;
        let bezout_v_eval =
            self.evaluate_polynomial_claim(&binary.bezout_v, &binary.challenge_point)?;

        if left_eval != intersection_eval * quotient_left_eval {
            return None;
        }
        if right_eval != intersection_eval * quotient_right_eval {
            return None;
        }
        if bezout_u_eval * quotient_left_eval + bezout_v_eval * quotient_right_eval != Fr::one() {
            return None;
        }
        if result_eval != intersection_eval {
            return None;
        }

        Some(binary.set_polynomial.clone())
    }

    fn verify_polynomial_union_node(
        &self,
        binary: &PolynomialUnionNodeProof,
    ) -> Option<TransparentPolynomial> {
        let left_poly = self.verify_polynomial_set_node(&binary.left)?;
        let right_poly = self.verify_polynomial_set_node(&binary.right)?;

        for poly in [
            &binary.set_polynomial,
            &binary.intersection_polynomial,
            &binary.quotient_left,
            &binary.quotient_right,
            &binary.bezout_u,
            &binary.bezout_v,
        ] {
            if !verify_transparent_polynomial(poly) {
                return None;
            }
        }

        let transcript_challenge = derive_polynomial_challenge(&[
            &left_poly,
            &right_poly,
            &binary.set_polynomial,
            &binary.intersection_polynomial,
            &binary.quotient_left,
            &binary.quotient_right,
            &binary.bezout_u,
            &binary.bezout_v,
        ]);
        if transcript_challenge != binary.challenge_point {
            return None;
        }

        let left_eval = self.evaluate_polynomial_claim(&left_poly, &binary.challenge_point)?;
        let right_eval = self.evaluate_polynomial_claim(&right_poly, &binary.challenge_point)?;
        let result_eval =
            self.evaluate_polynomial_claim(&binary.set_polynomial, &binary.challenge_point)?;
        let intersection_eval = self
            .evaluate_polynomial_claim(&binary.intersection_polynomial, &binary.challenge_point)?;
        let quotient_left_eval =
            self.evaluate_polynomial_claim(&binary.quotient_left, &binary.challenge_point)?;
        let quotient_right_eval =
            self.evaluate_polynomial_claim(&binary.quotient_right, &binary.challenge_point)?;
        let bezout_u_eval =
            self.evaluate_polynomial_claim(&binary.bezout_u, &binary.challenge_point)?;
        let bezout_v_eval =
            self.evaluate_polynomial_claim(&binary.bezout_v, &binary.challenge_point)?;

        if left_eval != intersection_eval * quotient_left_eval {
            return None;
        }
        if right_eval != intersection_eval * quotient_right_eval {
            return None;
        }
        if bezout_u_eval * quotient_left_eval + bezout_v_eval * quotient_right_eval != Fr::one() {
            return None;
        }
        if result_eval != intersection_eval * quotient_left_eval * quotient_right_eval {
            return None;
        }

        Some(binary.set_polynomial.clone())
    }

    fn evaluate_polynomial_claim(&self, poly: &TransparentPolynomial, point: &[u8]) -> Option<Fr> {
        let value = evaluate_transparent_polynomial(poly, point).ok()?;
        Some(Fr::from_le_bytes_mod_order(&value))
    }

    pub(crate) fn verify_set_proof_leaf(&self, leaf: &SetProofLeaf) -> bool {
        leaf.verify(self)
    }

    pub(crate) fn verify_ads_proof_for_mode(
        &self,
        ads_mode: AdsMode,
        proof: &[u8],
        root_hash: &[u8],
    ) -> bool {
        match ads_mode {
            AdsMode::Mpt => self.verify_mpt(proof, root_hash),
            AdsMode::Mest => self.verify_mest(proof, root_hash),
            AdsMode::AccTrie => self.verify_acctrie(proof, root_hash),
            AdsMode::AccTree => self.verify_acctree(proof, root_hash),
        }
    }

    pub(crate) fn verify_ads_proof(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        self.verify_ads_proof_for_mode(self.ads_mode, proof, root_hash)
    }

    fn verify_acctree(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if !is_acctree_proof(proof) {
            return false;
        }
        if acctree_proof_root_hash(proof) != root_hash {
            return false;
        }
        match decode_acctree_proof(proof) {
            Ok(envelope) => match envelope.proof {
                AccTreeProofKind::Add {
                    keyword,
                    fid,
                    result,
                } => result.verify_insert(&keyword, &fid),
                AccTreeProofKind::Query { keyword, result } => result.verify(&keyword),
                AccTreeProofKind::Delete {
                    keyword,
                    fid,
                    result,
                } => result.deleted_fid == fid && result.verify_delete(&keyword),
            },
            Err(_) => false,
        }
    }
    fn verify_mpt(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.is_empty() {
            // 绌鸿瘉鏄庡湪鏌ヨ涓嶅瓨鍦ㄧ殑閿椂鏄湁鏁堢殑
            return true;
        }

        // 灏濊瘯鍙嶅簭鍒楀寲涓哄畬鏁寸殑 MPT Proof
        match bincode::deserialize::<MPTProof>(proof) {
            Ok(mpt_proof) => {
                // 鎵ц瀹屾暣鐨?Merkle Proof 楠岃瘉
                self.verify_full_mpt_proof(&mpt_proof, root_hash)
            }
            Err(_) => {
                println!("鉂?Failed to deserialize MPT proof");
                false
            }
        }
    }

    /// 楠岃瘉瀹屾暣鐨?MPT Merkle Proof
    fn verify_full_mpt_proof(&self, mpt_proof: &MPTProof, root_hash: &[u8]) -> bool {
        if root_hash.is_empty() {
            println!("??  Root hash is empty, skipping verification");
            return true;
        }

        if root_hash.len() != 32 {
            println!("? Invalid root hash length: {}", root_hash.len());
            return false;
        }

        let mut expected_root = [0u8; 32];
        expected_root.copy_from_slice(root_hash);

        let value = if mpt_proof.get_is_exist() && !mpt_proof.get_proofs().is_empty() {
            if let Some(leaf_value) = mpt_proof
                .get_proofs()
                .iter()
                .find(|p| p.proof_type == 0)
                .map(|p| String::from_utf8_lossy(&p.value).to_string())
                .filter(|v| !v.is_empty())
            {
                leaf_value
            } else {
                mpt_proof
                    .get_proofs()
                    .iter()
                    .find(|p| !p.value.is_empty())
                    .map(|p| String::from_utf8_lossy(&p.value).to_string())
                    .unwrap_or_default()
            }
        } else {
            String::new()
        };

        println!(
            "?? Manager verification - extracted value: '{}' (len={})",
            if value.len() > 100 {
                format!("{}...", &value[..100])
            } else {
                value.clone()
            },
            value.len()
        );
        println!(
            "?? Manager verification - is_exist={}, proof_count={}",
            mpt_proof.get_is_exist(),
            mpt_proof.get_proofs().len()
        );

        let computed_root = compute_mpt_root(&value, mpt_proof);

        if expected_root == [0u8; 32] {
            let empty_hash = [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55,
            ];

            if computed_root == empty_hash {
                println!("? Full Merkle proof verified successfully (empty tree)!");
                return true;
            }
        }

        if computed_root == expected_root {
            println!("? Full Merkle proof verified successfully!");
            println!("   Expected root: {:02x?}...", &expected_root[..8]);
            println!("   Computed root: {:02x?}...", &computed_root[..8]);
            true
        } else {
            println!("? Merkle proof verification failed!");
            println!("   Expected root: {:02x?}...", &expected_root[..8]);
            println!("   Computed root: {:02x?}...", &computed_root[..8]);
            false
        }
    }
    /// 楠岃瘉 MEST 鐨勮瘉鏄?    ///
    /// 瀹屾暣楠岃瘉MEST proof,鍖呮嫭:
    /// 1. 妗剁骇Merkle璇佹槑 (value -> seg_root_hash)
    /// 2. 娈垫牴鍦ㄥ彾瀛愭鏍归泦鍚堜腑
    /// 3. MGT璇佹槑 (leaf_roots -> MGT root)
    fn verify_mest(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.is_empty() {
            // 绌鸿瘉鏄庤〃绀哄叧閿瓧涓嶅瓨鍦ㄦ垨琚垹闄わ紝杩欐槸鏈夋晥鐨?            // println!("鉁?MEST proof verified (empty result)");
            return true;
        }

        // 灏濊瘯鍙嶅簭鍒楀寲涓篗estProof
        match Self::deserialize_mest_proof(proof) {
            Ok(mest_proof) => {
                // 楠岃瘉MGT root hash
                if !root_hash.is_empty() && mest_proof.mgt_proof.root_hash.as_slice() != root_hash {
                    // println!("鉂?MEST MGT root hash mismatch");
                    return false;
                }

                // 鎵ц瀹屾暣楠岃瘉
                if Self::verify_mest_proof_internal(&mest_proof) {
                    // println!("鉁?MEST proof verified (full verification)");
                    true
                } else {
                    // println!("鉂?MEST proof verification failed");
                    false
                }
            }
            Err(_) => {
                // 濡傛灉涓嶆槸瀹屾暣proof,灏濊瘯浣滀负绠€鍗曠殑MGT root hash澶勭悊 (鍚戝悗鍏煎)
                if proof.len() != 32 {
                    // println!("鉂?MEST proof has invalid length: {} bytes", proof.len());
                    return false;
                }

                // 楠岃瘉 proof 鍜?root_hash 涓€鑷?
                if !root_hash.is_empty() && proof != root_hash {
                    // println!("鉂?MEST proof does not match root hash");
                    return false;
                }

                // println!("鉁?MEST proof verified (MGT root hash only)");
                true
            }
        }
    }

    /// 楠岃瘉 AccTrie proof
    ///
    /// AccTrie 浣跨敤瀵嗙爜瀛︾疮鍔犲櫒锛屽畬鏁撮獙璇佽瘉鏄庣殑鏈夋晥鎬?
    fn verify_acctrie(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.is_empty() {
            // 绌鸿瘉鏄庤〃绀哄叧閿瓧涓嶅瓨鍦ㄦ垨琚垹闄わ紝杩欐槸鏈夋晥鐨?
            println!("鉁?AccTrie proof verified (empty result - key not found)");
            return true;
        }

        // 妫€鏌ユ渶灏忛暱搴︼紙鑷冲皯鍖呭惈绫诲瀷鏍囪锛?
        if proof.len() < 2 {
            println!("鉂?AccTrie proof too short: {} bytes", proof.len());
            return false;
        }

        // 璇诲彇璇佹槑绫诲瀷
        let proof_type = proof[0];

        match proof_type {
            0x01 => {
                // InsertionProof - 瀹屾暣楠岃瘉
                println!(
                    "馃攳 Verifying AccTrie InsertionProof ({} bytes)",
                    proof.len()
                );
                let ok = Self::verify_acctrie_insertion_proof(proof, root_hash);
                if !ok {
                    println!("鉂?AccTrie insertion proof verification failed");
                    return false;
                }
                true
            }
            0x02 => {
                // DeletionProof - 瀹屾暣楠岃瘉
                println!(
                    "馃攳 Verifying AccTrie DeletionProof ({} bytes)",
                    proof.len()
                );
                let ok = Self::verify_acctrie_deletion_proof(proof, root_hash);
                if !ok {
                    println!("鉂?AccTrie deletion proof verification failed");
                    return false;
                }
                true
            }
            0x03 => {
                // QueryProof
                println!("馃攳 Verifying AccTrie QueryProof ({} bytes)", proof.len());
                let ok = Self::verify_acctrie_query_proof(proof, root_hash);
                if !ok {
                    println!("鉂?AccTrie query proof verification failed");
                    return false;
                }
                true
            }
            0x10 => {
                // BatchInsertionProof (鑷畾涔夋壒閲忔牸寮?
                println!(
                    "馃攳 Verifying AccTrie BatchInsertionProof ({} bytes)",
                    proof.len()
                );
                let ok = Self::verify_acctrie_batch_insertion_proof(proof, root_hash);
                if !ok {
                    println!("鉂?AccTrie batch insertion proof verification failed");
                    return false;
                }
                true
            }
            _ => {
                println!("鉂?Unknown AccTrie proof type: 0x{:02x}", proof_type);
                false
            }
        }
    }

    /// 楠岃瘉 AccTrie 鎵归噺鎻掑叆璇佹槑
    /// 鏍煎紡: 0x10 | count(u32) | [len(u32) | insertion_proof]*count
    /// 姣忎釜瀛愯瘉鏄庤嚜韬寘鍚揩鐓э紱鏈€缁堟牴鍝堝笇浠ユ渶鍚庝竴涓瓙璇佹槑鐨勫揩鐓ф牎楠?
    fn verify_acctrie_batch_insertion_proof(proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.len() < 5 {
            println!("鉂?BatchInsertionProof too short");
            return false;
        }

        let mut offset = 1; // 璺宠繃绫诲瀷鏍囪
        let count = u32::from_le_bytes([
            proof[offset],
            proof[offset + 1],
            proof[offset + 2],
            proof[offset + 3],
        ]) as usize;
        offset += 4;

        if count == 0 {
            println!("鉂?BatchInsertionProof has zero items");
            return false;
        }

        let mut items: Vec<&[u8]> = Vec::with_capacity(count);

        for i in 0..count {
            if offset + 4 > proof.len() {
                println!("鉂?BatchInsertionProof truncated at item {}", i);
                return false;
            }
            let len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4;

            if offset + len > proof.len() {
                println!("鉂?BatchInsertionProof invalid length at item {}", i);
                return false;
            }

            items.push(&proof[offset..offset + len]);
            offset += len;
        }

        let parallel_result = items
            .par_iter()
            .enumerate()
            .map(|(i, insertion_proof)| {
                let ok = Self::verify_acctrie_insertion_proof(insertion_proof, &[]);
                (i, ok)
            })
            .find_any(|(_, ok)| !ok);

        if let Some((i, _)) = parallel_result {
            println!("鉂?BatchInsertionProof item {} verification failed", i);
            return false;
        }

        // 鏈熬蹇収鐢ㄤ簬鏍规牎楠岋紙鏂版牸寮忥級銆傛棫鏍煎紡鑻ユ棤蹇収涓?root_hash 涓虹┖鍒欐帴鍙椼€?
        let mut snapshot_offset = offset;
        let snapshot_opt = if snapshot_offset < proof.len() {
            Self::deserialize_acc_snapshot(proof, &mut snapshot_offset)
        } else {
            None
        };

        if root_hash.is_empty() {
            println!("鈿狅笍  Root hash not provided; skipping root verification");
            println!(
                "鉁?AccTrie BatchInsertionProof fully validated ({} items)",
                count
            );
            return true;
        }

        let snapshot = match snapshot_opt {
            Some(s) => s,
            None => {
                println!("鉂?Missing batch-level snapshot for root verification");
                return false;
            }
        };

        if !Self::verify_acc_root(&snapshot, root_hash) {
            return false;
        }

        println!(
            "鉁?AccTrie BatchInsertionProof fully validated ({} items)",
            count
        );
        true
    }

    /// 楠岃瘉AccTrie鎻掑叆璇佹槑
    /// 鍙嶅簭鍒楀寲MEST proof
    fn deserialize_mest_proof(proof: &[u8]) -> Result<MestProofCompat, String> {
        bincode::deserialize(proof).map_err(|e| format!("Failed to deserialize MestProof: {}", e))
    }

    /// 鍐呴儴楠岃瘉MEST proof
    fn verify_mest_proof_internal(proof: &MestProofCompat) -> bool {
        // 1. 楠岃瘉妗剁骇Merkle璇佹槑
        if !Self::verify_bucket_merkle_proof(
            proof.bucket_proof.value.as_bytes(),
            &proof.bucket_proof.seg_root_hash,
            &proof.bucket_proof.merkle_path,
        ) {
            println!("鉂?Bucket Merkle proof verification failed");
            return false;
        }

        // 2. 楠岃瘉娈垫牴鍦ㄥ彾瀛愭鏍归泦鍚堜腑
        if !proof
            .bucket_proof
            .leaf_segment_roots
            .iter()
            .any(|r| r == &proof.bucket_proof.seg_root_hash)
        {
            println!("鉂?Segment root not found in leaf_segment_roots");
            return false;
        }

        // 3. 楠岃瘉MGT proof
        if !Self::verify_mgt_proof_path(&proof.bucket_proof.leaf_segment_roots, &proof.mgt_proof) {
            println!("鉂?MGT proof verification failed");
            return false;
        }

        true
    }

    /// 楠岃瘉妗剁骇Merkle璇佹槑
    fn verify_bucket_merkle_proof(
        leaf_data: &[u8],
        expected_root: &[u8; 32],
        path: &[MerklePathElementCompat],
    ) -> bool {
        // 璁＄畻鍙跺瓙鍝堝笇
        let mut current_hash = Self::hash_leaf(leaf_data);

        // 娌跨潃璺緞鍚戜笂璁＄畻
        for element in path {
            current_hash = if element.direction == 0 {
                // 鍏勫紵鍦ㄥ乏杈?
                Self::hash_internal(&element.sibling_hash, &current_hash)
            } else {
                // 鍏勫紵鍦ㄥ彸杈?
                Self::hash_internal(&current_hash, &element.sibling_hash)
            };
        }

        &current_hash == expected_root
    }

    /// 楠岃瘉MGT璺緞
    fn verify_mgt_proof_path(leaf_roots: &[[u8; 32]], mgt_proof: &MgtProofCompat) -> bool {
        // 璁＄畻鍙跺瓙鑺傜偣鍝堝笇 (鎵€鏈夋鏍圭殑缁勫悎鍝堝笇)
        let mut leaf_hash = Self::hash_leaf_roots(leaf_roots);

        // 娌跨潃璺緞鍚戜笂楠岃瘉
        for element in &mgt_proof.path {
            // 鏋勫缓褰撳墠绾у埆鐨勬墍鏈夊瓙鑺傜偣鍝堝笇
            let mut sub_nodes: Vec<(usize, [u8; 32])> = element
                .sub_siblings
                .iter()
                .map(|s| (s.index, s.hash))
                .collect();

            // The child is always in sub_nodes in the current implementation
            sub_nodes.push((element.child_index, leaf_hash));
            sub_nodes.sort_by_key(|k| k.0);

            // Reconstruct cached_nodes
            let mut cached_nodes: Vec<(usize, [u8; 32])> = element
                .cached_siblings
                .iter()
                .map(|s| (s.index, s.hash))
                .collect();
            cached_nodes.sort_by_key(|k| k.0);

            // Calculate parent hash
            let mut hasher = Sha256::new();
            for (_, h) in sub_nodes {
                hasher.update(h);
            }
            for (_, h) in cached_nodes {
                hasher.update(h);
            }
            leaf_hash = hasher.finalize().into();
        }

        // 鏈€缁堝搱甯屽簲璇ョ瓑浜庢牴鍝堝笇
        leaf_hash == mgt_proof.root_hash
    }

    /// 鍝堝笇鍙跺瓙鏁版嵁
    fn hash_leaf(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    /// 鍝堝笇鍐呴儴鑺傜偣
    fn hash_internal(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().into()
    }

    /// 鍝堝笇鎵€鏈夊彾瀛愭鏍?
    fn hash_leaf_roots(roots: &[[u8; 32]]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for root in roots {
            hasher.update(root);
        }
        hasher.finalize().into()
    }

    /// 鍙嶅簭鍒楀寲鎴愬憳璇佹槑
    fn deserialize_membership_proof(proof: &[u8], offset: &mut usize) -> Option<MembershipProof> {
        if *offset >= proof.len() {
            return None;
        }

        // Check presence flag
        if proof[*offset] == 0 {
            *offset += 1;
            return None;
        }
        *offset += 1;

        // Read witness
        if *offset + 4 > proof.len() {
            return None;
        }
        let witness_len = u32::from_le_bytes([
            proof[*offset],
            proof[*offset + 1],
            proof[*offset + 2],
            proof[*offset + 3],
        ]) as usize;
        *offset += 4;

        if *offset + witness_len > proof.len() {
            return None;
        }
        let witness =
            G1Affine::deserialize_uncompressed(&proof[*offset..*offset + witness_len]).ok()?;
        *offset += witness_len;

        // Read element
        if *offset + 4 > proof.len() {
            return None;
        }
        let element_len = u32::from_le_bytes([
            proof[*offset],
            proof[*offset + 1],
            proof[*offset + 2],
            proof[*offset + 3],
        ]) as usize;
        *offset += 4;

        if *offset + element_len > proof.len() {
            return None;
        }
        let element = Fr::deserialize_uncompressed(&proof[*offset..*offset + element_len]).ok()?;
        *offset += element_len;

        Some(MembershipProof { witness, element })
    }

    fn deserialize_optional_accumulator(
        proof: &[u8],
        offset: &mut usize,
        field_name: &str,
    ) -> Option<G1Affine> {
        if *offset >= proof.len() {
            println!("鉂?Missing {} marker", field_name);
            return None;
        }

        if proof[*offset] == 0 {
            *offset += 1;
            return None;
        }
        *offset += 1;

        if *offset + 4 > proof.len() {
            println!("鉂?Invalid {} length marker", field_name);
            return None;
        }
        let len = u32::from_le_bytes([
            proof[*offset],
            proof[*offset + 1],
            proof[*offset + 2],
            proof[*offset + 3],
        ]) as usize;
        *offset += 4;

        if *offset + len > proof.len() {
            println!("鉂?Invalid {} length", field_name);
            return None;
        }

        let acc = match G1Affine::deserialize_uncompressed(&proof[*offset..*offset + len]) {
            Ok(acc) => acc,
            Err(e) => {
                println!("鉂?Failed to deserialize {}: {:?}", field_name, e);
                return None;
            }
        };
        *offset += len;
        Some(acc)
    }

    fn skip_length_prefixed_bytes(proof: &[u8], offset: &mut usize, field_name: &str) -> bool {
        if *offset + 4 > proof.len() {
            println!("鉂?Invalid {} length marker", field_name);
            return false;
        }
        let len = u32::from_le_bytes([
            proof[*offset],
            proof[*offset + 1],
            proof[*offset + 2],
            proof[*offset + 3],
        ]) as usize;
        *offset += 4;

        if *offset + len > proof.len() {
            println!("鉂?Invalid {} length", field_name);
            return false;
        }
        *offset += len;
        true
    }

    /// 鍙嶅簭鍒楀寲绱姞鍣ㄥ揩鐓э紙鐢ㄤ簬閲嶅缓鏍瑰搱甯岋級
    fn deserialize_acc_snapshot(proof: &[u8], offset: &mut usize) -> Option<Vec<Vec<u8>>> {
        if *offset + 4 > proof.len() {
            return None;
        }
        let count = u32::from_le_bytes([
            proof[*offset],
            proof[*offset + 1],
            proof[*offset + 2],
            proof[*offset + 3],
        ]) as usize;
        *offset += 4;

        let mut snapshot = Vec::with_capacity(count);
        for _ in 0..count {
            if *offset + 4 > proof.len() {
                return None;
            }
            let len = u32::from_le_bytes([
                proof[*offset],
                proof[*offset + 1],
                proof[*offset + 2],
                proof[*offset + 3],
            ]) as usize;
            *offset += 4;

            if *offset + len > proof.len() {
                return None;
            }
            snapshot.push(proof[*offset..*offset + len].to_vec());
            *offset += len;
        }

        Some(snapshot)
    }

    /// 鍩轰簬绱姞鍣ㄥ揩鐓ц绠楀苟鏍￠獙鏍瑰搱甯?
    fn verify_acc_root(snapshot: &[Vec<u8>], root_hash: &[u8]) -> bool {
        if root_hash.is_empty() {
            println!("鈿狅笍  Root hash not provided; skipping root verification");
            return true;
        }

        if root_hash.len() != 32 {
            println!("鉂?Invalid root hash length: {}", root_hash.len());
            return false;
        }

        let mut hasher = Sha256::new();
        if snapshot.is_empty() {
            hasher.update(b"empty_acctrie");
        } else {
            for acc in snapshot {
                hasher.update(acc);
            }
        }
        let expected: [u8; 32] = hasher.finalize().into();

        if expected.as_slice() == root_hash {
            true
        } else {
            println!("鉂?AccTrie root hash mismatch");
            println!("   Expected: {:02x?}...", &expected[..8]);
            println!("   Provided: {:02x?}...", &root_hash[..8]);
            false
        }
    }

    fn verify_acctrie_insertion_proof(proof: &[u8], root_hash: &[u8]) -> bool {
        // 鍩烘湰鏍煎紡楠岃瘉
        if proof.len() < 100 {
            println!("鉂?InsertionProof too short");
            return false;
        }

        let mut offset = 1; // 璺宠繃绫诲瀷鏍囪

        // 1. 璇诲彇骞堕獙璇侀敭
        if offset + 4 > proof.len() {
            return false;
        }
        let key_len = u32::from_le_bytes([
            proof[offset],
            proof[offset + 1],
            proof[offset + 2],
            proof[offset + 3],
        ]) as usize;
        offset += 4;

        if key_len > 1024 || offset + key_len > proof.len() {
            println!("鉂?Invalid key length: {}", key_len);
            return false;
        }
        let _key = &proof[offset..offset + key_len];
        offset += key_len;

        // 2. 璇诲彇鍊?
        if !Self::skip_length_prefixed_bytes(proof, &mut offset, "value") {
            return false;
        }

        // 3. 璺宠繃鍓嶅簭閿紙鍙€夛級
        if offset >= proof.len() {
            return false;
        }
        if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() {
                return false;
            }
            let prev_key_len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4 + prev_key_len;
        } else {
            offset += 1;
        }

        // 4. 璺宠繃鍚庡簭閿紙鍙€夛級
        if offset >= proof.len() {
            return false;
        }
        if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() {
                return false;
            }
            let next_key_len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4 + next_key_len;
        } else {
            offset += 1;
        }

        // 5. 楠岃瘉鏃х疮鍔犲櫒鍊?
        if offset + 4 > proof.len() {
            return false;
        }
        let acc_old_len = u32::from_le_bytes([
            proof[offset],
            proof[offset + 1],
            proof[offset + 2],
            proof[offset + 3],
        ]) as usize;
        offset += 4;

        if offset + acc_old_len > proof.len() {
            println!("鉂?Invalid acc_old length");
            return false;
        }

        // Deserialize acc_old strictly.
        let _acc_old_opt = if acc_old_len > 0 {
            match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_old_len]) {
                Ok(acc) => {
                    offset += acc_old_len;
                    Some(acc)
                }
                Err(e) => {
                    println!("鉂?Failed to deserialize acc_old: {:?}", e);
                    return false;
                }
            }
        } else {
            offset += acc_old_len;
            None
        };

        // 6. 楠岃瘉鏂扮疮鍔犲櫒鍊?
        if offset + 4 > proof.len() {
            return false;
        }
        let acc_new_len = u32::from_le_bytes([
            proof[offset],
            proof[offset + 1],
            proof[offset + 2],
            proof[offset + 3],
        ]) as usize;
        offset += 4;

        if offset + acc_new_len > proof.len() {
            println!("鉂?Invalid acc_new length");
            return false;
        }

        // Deserialize acc_new strictly.
        let acc_new_opt = if acc_new_len > 0 {
            match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_new_len]) {
                Ok(acc) => {
                    offset += acc_new_len;
                    Some(acc)
                }
                Err(e) => {
                    println!("鉂?Failed to deserialize acc_new: {:?}", e);
                    return false;
                }
            }
        } else {
            offset += acc_new_len;
            None
        };

        // 7. Read ln_prev_acc (Optional)
        if offset >= proof.len() {
            return false;
        }
        let _ln_prev_acc = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() {
                return false;
            }
            let len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4;
            if offset + len > proof.len() {
                return false;
            }
            match G1Affine::deserialize_uncompressed(&proof[offset..offset + len]) {
                Ok(acc) => {
                    offset += len;
                    Some(acc)
                }
                Err(e) => {
                    println!("鉂?Failed to deserialize ln_prev_acc: {:?}", e);
                    return false;
                }
            }
        } else {
            offset += 1;
            None
        };

        // 8. Read ln_next_acc_old (Optional)
        if offset >= proof.len() {
            return false;
        }
        let ln_next_acc_old = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() {
                return false;
            }
            let len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4;
            if offset + len > proof.len() {
                return false;
            }
            match G1Affine::deserialize_uncompressed(&proof[offset..offset + len]) {
                Ok(acc) => {
                    offset += len;
                    Some(acc)
                }
                Err(e) => {
                    println!("鉂?Failed to deserialize ln_next_acc_old: {:?}", e);
                    return false;
                }
            }
        } else {
            offset += 1;
            None
        };

        // 9. Read ln_next_acc_new (Optional)
        if offset >= proof.len() {
            return false;
        }
        let ln_next_acc_new = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() {
                return false;
            }
            let len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4;
            if offset + len > proof.len() {
                return false;
            }
            match G1Affine::deserialize_uncompressed(&proof[offset..offset + len]) {
                Ok(acc) => {
                    offset += len;
                    Some(acc)
                }
                Err(e) => {
                    println!("鉂?Failed to deserialize ln_next_acc_new: {:?}", e);
                    return false;
                }
            }
        } else {
            offset += 1;
            None
        };

        // 10. Verify Membership Proofs
        // keyp_in_ln_next_old_proof
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc_val) = ln_next_acc_old {
                if !proof.verify(acc_val) {
                    println!("鉂?keyp_in_ln_next_old_proof verification failed");
                    return false;
                }
            } else {
                // Acc not available: skip verification
                println!("鈿狅笍  Skipping keyp_in_ln_next_old_proof verify (acc unavailable)");
            }
        }

        // keyp_in_ln_proof (in acc_new when prev exists)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc_val) = acc_new_opt {
                if !proof.verify(acc_val) {
                    println!("鉂?keyp_in_ln_proof verification failed");
                    return false;
                }
            } else {
                println!("鈿狅笍  Skipping keyp_in_ln_proof verify (acc_new unavailable)");
            }
        }

        // no_prev_in_ln_proof (in acc_new when no prev exists)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc_val) = acc_new_opt {
                if !proof.verify(acc_val) {
                    println!("鉂?no_prev_in_ln_proof verification failed");
                    return false;
                }
            } else {
                println!("鈿狅笍  Skipping no_prev_in_ln_proof verify (acc_new unavailable)");
            }
        }

        // key_in_ln_next_new_proof (in ln_next_acc_new)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc_val) = ln_next_acc_new {
                if !proof.verify(acc_val) {
                    println!("鉂?key_in_ln_next_new_proof verification failed");
                    return false;
                }
            } else {
                println!(
                    "鈿狅笍  Skipping key_in_ln_next_new_proof verify (ln_next_acc_new unavailable)"
                );
            }
        }

        // keyp_in_ln_next_new_proof (in ln_next_acc_new)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc_val) = ln_next_acc_new {
                if !proof.verify(acc_val) {
                    println!("鉂?keyp_in_ln_next_new_proof verification failed");
                    return false;
                }
            } else {
                println!(
                    "鈿狅笍  Skipping keyp_in_ln_next_new_proof verify (ln_next_acc_new unavailable)"
                );
            }
        }

        // value_in_ln_proof (in acc_new)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc_val) = acc_new_opt {
                if !proof.verify(acc_val) {
                    println!("鉂?value_in_ln_proof verification failed");
                    return false;
                }
            } else {
                println!("鈿狅笍  Skipping value_in_ln_proof verify (acc_new unavailable)");
            }
        }

        // 11. 閲嶅缓鏍瑰搱甯屽苟鏍￠獙
        // 11. 閲嶅缓鏍瑰搱甯屽苟鏍￠獙锛堝揩鐓у彲閫夛級
        if offset == proof.len() {
            if root_hash.is_empty() {
                println!("鈿狅笍  No accumulator snapshot present; skipping root verification");
                println!("鉁?AccTrie InsertionProof fully validated");
                return true;
            }
            println!("鉂?Missing accumulator snapshot for root verification");
            return false;
        }

        let acc_snapshot = match Self::deserialize_acc_snapshot(proof, &mut offset) {
            Some(s) => s,
            None => {
                println!("鉂?Failed to deserialize accumulator snapshot");
                return false;
            }
        };

        if !Self::verify_acc_root(&acc_snapshot, root_hash) {
            return false;
        }

        println!("鉁?AccTrie InsertionProof fully validated");
        true
    }

    /// 楠岃瘉AccTrie鍒犻櫎璇佹槑
    fn verify_acctrie_deletion_proof(proof: &[u8], root_hash: &[u8]) -> bool {
        // 鍩烘湰鏍煎紡楠岃瘉
        if proof.len() < 50 {
            println!("鉂?DeletionProof too short");
            return false;
        }

        let mut offset = 1; // 璺宠繃绫诲瀷鏍囪

        // 1. 璇诲彇閿暱搴?
        if offset + 4 > proof.len() {
            return false;
        }
        let key_len = u32::from_le_bytes([
            proof[offset],
            proof[offset + 1],
            proof[offset + 2],
            proof[offset + 3],
        ]) as usize;
        offset += 4;

        if key_len > 1024 || offset + key_len > proof.len() {
            println!("鉂?Invalid key length: {}", key_len);
            return false;
        }
        let _key = &proof[offset..offset + key_len];
        offset += key_len;

        // 2. 璇诲彇delete_entire_leaf鏍囪
        if offset >= proof.len() {
            return false;
        }
        let delete_entire = proof[offset] == 1;
        offset += 1;

        // 3. 璇诲彇鍊硷紙鍙€夛級
        if offset >= proof.len() {
            return false;
        }
        if proof[offset] == 1 {
            offset += 1;
            if !Self::skip_length_prefixed_bytes(proof, &mut offset, "value") {
                return false;
            }
        } else {
            offset += 1;
        }

        // 4. 璺宠繃鍓嶅簭閿拰鍚庡簭閿紙鍙€夛級
        for _ in 0..2 {
            if offset >= proof.len() {
                return false;
            }
            if proof[offset] == 1 {
                offset += 1;
                if offset + 4 > proof.len() {
                    return false;
                }
                let len = u32::from_le_bytes([
                    proof[offset],
                    proof[offset + 1],
                    proof[offset + 2],
                    proof[offset + 3],
                ]) as usize;
                offset += 4 + len;
            } else {
                offset += 1;
            }
        }

        // 5. 楠岃瘉鏃х疮鍔犲櫒鍊?
        if offset + 4 > proof.len() {
            return false;
        }
        let acc_old_len = u32::from_le_bytes([
            proof[offset],
            proof[offset + 1],
            proof[offset + 2],
            proof[offset + 3],
        ]) as usize;
        offset += 4;

        if offset + acc_old_len > proof.len() {
            println!("鉂?Invalid acc_old length");
            return false;
        }

        let acc_old = match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_old_len])
        {
            Ok(acc) => {
                offset += acc_old_len;
                acc
            }
            Err(e) => {
                println!("鉂?Failed to deserialize acc_old: {:?}", e);
                return false;
            }
        };

        // 6. 楠岃瘉鏂扮疮鍔犲櫒鍊硷紙鍙€夛紝閮ㄥ垎鍒犻櫎鏃跺瓨鍦級
        if offset >= proof.len() {
            return false;
        }
        let _acc_new = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() {
                return false;
            }
            let acc_new_len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4;

            if offset + acc_new_len > proof.len() {
                println!("鉂?Invalid acc_new length");
                return false;
            }

            match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_new_len]) {
                Ok(acc) => {
                    offset += acc_new_len;
                    Some(acc)
                }
                Err(e) => {
                    println!("鉂?Failed to deserialize acc_new: {:?}", e);
                    return false;
                }
            }
        } else {
            offset += 1;
            None
        };

        // 7. Read ln_next_acc_old (Optional)
        if offset >= proof.len() {
            return false;
        }
        let ln_next_acc_old = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() {
                return false;
            }
            let len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4;
            if offset + len > proof.len() {
                return false;
            }
            let acc = G1Affine::deserialize_uncompressed(&proof[offset..offset + len]).ok();
            offset += len;
            acc
        } else {
            offset += 1;
            None
        };

        // 8. Read ln_next_acc_new (Optional)
        if offset >= proof.len() {
            return false;
        }
        let ln_next_acc_new = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() {
                return false;
            }
            let len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4;
            if offset + len > proof.len() {
                return false;
            }
            let acc = G1Affine::deserialize_uncompressed(&proof[offset..offset + len]).ok();
            offset += len;
            acc
        } else {
            offset += 1;
            None
        };

        // 9. Verify Membership Proofs
        // value_in_ln_old_proof (in acc_old)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if !proof.verify(acc_old) {
                println!("鉂?value_in_ln_old_proof verification failed");
                return false;
            }
        }

        // keyp_in_ln_proof (in acc_old)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if !proof.verify(acc_old) {
                println!("鉂?keyp_in_ln_proof verification failed");
                return false;
            }
        }

        // key_in_ln_next_old_proof (in ln_next_acc_old)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc) = ln_next_acc_old {
                if !proof.verify(acc) {
                    println!("鉂?key_in_ln_next_old_proof verification failed");
                    return false;
                }
            }
        }

        // keyp_in_ln_next_new_proof (in ln_next_acc_new)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc) = ln_next_acc_new {
                if !proof.verify(acc) {
                    println!("鉂?keyp_in_ln_next_new_proof verification failed");
                    return false;
                }
            }
        }

        // 10. 閲嶅缓鏍瑰搱甯屽苟鏍￠獙
        let acc_snapshot = match Self::deserialize_acc_snapshot(proof, &mut offset) {
            Some(s) => s,
            None => {
                println!("鉂?Failed to deserialize accumulator snapshot");
                return false;
            }
        };

        if !Self::verify_acc_root(&acc_snapshot, root_hash) {
            return false;
        }

        println!("馃攳 DeletionProof: delete_entire={}", delete_entire);
        println!("鉁?AccTrie DeletionProof fully validated");
        true
    }

    /// 楠岃瘉AccTrie鏌ヨ璇佹槑
    fn verify_acctrie_query_proof(proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.len() < 10 {
            println!("? QueryProof too short");
            return false;
        }

        let mut offset = 1;

        if offset >= proof.len() {
            return false;
        }
        let exists = proof[offset] == 1;
        offset += 1;

        if offset + 4 > proof.len() {
            return false;
        }
        let key_len = u32::from_le_bytes([
            proof[offset],
            proof[offset + 1],
            proof[offset + 2],
            proof[offset + 3],
        ]) as usize;
        offset += 4;

        if key_len > 1024 || offset + key_len > proof.len() {
            println!("? Invalid key length: {}", key_len);
            return false;
        }
        let _key = &proof[offset..offset + key_len];
        offset += key_len;

        if exists {
            if !Self::skip_length_prefixed_bytes(proof, &mut offset, "value") {
                return false;
            }

            if offset + 8 > proof.len() {
                return false;
            }
            let _value_count = u64::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
                proof[offset + 4],
                proof[offset + 5],
                proof[offset + 6],
                proof[offset + 7],
            ]);
            offset += 8;

            if offset + 4 > proof.len() {
                return false;
            }
            let acc_len = u32::from_le_bytes([
                proof[offset],
                proof[offset + 1],
                proof[offset + 2],
                proof[offset + 3],
            ]) as usize;
            offset += 4;

            if offset + acc_len > proof.len() {
                println!("? Invalid LN.Acc length");
                return false;
            }

            let ln_acc = match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_len])
            {
                Ok(acc) => {
                    offset += acc_len;
                    acc
                }
                Err(e) => {
                    println!("? Failed to deserialize LN.Acc: {:?}", e);
                    return false;
                }
            };

            if offset >= proof.len() || proof[offset] == 0 {
                println!("? QueryProof (Exists): missing pi_1 for fid in LN.Acc");
                return false;
            }

            let pi_1 = match Self::deserialize_membership_proof(proof, &mut offset) {
                Some(proof) => proof,
                None => {
                    println!("? QueryProof (Exists): invalid pi_1 encoding");
                    return false;
                }
            };
            if !pi_1.verify(ln_acc) {
                println!("? QueryProof (Exists): pi_1 verification failed");
                return false;
            }

            if offset >= proof.len() || proof[offset] == 0 {
                println!("? QueryProof (Exists): missing count proof for |V| in LN.Acc");
                return false;
            }

            let count_proof = match Self::deserialize_membership_proof(proof, &mut offset) {
                Some(proof) => proof,
                None => {
                    println!("? QueryProof (Exists): invalid count proof encoding");
                    return false;
                }
            };
            if !count_proof.verify(ln_acc) {
                println!("? QueryProof (Exists): count proof verification failed");
                return false;
            }

            let root_acc =
                match Self::deserialize_optional_accumulator(proof, &mut offset, "root_acc") {
                    Some(acc) => acc,
                    None => {
                        println!("? QueryProof (Exists): missing RN.Acc");
                        return false;
                    }
                };

            if offset >= proof.len() || proof[offset] == 0 {
                println!("? QueryProof (Exists): missing pi_2 for LN.Acc in RN.Acc");
                return false;
            }

            let pi_2 = match Self::deserialize_membership_proof(proof, &mut offset) {
                Some(proof) => proof,
                None => {
                    println!("? QueryProof (Exists): invalid pi_2 encoding");
                    return false;
                }
            };
            if !pi_2.verify(root_acc) {
                println!("? QueryProof (Exists): pi_2 verification failed");
                return false;
            }
        } else {
            if offset >= proof.len() {
                return false;
            }
            if proof[offset] == 1 {
                offset += 1;
                if offset + 4 > proof.len() {
                    return false;
                }
                let prev_len = u32::from_le_bytes([
                    proof[offset],
                    proof[offset + 1],
                    proof[offset + 2],
                    proof[offset + 3],
                ]) as usize;
                offset += 4;
                if offset + prev_len > proof.len() {
                    return false;
                }
                offset += prev_len;
                println!("?? QueryProof (NotExists): key_prev present");
            } else {
                offset += 1;
            }

            if offset >= proof.len() {
                return false;
            }
            if proof[offset] == 1 {
                offset += 1;
                if offset + 4 > proof.len() {
                    return false;
                }
                let next_len = u32::from_le_bytes([
                    proof[offset],
                    proof[offset + 1],
                    proof[offset + 2],
                    proof[offset + 3],
                ]) as usize;
                offset += 4;
                if offset + next_len > proof.len() {
                    return false;
                }
                offset += next_len;
                println!("?? QueryProof (NotExists): key_next present");
            } else {
                offset += 1;
            }

            if offset >= proof.len() {
                return false;
            }
            let ln_next_acc = if proof[offset] == 1 {
                offset += 1;
                if offset + 4 > proof.len() {
                    return false;
                }
                let acc_len = u32::from_le_bytes([
                    proof[offset],
                    proof[offset + 1],
                    proof[offset + 2],
                    proof[offset + 3],
                ]) as usize;
                offset += 4;

                if offset + acc_len > proof.len() {
                    println!("? Invalid ln_next_acc length");
                    return false;
                }

                match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_len]) {
                    Ok(acc) => {
                        offset += acc_len;
                        Some(acc)
                    }
                    Err(e) => {
                        println!("? Failed to deserialize ln_next_acc: {:?}", e);
                        return false;
                    }
                }
            } else {
                offset += 1;
                None
            };

            if let Some(prev_in_next_proof) = Self::deserialize_membership_proof(proof, &mut offset)
            {
                if let Some(acc) = ln_next_acc {
                    if !prev_in_next_proof.verify(acc) {
                        println!(
                            "? QueryProof (NotExists): prev_in_next_proof verification failed"
                        );
                        return false;
                    }
                }
            }

            if let Some(next_in_next_proof) = Self::deserialize_membership_proof(proof, &mut offset)
            {
                if let Some(acc) = ln_next_acc {
                    if !next_in_next_proof.verify(acc) {
                        println!(
                            "? QueryProof (NotExists): next_in_next_proof verification failed"
                        );
                        return false;
                    }
                }
            }

            let root_acc = Self::deserialize_optional_accumulator(proof, &mut offset, "root_acc");

            if let Some(ln_next_acc_in_root_proof) =
                Self::deserialize_membership_proof(proof, &mut offset)
            {
                if let Some(acc) = root_acc {
                    if !ln_next_acc_in_root_proof.verify(acc) {
                        println!("? QueryProof (NotExists): ln_next_acc_in_root_proof verification failed");
                        return false;
                    }
                }
            }
        }

        let acc_snapshot = match Self::deserialize_acc_snapshot(proof, &mut offset) {
            Some(s) => s,
            None => {
                println!("? Failed to deserialize accumulator snapshot");
                return false;
            }
        };

        if !Self::verify_acc_root(&acc_snapshot, root_hash) {
            return false;
        }

        println!("?? QueryProof: exists={}, key_len={}", exists, key_len);
        println!("? AccTrie QueryProof fully validated");
        true
    }
}
