//! MEST (Merkle-based Extendible Segmented Hash Tree) ADS 实现
//!
//! MEST 是一个基于可扩展哈希和 Merkle 树的认证数据结构
//! 结合了 SEH (Segmented Extendible Hashing) 和 MGT (Merkle Group Tree)

use esa_rust::mest::{KVPair, MEHT, MestProof, BucketProof, MgtProof};
use super::AdsOperations;
use common::RootHash;
use std::sync::{Arc, RwLock};

/// MEST ADS 实现
pub struct MestAds {
    /// MEHT 实例 (Merkle-based Extendible Hash Table)
    meht: Arc<RwLock<MEHT>>,
}

impl MestAds {
    /// 创建新的 MEST ADS 实例
    ///
    /// # 参数
    /// - `rdx`: 基数 (每个节点的子节点数量)
    /// - `bucket_capacity`: 每个桶的容量
    /// - `bucket_seg_num`: 桶段数量
    pub fn new(rdx: i32, bucket_capacity: i32, bucket_seg_num: i32) -> Self {
        Self {
            meht: MEHT::new_simple(rdx, bucket_capacity, bucket_seg_num),
        }
    }

    /// 使用默认参数创建 MEST ADS
    /// - rdx=16 (16叉树)
    /// - bucket_capacity=100 (每桶100个条目)
    /// - bucket_seg_num=2 (2个段)
    pub fn new_default() -> Self {
        Self::new(16, 100, 2)
    }

    /// 编码 fid 列表为字符串 (逗号分隔)
    #[allow(dead_code)]
    fn encode_fids(fids: &[String]) -> String {
        fids.join(",")
    }

    /// 解码字符串为 fid 列表
    fn decode_fids(s: &str) -> Vec<String> {
        if s.is_empty() {
            Vec::new()
        } else {
            s.split(',').map(|s| s.to_string()).collect()
        }
    }

    /// 生成 MEST proof (完整的序列化格式)
    fn generate_mest_proof(&self, key_proof: &esa_rust::mest::KeyProof, is_exist: bool) -> Vec<u8> {
        use esa_rust::mest::proof::{MerklePathElement, MgtPathElement};
        
        // 转换桶级Merkle proof
        let merkle_path: Vec<MerklePathElement> = key_proof
            .bucket_proof
            .proof
            .proof_pairs
            .iter()
            .map(|(dir, hash)| MerklePathElement {
                direction: *dir,
                sibling_hash: *hash,
            })
            .collect();

        let bucket_proof = BucketProof {
            value: key_proof.bucket_proof.value.clone(),
            seg_root_hash: key_proof.bucket_proof.seg_root_hash,
            merkle_path,
            leaf_segment_roots: key_proof.bucket_proof.leaf_segment_roots.clone(),
        };

        // 转换MGT proof
        let mgt_path: Vec<MgtPathElement> = key_proof
            .mgt_proof
            .steps
            .iter()
            .enumerate()
            .map(|(level, step)| {
                use esa_rust::mest::proof::SiblingElement;
                
                let sub_siblings: Vec<SiblingElement> = step.sub_siblings.iter()
                    .map(|(idx, hash)| SiblingElement { index: *idx, hash: *hash })
                    .collect();
                    
                let cached_siblings: Vec<SiblingElement> = step.cached_siblings.iter()
                    .map(|(idx, hash)| SiblingElement { index: *idx, hash: *hash })
                    .collect();
                
                MgtPathElement {
                    level: level as u32,
                    child_index: step.idx,
                    node_hash: key_proof.mgt_proof.root_hash, // 使用根哈希作为占位符
                    sub_siblings,
                    cached_siblings,
                }
            })
            .collect();

        let mgt_proof = MgtProof {
            root_hash: key_proof.mgt_proof.root_hash,
            path: mgt_path,
        };

        // 组合成完整proof
        let mest_proof = MestProof {
            is_exist,
            key: key_proof.key.clone(),
            bucket_proof,
            mgt_proof,
        };

        // 序列化
        mest_proof.to_bytes()
    }
}

impl AdsOperations for MestAds {
    /// 添加 (keyword, fid) 对
    ///
    /// # 实现说明
    /// - 使用 keyword 作为 key
    /// - 如果 keyword 已存在，将 fid 追加到值列表中 (逗号分隔)
    /// - 返回完整的 MEST proof
    fn add(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let meht_w = self.meht.write().unwrap();

        // 插入 KVPair，MEHT 会自动合并同 key 的多个 value
        let key_proof = meht_w.insert(KVPair::new(keyword.to_string(), fid.to_string()));

        // 生成完整proof
        let proof = self.generate_mest_proof(&key_proof, true);
        let root_hash = key_proof.mgt_proof.root_hash.to_vec();

        drop(meht_w);

        (proof, root_hash)
    }

    /// 查询 keyword 对应的所有 fid
    ///
    /// # 实现说明
    /// - 查询 keyword 对应的值
    /// - 值是逗号分隔的 fid 列表
    /// - 返回完整的 MEST proof
    fn query(&self, keyword: &str) -> (Vec<String>, Vec<u8>) {
        let meht_r = self.meht.read().unwrap();

        // 查询 keyword
        if let Some(key_proof) = meht_r.query(keyword) {
            // 解码 fid 列表
            let fids = Self::decode_fids(&key_proof.bucket_proof.value);

            // 生成完整proof
            let proof = self.generate_mest_proof(&key_proof, true);

            drop(meht_r);
            (fids, proof)
        } else {
            // 未找到，返回空列表和空proof
            drop(meht_r);
            (Vec::new(), Vec::new())
        }
    }

