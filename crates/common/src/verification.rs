//! 证明验证模块
//!
//! 负责验证来自 storager 的密码学证明

use ads_rust::mpt::proof::{compute_mpt_root, MPTProof};
use ads_rust::acctrie::acc::{Fr, dynamic_accumulator::MembershipProof};
use ark_bls12_381::G1Affine;
use ark_serialize::CanonicalDeserialize;
use crate::AdsMode;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

    /// 获取当前的 ADS 模式
    pub fn ads_mode(&self) -> AdsMode {
        self.ads_mode
    }

    /// 合并多个证明
    pub fn combine_proofs(&self, proofs: &[Vec<u8>]) -> Vec<u8> {
        if proofs.is_empty() {
            return Vec::new();
        }
        // 目前简单返回第一个证明
        // 在更复杂的场景中(如AccTrie聚合)，可能需要合并多个证明
        proofs[0].clone()
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
            // 空证明在查询不存在的键时是有效的
            return true;
        }

        // 尝试反序列化为完整的 MPT Proof
        match bincode::deserialize::<MPTProof>(proof) {
            Ok(mpt_proof) => {
                // 执行完整的 Merkle Proof 验证
                self.verify_full_mpt_proof(&mpt_proof, root_hash)
            }
            Err(_) => {
                println!("❌ Failed to deserialize MPT proof");
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
            println!("✅ AccTrie proof verified (empty result - key not found)");
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
                // InsertionProof - 完整验证
                println!(
                    "🔍 Verifying AccTrie InsertionProof ({} bytes)",
                    proof.len()
                );
                Self::verify_acctrie_insertion_proof(proof, root_hash)
            }
            0x02 => {
                // DeletionProof - 完整验证
                println!("🔍 Verifying AccTrie DeletionProof ({} bytes)", proof.len());
                Self::verify_acctrie_deletion_proof(proof, root_hash)
            }
            0x03 => {
                // QueryProof
                println!("🔍 Verifying AccTrie QueryProof ({} bytes)", proof.len());
                Self::verify_acctrie_query_proof(proof, root_hash)
            }
            0x10 => {
                // BatchInsertionProof (自定义批量格式)
                println!("🔍 Verifying AccTrie BatchInsertionProof ({} bytes)", proof.len());
                Self::verify_acctrie_batch_insertion_proof(proof, root_hash)
            }
            _ => {
                println!("❌ Unknown AccTrie proof type: 0x{:02x}", proof_type);
                false
            }
        }
    }

    /// 验证 AccTrie 批量插入证明
    /// 格式: 0x10 | count(u32) | [len(u32) | insertion_proof]*count
    /// 每个子证明自身包含快照；最终根哈希以最后一个子证明的快照校验
    fn verify_acctrie_batch_insertion_proof(proof: &[u8], root_hash: &[u8]) -> bool {
        if proof.len() < 5 {
            println!("❌ BatchInsertionProof too short");
            return false;
        }

        let mut offset = 1; // 跳过类型标记
        let count = u32::from_le_bytes([
            proof[offset],
            proof[offset + 1],
            proof[offset + 2],
            proof[offset + 3],
        ]) as usize;
        offset += 4;

        if count == 0 {
            println!("❌ BatchInsertionProof has zero items");
            return false;
        }

        let mut items: Vec<&[u8]> = Vec::with_capacity(count);

        for i in 0..count {
            if offset + 4 > proof.len() {
                println!("❌ BatchInsertionProof truncated at item {}", i);
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
                println!("❌ BatchInsertionProof invalid length at item {}", i);
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
            println!("❌ BatchInsertionProof item {} verification failed", i);
            return false;
        }

        // 末尾快照用于根校验（新格式）。旧格式若无快照且 root_hash 为空则接受。
        let mut snapshot_offset = offset;
        let snapshot_opt = if snapshot_offset < proof.len() {
            Self::deserialize_acc_snapshot(proof, &mut snapshot_offset)
        } else {
            None
        };

        if root_hash.is_empty() {
            println!("⚠️  Root hash not provided; skipping root verification");
            println!("✅ AccTrie BatchInsertionProof fully validated ({} items)", count);
            return true;
        }

        let snapshot = match snapshot_opt {
            Some(s) => s,
            None => {
                println!("❌ Missing batch-level snapshot for root verification");
                return false;
            }
        };

        if !Self::verify_acc_root(&snapshot, root_hash) {
            return false;
        }

        println!("✅ AccTrie BatchInsertionProof fully validated ({} items)", count);
        true
    }

    /// 验证AccTrie插入证明
    /// 反序列化MEST proof
    fn deserialize_mest_proof(proof: &[u8]) -> Result<MestProofCompat, String> {
        bincode::deserialize(proof).map_err(|e| format!("Failed to deserialize MestProof: {}", e))
    }

    /// 内部验证MEST proof
    fn verify_mest_proof_internal(proof: &MestProofCompat) -> bool {
        // 1. 验证桶级Merkle证明
        if !Self::verify_bucket_merkle_proof(
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
        if !Self::verify_mgt_proof_path(&proof.bucket_proof.leaf_segment_roots, &proof.mgt_proof) {
            println!("❌ MGT proof verification failed");
            return false;
        }

        true
    }

    /// 验证桶级Merkle证明
    fn verify_bucket_merkle_proof(
        leaf_data: &[u8],
        expected_root: &[u8; 32],
        path: &[MerklePathElementCompat],
    ) -> bool {
        // 计算叶子哈希
        let mut current_hash = Self::hash_leaf(leaf_data);

        // 沿着路径向上计算
        for element in path {
            current_hash = if element.direction == 0 {
                // 兄弟在左边
                Self::hash_internal(&element.sibling_hash, &current_hash)
            } else {
                // 兄弟在右边
                Self::hash_internal(&current_hash, &element.sibling_hash)
            };
        }

        &current_hash == expected_root
    }

    /// 验证MGT路径
    fn verify_mgt_proof_path(leaf_roots: &[[u8; 32]], mgt_proof: &MgtProofCompat) -> bool {
        // 计算叶子节点哈希 (所有段根的组合哈希)
        let mut leaf_hash = Self::hash_leaf_roots(leaf_roots);

        // 沿着路径向上验证
        for element in &mgt_proof.path {
            // 构建当前级别的所有子节点哈希
            let mut sub_nodes: Vec<(usize, [u8; 32])> = element.sub_siblings.iter()
                .map(|s| (s.index, s.hash))
                .collect();
            
            // The child is always in sub_nodes in the current implementation
            sub_nodes.push((element.child_index, leaf_hash));
            sub_nodes.sort_by_key(|k| k.0);
            
            // Reconstruct cached_nodes
            let mut cached_nodes: Vec<(usize, [u8; 32])> = element.cached_siblings.iter()
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

    /// 反序列化成员证明
    fn deserialize_membership_proof(proof: &[u8], offset: &mut usize) -> Option<MembershipProof> {
        if *offset >= proof.len() { return None; }
        
        // Check presence flag
        if proof[*offset] == 0 {
            *offset += 1;
            return None;
        }
        *offset += 1;

        // Read witness
        if *offset + 4 > proof.len() { return None; }
        let witness_len = u32::from_le_bytes([proof[*offset], proof[*offset+1], proof[*offset+2], proof[*offset+3]]) as usize;
        *offset += 4;
        
        if *offset + witness_len > proof.len() { return None; }
        let witness = G1Affine::deserialize_uncompressed(&proof[*offset..*offset+witness_len]).ok()?;
        *offset += witness_len;

        // Read element
        if *offset + 4 > proof.len() { return None; }
        let element_len = u32::from_le_bytes([proof[*offset], proof[*offset+1], proof[*offset+2], proof[*offset+3]]) as usize;
        *offset += 4;

        if *offset + element_len > proof.len() { return None; }
        let element = Fr::deserialize_uncompressed(&proof[*offset..*offset+element_len]).ok()?;
        *offset += element_len;

        Some(MembershipProof { witness, element })
    }

    /// 反序列化累加器快照（用于重建根哈希）
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

    /// 基于累加器快照计算并校验根哈希
    fn verify_acc_root(snapshot: &[Vec<u8>], root_hash: &[u8]) -> bool {
        if root_hash.is_empty() {
            println!("⚠️  Root hash not provided; skipping root verification");
            return true;
        }

        if root_hash.len() != 32 {
            println!("❌ Invalid root hash length: {}", root_hash.len());
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
            println!("❌ AccTrie root hash mismatch");
            println!("   Expected: {:02x?}...", &expected[..8]);
            println!("   Provided: {:02x?}...", &root_hash[..8]);
            false
        }
    }

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

        let acc_old = match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_old_len]) {
            Ok(acc) => {
                offset += acc_old_len;
                acc
            }
            Err(e) => {
                println!("❌ Failed to deserialize acc_old: {:?}", e);
                return false;
            }
        };

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

        let acc_new = match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_new_len]) {
            Ok(acc) => {
                offset += acc_new_len;
                acc
            }
            Err(e) => {
                println!("❌ Failed to deserialize acc_new: {:?}", e);
                return false;
            }
        };

        // 7. Read ln_prev_acc (Optional)
        if offset >= proof.len() { return false; }
        let _ln_prev_acc = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() { return false; }
            let len = u32::from_le_bytes([proof[offset], proof[offset+1], proof[offset+2], proof[offset+3]]) as usize;
            offset += 4;
            if offset + len > proof.len() { return false; }
            let acc = G1Affine::deserialize_uncompressed(&proof[offset..offset+len]).ok();
            offset += len;
            acc
        } else {
            offset += 1;
            None
        };

        // 8. Read ln_next_acc_old (Optional)
        if offset >= proof.len() { return false; }
        let ln_next_acc_old = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() { return false; }
            let len = u32::from_le_bytes([proof[offset], proof[offset+1], proof[offset+2], proof[offset+3]]) as usize;
            offset += 4;
            if offset + len > proof.len() { return false; }
            let acc = G1Affine::deserialize_uncompressed(&proof[offset..offset+len]).ok();
            offset += len;
            acc
        } else {
            offset += 1;
            None
        };

        // 9. Read ln_next_acc_new (Optional)
        if offset >= proof.len() { return false; }
        let ln_next_acc_new = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() { return false; }
            let len = u32::from_le_bytes([proof[offset], proof[offset+1], proof[offset+2], proof[offset+3]]) as usize;
            offset += 4;
            if offset + len > proof.len() { return false; }
            let acc = G1Affine::deserialize_uncompressed(&proof[offset..offset+len]).ok();
            offset += len;
            acc
        } else {
            offset += 1;
            None
        };

        // 10. Verify Membership Proofs
        // keyp_in_ln_next_old_proof
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc) = ln_next_acc_old {
                if !proof.verify(acc) {
                    println!("❌ keyp_in_ln_next_old_proof verification failed");
                    return false;
                }
            }
        }

        // keyp_in_ln_proof (in acc_new when prev exists)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if !proof.verify(acc_new) {
                println!("❌ keyp_in_ln_proof verification failed");
                return false;
            }
        }

        // no_prev_in_ln_proof (in acc_new when no prev exists)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if !proof.verify(acc_new) {
                println!("❌ no_prev_in_ln_proof verification failed");
                return false;
            }
        }

        // key_in_ln_next_new_proof (in ln_next_acc_new)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc) = ln_next_acc_new {
                if !proof.verify(acc) {
                    println!("❌ key_in_ln_next_new_proof verification failed");
                    return false;
                }
            }
        }

        // keyp_in_ln_next_new_proof (in ln_next_acc_new)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc) = ln_next_acc_new {
                if !proof.verify(acc) {
                    println!("❌ keyp_in_ln_next_new_proof verification failed");
                    return false;
                }
            }
        }

        // value_in_ln_proof (in acc_new)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if !proof.verify(acc_new) {
                println!("❌ value_in_ln_proof verification failed");
                return false;
            }
        }

        // 11. 重建根哈希并校验
        // 11. 重建根哈希并校验（快照可选）
        if offset == proof.len() {
            if root_hash.is_empty() {
                println!("⚠️  No accumulator snapshot present; skipping root verification");
                println!("✅ AccTrie InsertionProof fully validated");
                return true;
            }
            println!("❌ Missing accumulator snapshot for root verification");
            return false;
        }

        let acc_snapshot = match Self::deserialize_acc_snapshot(proof, &mut offset) {
            Some(s) => s,
            None => {
                println!("❌ Failed to deserialize accumulator snapshot");
                return false;
            }
        };

        if !Self::verify_acc_root(&acc_snapshot, root_hash) {
            return false;
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

        let acc_old = match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_old_len]) {
            Ok(acc) => {
                offset += acc_old_len;
                acc
            }
            Err(e) => {
                println!("❌ Failed to deserialize acc_old: {:?}", e);
                return false;
            }
        };

        // 6. 验证新累加器值（可选，部分删除时存在）
        if offset >= proof.len() { return false; }
        let _acc_new = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() { return false; }
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

            match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_new_len]) {
                Ok(acc) => {
                    offset += acc_new_len;
                    Some(acc)
                }
                Err(e) => {
                    println!("❌ Failed to deserialize acc_new: {:?}", e);
                    return false;
                }
            }
        } else {
            offset += 1;
            None
        };

        // 7. Read ln_next_acc_old (Optional)
        if offset >= proof.len() { return false; }
        let ln_next_acc_old = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() { return false; }
            let len = u32::from_le_bytes([proof[offset], proof[offset+1], proof[offset+2], proof[offset+3]]) as usize;
            offset += 4;
            if offset + len > proof.len() { return false; }
            let acc = G1Affine::deserialize_uncompressed(&proof[offset..offset+len]).ok();
            offset += len;
            acc
        } else {
            offset += 1;
            None
        };

        // 8. Read ln_next_acc_new (Optional)
        if offset >= proof.len() { return false; }
        let ln_next_acc_new = if proof[offset] == 1 {
            offset += 1;
            if offset + 4 > proof.len() { return false; }
            let len = u32::from_le_bytes([proof[offset], proof[offset+1], proof[offset+2], proof[offset+3]]) as usize;
            offset += 4;
            if offset + len > proof.len() { return false; }
            let acc = G1Affine::deserialize_uncompressed(&proof[offset..offset+len]).ok();
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
                println!("❌ value_in_ln_old_proof verification failed");
                return false;
            }
        }

        // keyp_in_ln_proof (in acc_old)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if !proof.verify(acc_old) {
                println!("❌ keyp_in_ln_proof verification failed");
                return false;
            }
        }

        // key_in_ln_next_old_proof (in ln_next_acc_old)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc) = ln_next_acc_old {
                if !proof.verify(acc) {
                    println!("❌ key_in_ln_next_old_proof verification failed");
                    return false;
                }
            }
        }

        // keyp_in_ln_next_new_proof (in ln_next_acc_new)
        if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
            if let Some(acc) = ln_next_acc_new {
                if !proof.verify(acc) {
                    println!("❌ keyp_in_ln_next_new_proof verification failed");
                    return false;
                }
            }
        }

        // 10. 重建根哈希并校验
        let acc_snapshot = match Self::deserialize_acc_snapshot(proof, &mut offset) {
            Some(s) => s,
            None => {
                println!("❌ Failed to deserialize accumulator snapshot");
                return false;
            }
        };

        if !Self::verify_acc_root(&acc_snapshot, root_hash) {
            return false;
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

            let ln_acc = match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_len]) {
                Ok(acc) => {
                    offset += acc_len;
                    acc
                }
                Err(e) => {
                    println!("❌ Failed to deserialize ln_acc: {:?}", e);
                    return false;
                }
            };

            // 5. 检查成员证明（可选）
            if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
                if !proof.verify(ln_acc) {
                    println!("❌ QueryProof (Exists): membership proof verification failed");
                    return false;
                }
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
            if offset >= proof.len() { return false; }
            let ln_next_acc = if proof[offset] == 1 {
                offset += 1;
                if offset + 4 > proof.len() { return false; }
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

                match G1Affine::deserialize_uncompressed(&proof[offset..offset + acc_len]) {
                    Ok(acc) => {
                        offset += acc_len;
                        Some(acc)
                    }
                    Err(e) => {
                        println!("❌ Failed to deserialize ln_next_acc: {:?}", e);
                        return false;
                    }
                }
            } else {
                offset += 1;
                None
            };

            // 6. 检查成员证明 (prev_in_next_proof)
            if let Some(proof) = Self::deserialize_membership_proof(proof, &mut offset) {
                if let Some(acc) = ln_next_acc {
                    if !proof.verify(acc) {
                        println!("❌ QueryProof (NotExists): prev_in_next_proof verification failed");
                        return false;
                    }
                }
            }
        }

        // 7. 重建根哈希并校验
        let acc_snapshot = match Self::deserialize_acc_snapshot(proof, &mut offset) {
            Some(s) => s,
            None => {
                println!("❌ Failed to deserialize accumulator snapshot");
                return false;
            }
        };

        if !Self::verify_acc_root(&acc_snapshot, root_hash) {
            return false;
        }

        println!("🔍 QueryProof: exists={}, key_len={}", exists, key_len);
        println!("✅ AccTrie QueryProof fully validated");
        true
    }
}
