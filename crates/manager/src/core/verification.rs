//! 证明验证模块
//!
//! 负责验证来自 storager 的密码学证明

use common::AdsMode;
use esa_rust::mpt::proof::{compute_mpt_root, MPTProof};

/// 证明验证器
pub struct ProofVerifier {
    ads_mode: AdsMode,
}

impl ProofVerifier {
    /// 创建新的证明验证器
    pub fn new(ads_mode: AdsMode) -> Self {
        ProofVerifier { ads_mode }
    }

    /// 验证证明
    ///
    /// # Arguments
    /// * `proof` - 证明数据 (实际上是新的 root hash)
    /// * `root_hash` - 期望的根哈希 (用于写操作验证)
    ///
    /// # Returns
    /// 验证是否成功
    pub fn verify(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        match self.ads_mode {
            AdsMode::Mpt => self.verify_mpt(proof, root_hash),
            AdsMode::Mest => self.verify_mest(proof, root_hash),
        }
    }

    fn verify_mpt(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.is_empty() {
            // Strict mode: empty proof is not acceptable
            println!("❌ MPT proof is empty - rejecting");
            return false;
        }
        
        // 尝试反序列化为完整的 MPT Proof
        match bincode::deserialize::<MPTProof>(proof) {
            Ok(mpt_proof) => {
                println!("📦 Verifying full Merkle proof ({} bytes, {} levels)...", 
                         proof.len(), mpt_proof.get_levels());
                
                // 验证完整的 Merkle Proof
                return self.verify_full_mpt_proof(&mpt_proof, root_hash);
            }
            Err(e) => {
                println!("⚠️  Failed to deserialize MPT proof ({} bytes): {}", proof.len(), e);
                println!("❌ MPT proof has invalid format: {} bytes", proof.len());
                false
            }
        }
    }
    
    /// 验证完整的 MPT Merkle Proof
    fn verify_full_mpt_proof(&self, mpt_proof: &MPTProof, root_hash: &[u8]) -> bool {
        if root_hash.is_empty() {
            println!("⚠️  Root hash is empty, skipping verification");
            return true;
        }
        
        if root_hash.len() != 32 {
            println!("❌ Invalid root hash length: {}", root_hash.len());
            return false;
        }
        
        // 将 root_hash 转换为 [u8; 32]
        let mut expected_root = [0u8; 32];
        expected_root.copy_from_slice(root_hash);
        
        // 从证明中提取 value
        // 对于存在性证明，value 可能在 leaf node (type=0) 或 branch node (type=2) 的 value 字段中
        let value = if mpt_proof.get_is_exist() && !mpt_proof.get_proofs().is_empty() {
            // 优先查找 leaf node (type=0) 中的 value
            if let Some(leaf_value) = mpt_proof.get_proofs()
                .iter()
                .find(|p| p.proof_type == 0)
                .map(|p| String::from_utf8_lossy(&p.value).to_string())
                .filter(|v| !v.is_empty())
            {
                leaf_value
            } else {
                // 如果 leaf 中没有 value，尝试从其它节点类型（特别是 branch node）中提取
                mpt_proof.get_proofs()
                    .iter()
                    .find(|p| !p.value.is_empty())
                    .map(|p| String::from_utf8_lossy(&p.value).to_string())
                    .unwrap_or_default()
            }
        } else {
            String::new()
        };
        
        println!("📋 Manager verification - extracted value: '{}' (len={})", 
                 if value.len() > 100 { format!("{}...", &value[..100]) } else { value.clone() }, 
                 value.len());
        println!("📋 Manager verification - is_exist={}, proof_count={}", mpt_proof.get_is_exist(), mpt_proof.get_proofs().len());
        
        // 使用证明中的信息重新计算根哈希
        let computed_root = compute_mpt_root(&value, mpt_proof);
        
        // 特殊处理：如果 expected_root 是全零（空树），但 computed_root 是空分支节点的哈希
        // 这是因为 MPT 实现中空树的 root_hash 是 [0; 32]，但 compute_mpt_root 计算的是空节点的哈希
        if expected_root == [0u8; 32] {
            // SHA256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
            let empty_hash = [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 
                0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24, 
                0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 
                0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55
            ];
            
            if computed_root == empty_hash {
                println!("✅ Full Merkle proof verified successfully (empty tree)!");
                return true;
            }
        }

        if computed_root == expected_root {
            println!("✅ Full Merkle proof verified successfully!");
            println!("   Expected root: {:02x?}...", &expected_root[..8]);
            println!("   Computed root: {:02x?}...", &computed_root[..8]);
            true
        } else {
            println!("❌ Merkle proof verification failed!");
            println!("   Expected root: {:02x?}...", &expected_root[..8]);
            println!("   Computed root: {:02x?}...", &computed_root[..8]);
            false
        }
    }