    /// 删除 (keyword, fid) 对
    ///
    /// # 实现说明
    /// - 从 keyword 的值列表中删除指定的 fid
    /// - 如果删除后值列表为空，则删除整个 keyword
    /// - 返回完整的 MEST proof
    fn delete(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let meht_w = self.meht.write().unwrap();

        // 删除指定的 fid
        let _changed = meht_w.delete(keyword, fid);

        // 尝试查询以获取proof (如果还存在)
        let proof = if let Some(key_proof) = meht_w.query(keyword) {
            self.generate_mest_proof(&key_proof, true)
        } else {
            // keyword已完全删除,返回当前MGT root hash
            let mgt = meht_w.get_mgt();
            let mgt_r = mgt.read().unwrap();
            mgt_r.mgt_root_hash.to_vec()
        };

        // 获取更新后的 MGT root hash
        let mgt = meht_w.get_mgt();
        let mgt_r = mgt.read().unwrap();
        let root_hash = mgt_r.mgt_root_hash.to_vec();

        drop(mgt_r);
        drop(meht_w);

        (proof, root_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mest_ads_basic_operations() {
        let mut ads = MestAds::new_default();

        // Test Add
        let (proof1, root1) = ads.add("rust", "file1");
        assert!(proof1.len() > 32); // Full serialized proof is larger than just MGT root hash
        assert_eq!(root1.len(), 32);

        let (proof2, root2) = ads.add("rust", "file2");
        assert!(proof2.len() > 32);
        // Root hash should change after adding new data
        assert_ne!(root1, root2);

        // Test Query
        let (fids, proof) = ads.query("rust");
        assert_eq!(fids.len(), 2);
        assert!(fids.contains(&"file1".to_string()));
        assert!(fids.contains(&"file2".to_string()));
        assert!(proof.len() > 32); // Full proof

        // Test Delete
        let (proof3, root3) = ads.delete("rust", "file1");
        assert!(proof3.len() > 32); // Still returns full proof when keyword exists
        assert_ne!(root2, root3); // Root should change

        let (fids2, _) = ads.query("rust");
        assert_eq!(fids2.len(), 1);
        assert_eq!(fids2[0], "file2");

        // Delete last fid
        let (proof4, root4) = ads.delete("rust", "file2");
        assert_eq!(proof4.len(), 32); // Returns simple root hash when keyword is completely deleted
        assert_ne!(root3, root4);

        let (fids3, _) = ads.query("rust");
        assert_eq!(fids3.len(), 0); // Should be empty
    }

    #[test]
    fn test_mest_ads_multiple_keywords() {
        let mut ads = MestAds::new_default();

        // Add multiple keywords
        ads.add("rust", "f1");
        ads.add("storage", "f2");
        ads.add("distributed", "f3");
        ads.add("rust", "f4");
        ads.add("storage", "f5");

        // Query each keyword
        let (fids1, _) = ads.query("rust");
        assert_eq!(fids1.len(), 2);
        assert!(fids1.contains(&"f1".to_string()));
        assert!(fids1.contains(&"f4".to_string()));

        let (fids2, _) = ads.query("storage");
        assert_eq!(fids2.len(), 2);
        assert!(fids2.contains(&"f2".to_string()));
        assert!(fids2.contains(&"f5".to_string()));

        let (fids3, _) = ads.query("distributed");
        assert_eq!(fids3.len(), 1);
        assert_eq!(fids3[0], "f3");

        // Query non-existent keyword
        let (fids4, _) = ads.query("nonexistent");
        assert_eq!(fids4.len(), 0);
    }

    #[test]
    fn test_mest_ads_proof_verification() {
        let mut ads = MestAds::new_default();

        // Add some data
        let (proof1, root1) = ads.add("test", "file1");
        assert!(proof1.len() > 32); // Full proof
        let (proof2, root2) = ads.add("test", "file2");
        assert!(proof2.len() > 32); // Full proof

        // Root should change
        assert_ne!(root1, root2);

        // Query should return consistent full proof
        let (fids, proof_query) = ads.query("test");
        assert_eq!(fids.len(), 2);
        assert!(proof_query.len() > 32); // Full proof, not just root hash
        
        // Verify proof can be deserialized
        let mest_proof = MestProof::from_bytes(&proof_query).unwrap();
        assert!(mest_proof.is_exist);
        assert_eq!(mest_proof.key, "test");

        // Delete should update root
        let (proof3, root3) = ads.delete("test", "file1");
        assert!(proof3.len() > 32); // Still full proof as keyword exists
        assert_ne!(root2, root3);

        // Subsequent query should reflect the deletion
        let (fids, proof_after_delete) = ads.query("test");
        assert_eq!(fids.len(), 1);
        assert_eq!(fids[0], "file2");
        assert!(proof_after_delete.len() > 32); // Full proof
    }
}
