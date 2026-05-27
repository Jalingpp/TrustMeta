//! MEST Proof 序列化和验证
//!
//! 实现与MPT类似的proof序列化格式,用于网络传输和验证

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// MEST完整证明结构
/// 包含桶级Merkle证明和MGT级证明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MestProof {
    /// 是否存在该key
    pub is_exist: bool,
    /// key (keyword)
    pub key: String,
    /// 桶级证明
    pub bucket_proof: BucketProof,
    /// MGT级证明
    pub mgt_proof: MgtProof,
}

/// 桶级Merkle证明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketProof {
    /// 值 (逗号分隔的fid列表)
    pub value: String,
    /// 段根哈希
    pub seg_root_hash: [u8; 32],
    /// Merkle路径 (从叶子到段根)
    pub merkle_path: Vec<MerklePathElement>,
    /// 桶的所有段根哈希
    pub leaf_segment_roots: Vec<[u8; 32]>,
}

/// Merkle路径元素
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerklePathElement {
    /// 方向: 0=左兄弟, 1=右兄弟
    pub direction: u8,
    /// 兄弟节点哈希
    pub sibling_hash: [u8; 32],
}

/// MGT级证明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MgtProof {
    /// MGT根哈希
    pub root_hash: [u8; 32],
    /// 从叶子到根的路径
    pub path: Vec<MgtPathElement>,
}

/// MGT路径元素
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MgtPathElement {
    /// 节点级别 (0=根)
    pub level: u32,
    /// 子节点索引
    pub child_index: usize,
    /// 节点哈希
    pub node_hash: [u8; 32],
    /// 子节点兄弟 (index, hash)
    pub sub_siblings: Vec<SiblingElement>,
    /// 缓存节点兄弟 (index, hash)
    pub cached_siblings: Vec<SiblingElement>,
}

/// 兄弟节点元素
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiblingElement {
    pub index: usize,
    pub hash: [u8; 32],
}

impl MestProof {
    /// 序列化为字节数组 (用于网络传输)
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).expect("Failed to serialize MestProof")
    }

    /// 从字节数组反序列化
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        bincode::deserialize(data).map_err(|e| format!("Failed to deserialize MestProof: {}", e))
    }

    /// 计算proof大小
    pub fn size(&self) -> usize {
        std::mem::size_of::<bool>()
            + self.key.len()
            + self.bucket_proof.size()
            + self.mgt_proof.size()
    }

    /// 打印proof信息 (用于调试)
    pub fn print(&self) {
        println!("MEST Proof:");
        println!("  is_exist: {}", self.is_exist);
        println!("  key: {}", self.key);
        println!("  Bucket Proof:");
        println!("    value: {}", self.bucket_proof.value);
        println!(
            "    seg_root_hash: {:x?}",
            &self.bucket_proof.seg_root_hash[..8]
        );
        println!(
            "    merkle_path length: {}",
            self.bucket_proof.merkle_path.len()
        );
        println!(
            "    leaf_segment_roots: {}",
            self.bucket_proof.leaf_segment_roots.len()
        );
        println!("  MGT Proof:");
        println!("    root_hash: {:x?}", &self.mgt_proof.root_hash[..8]);
        println!("    path length: {}", self.mgt_proof.path.len());
    }
}

impl BucketProof {
    fn size(&self) -> usize {
        self.value.len()
            + 32 // seg_root_hash
            + self.merkle_path.len() * (1 + 32) // direction + sibling_hash
            + self.leaf_segment_roots.len() * 32
    }
}

impl MgtProof {
    fn size(&self) -> usize {
        32 // root_hash
            + self.path.iter().map(|p| {
                4 + 8 + 32 // level + child_index + node_hash
                + (p.sub_siblings.len() + p.cached_siblings.len()) * (8 + 32) // (index + hash)
            }).sum::<usize>()
    }
}