    /// 验证 MEST 的证明
    ///
    /// 在当前实现中，proof 是 MGT root hash
    /// 我们验证它的格式是否正确，并与返回的 root_hash 一致
    fn verify_mest(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.is_empty() {
            // 空证明表示关键字不存在或被删除，这是有效的
            println!("✅ MEST proof verified (empty result)");
            return true;
        }
        
        if proof.len() != 32 {
            println!(
                "❌ MEST proof has invalid length: {} bytes (expected 32)",
                proof.len()
            );
            return false;
        }
        
        // 验证 proof 和 root_hash 一致
        if !root_hash.is_empty() && proof != root_hash {
            println!("❌ MEST proof does not match root hash");
            return false;
        }
        
        println!("✅ MEST proof verified (MGT root hash)");
        true
    }

    /// 合并多个证明
    ///
    /// 用于布尔查询等需要合并多个 storager 证明的场景
    ///
    /// # Arguments
    /// * `proofs` - 证明列表
    ///
    /// # Returns
    /// 合并后的证明
    pub fn combine_proofs(&self, proofs: &[Vec<u8>]) -> Vec<u8> {
        if proofs.is_empty() {
            return Vec::new();
        }

        match self.ads_mode {
            AdsMode::Mpt | AdsMode::Mest => {
                // 对于 MPT/MEST 证明系统:
                // 1. 如果所有证明都是 32 字节(root hash),使用聚合策略
                // 2. 否则,直接连接所有完整的 Merkle Proof
                
                let non_empty_proofs: Vec<&Vec<u8>> = proofs
                    .iter()
                    .filter(|p| !p.is_empty())
                    .collect();

                if non_empty_proofs.is_empty() {
                    return Vec::new();
                }

                // 检查是否所有都是 32 字节的 root hash
                let all_root_hashes = non_empty_proofs.iter().all(|p| p.len() == 32);

                if all_root_hashes {
                    // 旧的 root hash 组合逻辑
                    let first = non_empty_proofs[0];
                    if non_empty_proofs.iter().all(|p| *p == first) {
                        return first.clone();
                    }

                    // 不同的 root hash - 创建组合哈希
                    use sha2::{Sha256, Digest};
                    let mut hasher = Sha256::new();
                    
                    let mut sorted_proofs = non_empty_proofs.clone();
                    sorted_proofs.sort();
                    
                    for proof in sorted_proofs {
                        hasher.update(proof);
                    }
                    
                    hasher.finalize().to_vec()
                } else {
                    // 完整 Merkle Proof - 直接连接所有证明
                    // 格式: [proof1_len(4 bytes)][proof1][proof2_len(4 bytes)][proof2]...
                    let mut combined = Vec::new();
                    for proof in &non_empty_proofs {
                        // 添加证明长度(4 字节,大端序)
                        combined.extend_from_slice(&(proof.len() as u32).to_be_bytes());
                        // 添加证明内容
                        combined.extend_from_slice(proof);
                    }
                    combined
                }
            }
        }
    }

    /// 获取当前的 ADS 模式
    pub fn ads_mode(&self) -> AdsMode {
        self.ads_mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_proof_mpt() {
        let verifier = ProofVerifier::new(AdsMode::Mpt);
        assert!(verifier.verify(&[], &[]));
    }

    #[test]
    fn test_empty_proof_mest() {
        let verifier = ProofVerifier::new(AdsMode::Mest);
        assert!(verifier.verify(&[], &[]));
    }
    
    #[test]
    fn test_valid_proof_mest() {
        let verifier = ProofVerifier::new(AdsMode::Mest);
        let proof = vec![1u8; 32];
        let root_hash = vec![1u8; 32];
        assert!(verifier.verify(&proof, &root_hash));
    }
    
    #[test]
    fn test_invalid_proof_length() {
        let verifier = ProofVerifier::new(AdsMode::Mest);
        let proof = vec![1u8; 16]; // 错误的长度
        let root_hash = vec![1u8; 32];
        assert!(!verifier.verify(&proof, &root_hash));
    }
    
    #[test]
    fn test_mismatched_proof_and_root_hash() {
        let verifier = ProofVerifier::new(AdsMode::Mest);
        let proof = vec![1u8; 32];
        let root_hash = vec![2u8; 32];
        assert!(!verifier.verify(&proof, &root_hash));
    }
    
    #[test]
    fn test_combine_same_proofs() {
        let verifier = ProofVerifier::new(AdsMode::Mest);
        let proof1 = vec![1u8; 32];
        let proof2 = vec![1u8; 32];
        let combined = verifier.combine_proofs(&[proof1.clone(), proof2]);
        assert_eq!(combined, proof1);
    }
    
    #[test]
    fn test_combine_different_proofs() {
        let verifier = ProofVerifier::new(AdsMode::Mest);
        let proof1 = vec![1u8; 32];
        let proof2 = vec![2u8; 32];
        let combined = verifier.combine_proofs(&[proof1, proof2]);
        // 应该返回聚合哈希，长度仍为 32
        assert_eq!(combined.len(), 32);
    }
}
