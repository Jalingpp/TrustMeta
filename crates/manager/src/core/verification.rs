//! 证明验证模块
//!
//! 负责验证来自 storager 的密码学证明

use common::AdsMode;

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

    /// 验证 MPT 的证明
    ///
    /// 在当前实现中，proof 实际上就是新的 root hash
    /// 我们验证它的格式是否正确，并与返回的 root_hash 一致
    fn verify_mpt(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.is_empty() {
            // 空证明表示关键字不存在或被删除，这是有效的
            println!("✅ MPT proof verified (empty result)");
            return true;
        }
        
        if proof.len() != 32 {
            println!(
                "❌ MPT proof has invalid length: {} bytes (expected 32)",
                proof.len()
            );
            return false;
        }
        
        // 验证 proof 和 root_hash 一致
        if !root_hash.is_empty() && proof != root_hash {
            println!("❌ MPT proof does not match root hash");
            return false;
        }
        
        println!("✅ MPT proof verified");
        true
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
                // 对于基于 root hash 的证明系统，我们使用以下策略：
                // 1. 收集所有非空的 root hash
                // 2. 如果所有 root hash 都相同，返回该 hash
                // 3. 如果 root hash 不同（跨多个 storager），则对所有 hash 做聚合哈希
                //    这样可以验证所有参与查询的 storager 的状态
                
                let non_empty_proofs: Vec<&Vec<u8>> = proofs
                    .iter()
                    .filter(|p| !p.is_empty() && p.len() == 32)
                    .collect();

                if non_empty_proofs.is_empty() {
                    return Vec::new();
                }

                // 检查所有 proof 是否相同
                let first = non_empty_proofs[0];
                if non_empty_proofs.iter().all(|p| *p == first) {
                    return first.clone();
                }

                // 不同的 proof - 创建组合哈希
                use sha2::{Sha256, Digest};
                let mut hasher = Sha256::new();
                
                // 对所有 proof 排序后哈希，确保相同的集合产生相同的结果
                let mut sorted_proofs = non_empty_proofs.clone();
                sorted_proofs.sort();
                
                for proof in sorted_proofs {
                    hasher.update(proof);
                }
                
                hasher.finalize().to_vec()
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
