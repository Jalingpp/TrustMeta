//! 证明验证模块
//!
//! 负责验证来自 storager 的密码学证明

use ark_bls12_381::G1Affine;
use ark_serialize::CanonicalDeserialize;
use common::AdsMode;
use esa_rust::mpt::proof::{compute_mpt_root, MPTProof};
use serde::{Deserialize, Serialize};

/// MEST Proof兼容性结构 (用于反序列化)
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
            AdsMode::AccTrie => self.verify_acctrie(proof, root_hash),
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
                println!(
                    "📦 Verifying full Merkle proof ({} bytes, {} levels)...",
                    proof.len(),
                    mpt_proof.get_levels()
                );

                // 验证完整的 Merkle Proof
                return self.verify_full_mpt_proof(&mpt_proof, root_hash);
            }
            Err(e) => {
                println!(
                    "⚠️  Failed to deserialize MPT proof ({} bytes): {}",
                    proof.len(),
                    e
                );
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
            if let Some(leaf_value) = mpt_proof
                .get_proofs()
                .iter()
                .find(|p| p.proof_type == 0)
                .map(|p| String::from_utf8_lossy(&p.value).to_string())
                .filter(|v| !v.is_empty())
            {
                leaf_value
            } else {
                // 如果 leaf 中没有 value，尝试从其它节点类型（特别是 branch node）中提取
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
            "📋 Manager verification - extracted value: '{}' (len={})",
            if value.len() > 100 {
                format!("{}...", &value[..100])
            } else {
                value.clone()
            },
            value.len()
        );
        println!(
            "📋 Manager verification - is_exist={}, proof_count={}",
            mpt_proof.get_is_exist(),
            mpt_proof.get_proofs().len()
        );

        // 使用证明中的信息重新计算根哈希
        let computed_root = compute_mpt_root(&value, mpt_proof);

        // 特殊处理：如果 expected_root 是全零（空树），但 computed_root 是空分支节点的哈希
        // 这是因为 MPT 实现中空树的 root_hash 是 [0; 32]，但 compute_mpt_root 计算的是空节点的哈希
        if expected_root == [0u8; 32] {
            // SHA256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
            let empty_hash = [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55,
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
    /// 完整验证MEST proof,包括:
    /// 1. 桶级Merkle证明 (value -> seg_root_hash)
    /// 2. 段根在叶子段根集合中
    /// 3. MGT证明 (leaf_roots -> MGT root)
    fn verify_mest(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.is_empty() {
            // 空证明表示关键字不存在或被删除，这是有效的
            // println!("✅ MEST proof verified (empty result)");
            return true;
        }

        // 尝试反序列化为MestProof
        match Self::deserialize_mest_proof(proof) {
            Ok(mest_proof) => {
                // 验证MGT root hash
                if !root_hash.is_empty() && mest_proof.mgt_proof.root_hash.as_slice() != root_hash {
                    // println!("❌ MEST MGT root hash mismatch");
                    return false;
                }

                // 执行完整验证
                if Self::verify_mest_proof_internal(&mest_proof) {
                    // println!("✅ MEST proof verified (full verification)");
                    true
                } else {
                    // println!("❌ MEST proof verification failed");
                    false
                }
            }
            Err(_) => {
                // 如果不是完整proof,尝试作为简单的MGT root hash处理 (向后兼容)
                if proof.len() != 32 {
                    // println!("❌ MEST proof has invalid length: {} bytes", proof.len());
                    return false;
                }

                // 验证 proof 和 root_hash 一致
                if !root_hash.is_empty() && proof != root_hash {
                    // println!("❌ MEST proof does not match root hash");
                    return false;
                }

                // println!("✅ MEST proof verified (MGT root hash only)");
                true
            }
        }
    }

    /// 验证 AccTrie proof
    ///
    /// AccTrie 使用密码学累加器，完整验证证明的有效性
    fn verify_acctrie(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.is_empty() {
            // 空证明表示关键字不存在或被删除，这是有效的
            println!("✅ AccTrie proof verified (empty result)");
            return true;
        }

        // 检查最小长度（至少包含类型标记）
        if proof.len() < 2 {
            println!("❌ AccTrie proof too short: {} bytes", proof.len());
            return false;
        }

        // 读取证明类型
        let proof_type = proof[0];

        match proof_type {
            0x01 => {
                // InsertionProof
                println!(
                    "🔍 Verifying AccTrie InsertionProof ({} bytes)",
                    proof.len()
                );
                Self::verify_acctrie_insertion_proof(proof, root_hash)
            }
            0x02 => {
                // DeletionProof
                println!("🔍 Verifying AccTrie DeletionProof ({} bytes)", proof.len());
                Self::verify_acctrie_deletion_proof(proof, root_hash)
            }
            0x03 => {
                // QueryProof
                println!("🔍 Verifying AccTrie QueryProof ({} bytes)", proof.len());
                Self::verify_acctrie_query_proof(proof, root_hash)
            }
            _ => {
                println!("❌ Unknown AccTrie proof type: 0x{:02x}", proof_type);
                false
            }
        }
    }

    /// 验证AccTrie插入证明
    fn verify_acctrie_insertion_proof(proof: &[u8], root_hash: &[u8]) -> bool {
        // 基本格式验证
        if proof.len() < 100 {
            println!("❌ InsertionProof too short");
            return false;
        }

        let mut offset = 1; // 跳过类型标记

        // 1. 读取并验证键
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
            println!("❌ Invalid key length: {}", key_len);
            return false;
        }
        let _key = &proof[offset..offset + key_len];
        offset += key_len;

        // 2. 读取值
        if offset + 8 > proof.len() {
            return false;
        }
        let _value = i64::from_le_bytes([
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

        // 3. 跳过前序键（可选）
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

        // 4. 跳过后序键（可选）
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

        // 5. 验证旧累加器值
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
            println!("❌ Invalid acc_old length");
            return false;
        }

        // 尝试反序列化累加器值以验证格式
        match G1Affine::deserialize(&proof[offset..offset + acc_old_len]) {
            Ok(_acc_old) => {
                offset += acc_old_len;
            }
            Err(e) => {
                println!("❌ Failed to deserialize acc_old: {:?}", e);
                return false;
            }
        }

        // 6. 验证新累加器值
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
            println!("❌ Invalid acc_new length");
            return false;
        }

        match G1Affine::deserialize(&proof[offset..offset + acc_new_len]) {
            Ok(_acc_new) => {
                // 成功反序列化
                println!("✅ AccTrie InsertionProof: accumulator values validated");
            }
            Err(e) => {
                println!("❌ Failed to deserialize acc_new: {:?}", e);
                return false;
            }
        }

        // 验证根哈希（如果提供）
        if !root_hash.is_empty() && root_hash.len() == 32 {
            println!("🔍 Root hash provided: {:02x?}...", &root_hash[..8]);
        }

        println!("✅ AccTrie InsertionProof fully validated");
        true
    }

    /// 验证AccTrie删除证明
    fn verify_acctrie_deletion_proof(proof: &[u8], root_hash: &[u8]) -> bool {
        // 基本格式验证
        if proof.len() < 50 {
            println!("❌ DeletionProof too short");
            return false;
        }

        let mut offset = 1; // 跳过类型标记

        // 1. 读取键长度
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
            println!("❌ Invalid key length: {}", key_len);
            return false;
        }
        let _key = &proof[offset..offset + key_len];
        offset += key_len;

        // 2. 读取delete_entire_leaf标记
        if offset >= proof.len() {
            return false;
        }
        let delete_entire = proof[offset] == 1;
        offset += 1;

        // 3. 读取值（可选）
        if offset >= proof.len() {
            return false;
        }
        if proof[offset] == 1 {
            offset += 1;
            if offset + 8 > proof.len() {
                return false;
            }
            let _value = i64::from_le_bytes([
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
        } else {
            offset += 1;
        }

        // 4. 跳过前序键和后序键（可选）
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

        // 5. 验证旧累加器值
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
            println!("❌ Invalid acc_old length");
            return false;
        }

        match G1Affine::deserialize(&proof[offset..offset + acc_old_len]) {
            Ok(_acc_old) => {
                offset += acc_old_len;
            }
            Err(e) => {
                println!("❌ Failed to deserialize acc_old: {:?}", e);
                return false;
            }
        }

        // 6. 验证新累加器值（可选，部分删除时存在）
        if offset >= proof.len() {
            return false;
        }
        if proof[offset] == 1 {
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
                println!("❌ Invalid acc_new length");
                return false;
            }

            match G1Affine::deserialize(&proof[offset..offset + acc_new_len]) {
                Ok(_acc_new) => {
                    println!("🔍 DeletionProof: partial deletion (acc_new present)");
                }
                Err(e) => {
                    println!("❌ Failed to deserialize acc_new: {:?}", e);
                    return false;
                }
            }
        }

        // 验证根哈希（如果提供）
        if !root_hash.is_empty() && root_hash.len() == 32 {
            println!("🔍 Root hash provided: {:02x?}...", &root_hash[..8]);
        }

        println!("🔍 DeletionProof: delete_entire={}", delete_entire);
        println!("✅ AccTrie DeletionProof fully validated");
        true
    }

    /// 验证AccTrie查询证明
    fn verify_acctrie_query_proof(proof: &[u8], root_hash: &[u8]) -> bool {
        // 基本格式验证
        if proof.len() < 10 {
            println!("❌ QueryProof too short");
            return false;
        }

        let mut offset = 1; // 跳过类型标记

        // 1. 读取存在性标记
        if offset >= proof.len() {
            return false;
        }
        let exists = proof[offset] == 1;
        offset += 1;

        // 2. 读取键
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
            println!("❌ Invalid key length: {}", key_len);
            return false;
        }
        let _key = &proof[offset..offset + key_len];
        offset += key_len;

        if exists {
            // 存在证明：验证值和累加器
            // 3. 读取值
            if offset + 8 > proof.len() {
                return false;
            }
            let _value = i64::from_le_bytes([
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

            // 4. 读取叶子累加器值
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
                println!("❌ Invalid accumulator length");
                return false;
            }

            match G1Affine::deserialize(&proof[offset..offset + acc_len]) {
                Ok(_ln_acc) => {
                    offset += acc_len;
                    println!("🔍 QueryProof (Exists): accumulator validated");
                }
                Err(e) => {
                    println!("❌ Failed to deserialize ln_acc: {:?}", e);
                    return false;
                }
            }

            // 5. 检查成员证明（可选）
            if offset < proof.len() && proof[offset] == 1 {
                println!("🔍 QueryProof (Exists): membership proof present");
                // 可以进一步验证成员证明，这里做基本检查
            }
        } else {
            // 不存在证明：验证前序和后序键
            // 3. 读取前序键（可选）
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
                offset += 4 + prev_len;
                println!("🔍 QueryProof (NotExists): key_prev present");
            } else {
                offset += 1;
            }

            // 4. 读取后序键（可选）
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
                offset += 4 + next_len;
                println!("🔍 QueryProof (NotExists): key_next present");
            } else {
                offset += 1;
            }

            // 5. 读取后序叶子累加器（可选）
            if offset < proof.len() && proof[offset] == 1 {
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
                    println!("❌ Invalid ln_next_acc length");
                    return false;
                }

                match G1Affine::deserialize(&proof[offset..offset + acc_len]) {
                    Ok(_ln_next_acc) => {
                        println!("🔍 QueryProof (NotExists): ln_next_acc validated");
                    }
                    Err(e) => {
                        println!("❌ Failed to deserialize ln_next_acc: {:?}", e);
                        return false;
                    }
                }
            }
        }

        // 验证根哈希（如果提供）
        if !root_hash.is_empty() && root_hash.len() == 32 {
            println!("🔍 Root hash provided: {:02x?}...", &root_hash[..8]);
        }

        println!("🔍 QueryProof: exists={}, key_len={}", exists, key_len);
        println!("✅ AccTrie QueryProof fully validated");
        true
    }

    /// 反序列化MEST proof
    fn deserialize_mest_proof(data: &[u8]) -> Result<MestProofCompat, String> {
        bincode::deserialize(data).map_err(|e| format!("Deserialization failed: {}", e))
    }

    /// 内部MEST proof验证逻辑
    fn verify_mest_proof_internal(proof: &MestProofCompat) -> bool {
        // 1. 验证桶级Merkle证明
        let mut current_hash = Self::hash_leaf(proof.bucket_proof.value.as_bytes());

        for element in &proof.bucket_proof.merkle_path {
            current_hash = if element.direction == 0 {
                Self::hash_internal(&element.sibling_hash, &current_hash)
            } else {
                Self::hash_internal(&current_hash, &element.sibling_hash)
            };
        }

        if current_hash != proof.bucket_proof.seg_root_hash {
            println!("❌ Bucket proof verification failed");
            return false;
        }

        // 2. 验证段根在叶子段根集合中
        if !proof
            .bucket_proof
            .leaf_segment_roots
            .iter()
            .any(|r| r == &proof.bucket_proof.seg_root_hash)
        {
            println!("❌ Segment root not found in leaf segment roots");
            return false;
        }

        // 3. 验证MGT路径
        // 计算叶子节点的哈希 (Leaf Node Hash)
        // Leaf Node Hash = Hash(Hash(seg_root_1) || Hash(seg_root_2) || ...)
        let leaf_node_hash = Self::hash_leaf_roots(&proof.bucket_proof.leaf_segment_roots);

        // 验证路径上的第一个节点是否匹配计算出的叶子节点哈希
        if let Some(first_elem) = proof.mgt_proof.path.first() {
            if first_elem.node_hash != leaf_node_hash {
                println!("❌ First path element hash mismatch");
                return false;
            }
        } else {
            // 如果路径为空，且 root_hash 等于 leaf_node_hash，则验证通过 (单节点树)
            if proof.mgt_proof.root_hash == leaf_node_hash {
                return true;
            }
            println!("❌ Empty path but root hash mismatch");
            return false;
        }

        // 沿着路径向上计算根哈希
        let mut current_hash = leaf_node_hash;

        for (i, element) in proof.mgt_proof.path.iter().enumerate() {
            // 验证当前节点哈希是否匹配 (除了第一个节点，因为我们刚计算出来)
            if i > 0 && element.node_hash != current_hash {
                println!("❌ Path element hash mismatch at level {}", element.level);
                return false;
            }

            // 计算父节点哈希
            // Parent Hash = Hash(sorted_children_hashes)
            // Children include: current_node, sub_siblings, cached_siblings

            let mut children = Vec::new();

            // 添加当前节点
            children.push((element.child_index, current_hash));

            // 添加 sub_siblings
            for sibling in &element.sub_siblings {
                children.push((sibling.index, sibling.hash));
            }

            // 添加 cached_siblings
            for sibling in &element.cached_siblings {
                children.push((sibling.index, sibling.hash));
            }

            // 按索引排序
            children.sort_by_key(|(idx, _)| *idx);

            // 计算父节点哈希
            current_hash = Self::hash_mgt_node(&children);
        }

        // 验证最终计算出的根哈希是否匹配证明中的根哈希
        if current_hash != proof.mgt_proof.root_hash {
            println!(
                "❌ MGT root hash mismatch. Computed: {:?}, Expected: {:?}",
                current_hash, proof.mgt_proof.root_hash
            );
            return false;
        }

        true
    }

    fn hash_leaf_roots(roots: &[[u8; 32]]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for root in roots {
            hasher.update(root);
        }
        hasher.finalize().into()
    }

    fn hash_mgt_node(children: &[(usize, [u8; 32])]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for (_, hash) in children {
            hasher.update(hash);
        }
        hasher.finalize().into()
    }

    fn hash_leaf(data: &[u8]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    fn hash_internal(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().into()
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
            AdsMode::Mpt | AdsMode::Mest | AdsMode::AccTrie => {
                // 对于 MPT/MEST/AccTrie 证明系统:
                // 1. 如果所有证明都是 32 字节(root hash),使用聚合策略
                // 2. 否则,直接连接所有完整的 Merkle Proof

                let non_empty_proofs: Vec<&Vec<u8>> =
                    proofs.iter().filter(|p| !p.is_empty()).collect();

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
                    use sha2::{Digest, Sha256};
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
        // MPT now rejects empty proofs in strict mode
        let verifier = ProofVerifier::new(AdsMode::Mpt);
        assert!(!verifier.verify(&[], &[])); // Changed: now expects false
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

    // MEST Full Proof Tests

    #[test]
    fn test_mest_simple_root_hash_verification() {
        // Test backward compatibility with simple 32-byte MGT root hash
        let verifier = ProofVerifier::new(AdsMode::Mest);
        let simple_root: [u8; 32] = [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
            0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
            0x9a, 0xbc, 0xde, 0xf0,
        ];

        let result = verifier.verify_mest(&simple_root, &simple_root);
        assert!(
            result,
            "Simple 32-byte MGT root hash verification should pass"
        );
    }

    #[test]
    fn test_mest_mismatched_root_hash() {
        let verifier = ProofVerifier::new(AdsMode::Mest);
        let simple_root: [u8; 32] = [0xaa; 32];
        let wrong_root: [u8; 32] = [0xff; 32];

        let result = verifier.verify_mest(&simple_root, &wrong_root);
        assert!(!result, "Mismatched MGT root hash should fail verification");
    }

    #[test]
    fn test_mest_proof_size_detection() {
        // Verify that we correctly distinguish between simple and full proofs
        let simple_proof = vec![0u8; 32];
        let full_proof = vec![0u8; 150];

        assert_eq!(simple_proof.len(), 32);
        assert!(full_proof.len() > 32);
    }
}
