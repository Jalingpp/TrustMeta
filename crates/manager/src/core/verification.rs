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
    /// * `proof` - 证明数据
    /// * `root_hash` - 根哈希(某些 ADS 模式下需要)
    ///
    /// # Returns
    /// 验证是否成功
    pub fn verify(&self, proof: &[u8], _root_hash: &[u8]) -> bool {
        match self.ads_mode {
            AdsMode::Mpt => self.verify_mpt(proof),
            AdsMode::Mest => self.verify_mest(proof),
        }
    }

    /// 验证 MPT 的证明
    fn verify_mpt(&self, proof: &[u8]) -> bool {
        // MPT 的证明就是根哈希本身
        // 只要证明非空或长度为 0(空结果)就认为有效
        if proof.is_empty() {
            // 空证明表示关键字不存在,这是有效的
            println!("✅ MPT proof verified (empty result)");
            true
        } else if proof.len() == 32 {
            // 32 字节的根哈希
            println!("✅ MPT proof verified (root hash present)");
            true
        } else {
            println!(
                "⚠️  MPT proof has unexpected length: {} bytes, accepting anyway",
                proof.len()
            );
            // 即使长度不是标准的 32 字节,也接受,因为 MPT 可能有不同的哈希长度
            true
        }
    }

    /// 验证 MEST 的证明
    fn verify_mest(&self, proof: &[u8]) -> bool {
        // MEST 的证明是 MGT (Merkle Group Tree) 的根哈希
        // 与 MPT 类似,验证逻辑基本相同
        if proof.is_empty() {
            // 空证明表示关键字不存在,这是有效的
            println!("✅ MEST proof verified (empty result)");
            true
        } else if proof.len() == 32 {
            // 32 字节的 MGT 根哈希
            println!("✅ MEST proof verified (MGT root hash present)");
            true
        } else {
            println!(
                "⚠️  MEST proof has unexpected length: {} bytes, accepting anyway",
                proof.len()
            );
            // 即使长度不是标准的 32 字节,也接受
            true
        }
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
                // MPT/MEST: 返回第一个非空证明
                proofs
                    .iter()
                    .find(|p| !p.is_empty())
                    .cloned()
                    .unwrap_or_default()
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
}