/// 验证MEST proof
pub fn verify_mest_proof(proof: &MestProof, expected_root: &[u8; 32]) -> bool {
    // 1. 验证桶级Merkle证明
    if !verify_bucket_merkle_proof(
        proof.bucket_proof.value.as_bytes(),
        &proof.bucket_proof.seg_root_hash,
        &proof.bucket_proof.merkle_path,
    ) {
        println!("❌ Bucket Merkle proof verification failed");
        return false;
    }

    // 2. 验证段根在叶子段根集合中
    if !proof
        .bucket_proof
        .leaf_segment_roots
        .iter()
        .any(|r| r == &proof.bucket_proof.seg_root_hash)
    {
        println!("❌ Segment root not found in leaf_segment_roots");
        return false;
    }

    // 3. 验证MGT proof
    if !verify_mgt_proof_path(&proof.bucket_proof.leaf_segment_roots, &proof.mgt_proof) {
        println!("❌ MGT proof verification failed");
        return false;
    }

    // 4. 验证MGT根哈希
    if &proof.mgt_proof.root_hash != expected_root {
        println!("❌ MGT root hash mismatch");
        return false;
    }

    true
}

/// 验证桶级Merkle证明
fn verify_bucket_merkle_proof(
    leaf_data: &[u8],
    expected_root: &[u8; 32],
    path: &[MerklePathElement],
) -> bool {
    // 计算叶子哈希
    let mut current_hash = hash_leaf(leaf_data);

    // 沿着路径向上计算
    for element in path {
        current_hash = if element.direction == 0 {
            // 兄弟在左边
            hash_internal(&element.sibling_hash, &current_hash)
        } else {
            // 兄弟在右边
            hash_internal(&current_hash, &element.sibling_hash)
        };
    }

    &current_hash == expected_root
}

/// 验证MGT路径
fn verify_mgt_proof_path(leaf_roots: &[[u8; 32]], mgt_proof: &MgtProof) -> bool {
    // 计算叶子节点哈希 (所有段根的组合哈希)
    let mut leaf_hash = hash_leaf_roots(leaf_roots);

    // 沿着路径向上验证
    for element in &mgt_proof.path {
        // 构建当前级别的所有子节点哈希
        // Reconstruct sub_nodes
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

    // 最终哈希应该等于根哈希
    leaf_hash == mgt_proof.root_hash
}

/// 哈希叶子数据
fn hash_leaf(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// 哈希内部节点
fn hash_internal(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

/// 哈希所有叶子段根
fn hash_leaf_roots(roots: &[[u8; 32]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for root in roots {
        hasher.update(root);
    }
    hasher.finalize().into()
}

/// 哈希MGT节点 (多个子节点)
#[allow(dead_code)]
fn hash_mgt_node(children: &[[u8; 32]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for child in children {
        hasher.update(child);
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bucket_merkle_proof() {
        let data = b"test_value";
        let leaf_hash = hash_leaf(data);

        // 创建简单的Merkle路径
        let path = vec![MerklePathElement {
            direction: 0,
            sibling_hash: [1u8; 32],
        }];

        let expected_root = hash_internal(&[1u8; 32], &leaf_hash);

        assert!(verify_bucket_merkle_proof(data, &expected_root, &path));
    }

    #[test]
    fn test_mest_proof_serialization() {
        let proof = MestProof {
            is_exist: true,
            key: "test_key".to_string(),
            bucket_proof: BucketProof {
                value: "fid1,fid2".to_string(),
                seg_root_hash: [1u8; 32],
                merkle_path: vec![],
                leaf_segment_roots: vec![[1u8; 32]],
            },
            mgt_proof: MgtProof {
                root_hash: [2u8; 32],
                path: vec![],
            },
        };

        let bytes = proof.to_bytes();
        let decoded = MestProof::from_bytes(&bytes).unwrap();

        assert_eq!(decoded.key, proof.key);
        assert_eq!(decoded.bucket_proof.value, proof.bucket_proof.value);
        assert_eq!(decoded.mgt_proof.root_hash, proof.mgt_proof.root_hash);
    }
}
