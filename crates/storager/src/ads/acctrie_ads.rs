//! AccTrie (Accumulator-based Trie) ADS 实现
//!
//! AccTrie 是一个结合密码学累加器的前缀树数据结构
//! 每个叶子节点维护一个值集合及其对应的密码学累加器，支持高效的成员证明和集合操作

use super::AdsOperations;
use common::RootHash;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// 引入 acctrie 库
use ads_rust::acctrie::{AccTrie, DeletionProof, InsertionProof, Node, QueryResult};

/// AccTrie ADS 实现
pub struct AccTrieAds {
    /// AccTrie 实例
    trie: Arc<RwLock<AccTrie>>,

    /// 存储 keyword -> 多个 fid 的映射
    /// 由于 AccTrie 的 Value 类型是 i64，我们需要额外的映射来存储字符串 fid
    fid_storage: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl AccTrieAds {
    /// 创建新的 AccTrie ADS 实例
    pub fn new() -> Self {
        Self {
            trie: Arc::new(RwLock::new(AccTrie::new())),
            fid_storage: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 将字符串 fid 转换为 i64 值（用于存储在累加器中）
    fn fid_to_value(fid: &str) -> i64 {
        // 使用简单的哈希函数将字符串转换为 i64
        // 实际应用中可以使用更好的哈希函数
        let mut hash: i64 = 0;
        for (i, byte) in fid.bytes().enumerate() {
            hash = hash.wrapping_add((byte as i64).wrapping_mul(31_i64.wrapping_pow(i as u32)));
        }
        hash
    }

    /// 序列化插入证明为字节数组（完整版本）
    fn serialize_insertion_proof(proof: &InsertionProof) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();

        // 证明类型标记: 0x01 = InsertionProof
        bytes.push(0x01);

        // 序列化键
        bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&proof.key);

        // 序列化值
        bytes.extend_from_slice(&proof.value.to_le_bytes());

        // 序列化前序键（可选）
        if let Some(ref key_prev) = proof.key_prev {
            bytes.push(1);
            bytes.extend_from_slice(&(key_prev.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key_prev);
        } else {
            bytes.push(0);
        }

        // 序列化后序键（可选）
        if let Some(ref key_next) = proof.key_next {
            bytes.push(1);
            bytes.extend_from_slice(&(key_next.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key_next);
        } else {
            bytes.push(0);
        }

        // 序列化累加器值
        let mut acc_old_bytes = Vec::new();
        proof
            .ln_acc_old
            .serialize_uncompressed(&mut acc_old_bytes)
            .unwrap();
        bytes.extend_from_slice(&(acc_old_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&acc_old_bytes);

        let mut acc_new_bytes = Vec::new();
        proof
            .ln_acc_new
            .serialize_uncompressed(&mut acc_new_bytes)
            .unwrap();
        bytes.extend_from_slice(&(acc_new_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&acc_new_bytes);

        // 序列化前序叶子累加器（可选）
        if let Some(ln_prev_acc) = proof.ln_prev_acc {
            bytes.push(1);
            let mut prev_acc_bytes = Vec::new();
            ln_prev_acc
                .serialize_uncompressed(&mut prev_acc_bytes)
                .unwrap();
            bytes.extend_from_slice(&(prev_acc_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&prev_acc_bytes);
        } else {
            bytes.push(0);
        }

        // 序列化后序叶子累加器（可选）
        if let Some(ln_next_acc_old) = proof.ln_next_acc_old {
            bytes.push(1);
            let mut next_old_bytes = Vec::new();
            ln_next_acc_old
                .serialize_uncompressed(&mut next_old_bytes)
                .unwrap();
            bytes.extend_from_slice(&(next_old_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&next_old_bytes);
        } else {
            bytes.push(0);
        }

        if let Some(ln_next_acc_new) = proof.ln_next_acc_new {
            bytes.push(1);
            let mut next_new_bytes = Vec::new();
            ln_next_acc_new
                .serialize_uncompressed(&mut next_new_bytes)
                .unwrap();
            bytes.extend_from_slice(&(next_new_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&next_new_bytes);
        } else {
            bytes.push(0);
        }

        bytes
    }

    /// 序列化删除证明为字节数组（完整版本）
    fn serialize_deletion_proof(proof: &DeletionProof) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();

        // 证明类型标记: 0x02 = DeletionProof
        bytes.push(0x02);

        // 序列化键
        bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&proof.key);

        // 是否删除整个叶子
        bytes.push(if proof.delete_entire_leaf { 1 } else { 0 });

        // 序列化值（可选）
        if let Some(value) = proof.value {
            bytes.push(1);
            bytes.extend_from_slice(&value.to_le_bytes());
        } else {
            bytes.push(0);
        }

        // 序列化前序键（可选）
        if let Some(ref key_prev) = proof.key_prev {
            bytes.push(1);
            bytes.extend_from_slice(&(key_prev.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key_prev);
        } else {
            bytes.push(0);
        }

        // 序列化后序键（可选）
        if let Some(ref key_next) = proof.key_next {
            bytes.push(1);
            bytes.extend_from_slice(&(key_next.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key_next);
        } else {
            bytes.push(0);
        }

        // 序列化旧累加器值
        let mut acc_old_bytes = Vec::new();
        proof
            .ln_acc_old
            .serialize_uncompressed(&mut acc_old_bytes)
            .unwrap();
        bytes.extend_from_slice(&(acc_old_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&acc_old_bytes);

        // 序列化新累加器值（可选）
        if let Some(acc_new) = proof.ln_acc_new {
            bytes.push(1);
            let mut acc_new_bytes = Vec::new();
            acc_new.serialize_uncompressed(&mut acc_new_bytes).unwrap();
            bytes.extend_from_slice(&(acc_new_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&acc_new_bytes);
        } else {
            bytes.push(0);
        }

        // 序列化后序叶子累加器（可选）
        if let Some(ln_next_acc_old) = proof.ln_next_acc_old {
            bytes.push(1);
            let mut next_old_bytes = Vec::new();
            ln_next_acc_old
                .serialize_uncompressed(&mut next_old_bytes)
                .unwrap();
            bytes.extend_from_slice(&(next_old_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&next_old_bytes);
        } else {
            bytes.push(0);
        }

        if let Some(ln_next_acc_new) = proof.ln_next_acc_new {
            bytes.push(1);
            let mut next_new_bytes = Vec::new();
            ln_next_acc_new
                .serialize_uncompressed(&mut next_new_bytes)
                .unwrap();
            bytes.extend_from_slice(&(next_new_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&next_new_bytes);
        } else {
            bytes.push(0);
        }

        bytes
    }

    /// 序列化查询结果为字节数组（完整版本，包含成员证明）
    fn serialize_query_result(result: &QueryResult) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();

        match result {
            QueryResult::Exists(proof) => {
                // 证明类型标记: 0x03 = QueryProof (Exists)
                bytes.push(0x03);
                bytes.push(1); // 存在标记

                bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&proof.key);
                bytes.extend_from_slice(&proof.value.to_le_bytes());

                // 序列化叶子累加器值
                let mut acc_bytes = Vec::new();
                proof.ln_acc.serialize_uncompressed(&mut acc_bytes).unwrap();
                bytes.extend_from_slice(&(acc_bytes.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&acc_bytes);

                // 序列化成员证明（如果有）
                if let Some(ref membership_proof) = proof.membership_proof {
                    bytes.push(1);
                    // 序列化witness
                    let mut witness_bytes = Vec::new();
                    membership_proof
                        .witness
                        .serialize_uncompressed(&mut witness_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(witness_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&witness_bytes);

                    // 序列化element
                    let mut element_bytes = Vec::new();
                    membership_proof
                        .element
                        .serialize_uncompressed(&mut element_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(element_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&element_bytes);
                } else {
                    bytes.push(0);
                }
            }
            QueryResult::NotExists(proof) => {
                // 证明类型标记: 0x03 = QueryProof (NotExists)
                bytes.push(0x03);
                bytes.push(0); // 不存在标记

                bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&proof.key);

                // 序列化前序键
                if let Some(ref key_prev) = proof.key_prev {
                    bytes.push(1);
                    bytes.extend_from_slice(&(key_prev.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(key_prev);
                } else {
                    bytes.push(0);
                }

                // 序列化后序键
                if let Some(ref key_next) = proof.key_next {
                    bytes.push(1);
                    bytes.extend_from_slice(&(key_next.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(key_next);
                } else {
                    bytes.push(0);
                }

                // 序列化后序叶子累加器
                if let Some(ln_next_acc) = proof.ln_next_acc {
                    bytes.push(1);
                    let mut acc_bytes = Vec::new();
                    ln_next_acc.serialize_uncompressed(&mut acc_bytes).unwrap();
                    bytes.extend_from_slice(&(acc_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&acc_bytes);
                } else {
                    bytes.push(0);
                }

                // 序列化前序在后序中的成员证明（如果有）
                if let Some(ref prev_in_next_proof) = proof.prev_in_next_proof {
                    bytes.push(1);
                    let mut witness_bytes = Vec::new();
                    prev_in_next_proof
                        .witness
                        .serialize_uncompressed(&mut witness_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(witness_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&witness_bytes);

                    let mut element_bytes = Vec::new();
                    prev_in_next_proof
                        .element
                        .serialize_uncompressed(&mut element_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(element_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&element_bytes);
                } else {
                    bytes.push(0);
                }
            }
        }

        bytes
    }

    /// 从 AccTrie 获取根哈希
    /// 由于 AccTrie 使用累加器，我们需要从所有叶子节点的累加器值计算根哈希
    fn get_root_hash(&self) -> RootHash {
        use sha2::{Digest, Sha256};

        let trie = self.trie.read().unwrap();

        // 遍历所有叶子节点，收集累加器值
        let mut hasher = Sha256::new();

        if trie.is_empty() {
            // 空树，返回零哈希
            hasher.update(b"empty_acctrie");
        } else {
            // 遍历叶子链表
            let mut current = trie.head_leaf.clone();
            while let Some(current_ref) = current {
                let node = current_ref.read().unwrap();
                if let Node::Leaf(leaf) = &*node {
                    // 序列化累加器值并加入哈希
                    use ark_serialize::CanonicalSerialize;
                    let mut acc_bytes = Vec::new();
                    leaf.accumulator_value()
                        .serialize_uncompressed(&mut acc_bytes)
                        .unwrap();
                    hasher.update(&acc_bytes);

                    // 移动到下一个叶子
                    current = leaf.next.clone();
                } else {
                    break;
                }
            }
        }

        hasher.finalize().to_vec()
    }
}

impl Default for AccTrieAds {
    fn default() -> Self {
        Self::new()
    }
}

impl AdsOperations for AccTrieAds {
    /// 添加 (keyword, fid) 对到 AccTrie
    /// 返回: (proof, root_hash)
    fn add(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        // 将 fid 转换为 i64 值
        let value = Self::fid_to_value(fid);

        // 存储 fid 到映射中
        {
            let mut storage = self.fid_storage.write().unwrap();
            storage
                .entry(keyword.to_string())
                .or_insert_with(Vec::new)
                .push(fid.to_string());
        }

        // 插入到 AccTrie
        let mut trie = self.trie.write().unwrap();
        let key = keyword.as_bytes().to_vec();

        let proof = match trie.insert(key, value) {
            Ok(proof) => {
                eprintln!(
                    "🔧 AccTrie Add: keyword='{}', fid='{}' (success)",
                    keyword, fid
                );
                Self::serialize_insertion_proof(&proof)
            }
            Err(e) => {
                eprintln!(
                    "❌ AccTrie Add: keyword='{}', fid='{}' failed: {:?}",
                    keyword, fid, e
                );
                Vec::new() // 插入失败，返回空证明
            }
        };

        // 获取根哈希
        drop(trie);
        let root_hash = self.get_root_hash();

        eprintln!(
            "🔧 AccTrie Add: proof size={} bytes, root_hash={:02x?}...",
            proof.len(),
            &root_hash[..8.min(root_hash.len())]
        );

        (proof, root_hash)
    }

    /// 查询 keyword 对应的所有 fid
    /// 返回: (fids, proof)
    fn query(&self, keyword: &str) -> (Vec<String>, Vec<u8>) {
        // 从存储中获取 fid 列表
        let fids = {
            let storage = self.fid_storage.read().unwrap();
            storage.get(keyword).cloned().unwrap_or_default()
        };

        if fids.is_empty() {
            eprintln!("🔍 AccTrie Query: keyword='{}' not found", keyword);
            return (Vec::new(), Vec::new());
        }

        eprintln!(
            "🔍 AccTrie Query: keyword='{}', found {} fids",
            keyword,
            fids.len()
        );

        // 查询第一个 fid 的证明（作为代表）
        let value = Self::fid_to_value(&fids[0]);
        let key = keyword.as_bytes().to_vec();

        let proof = {
            let trie = self.trie.read().unwrap();
            match trie.query(&key, value) {
                Ok(result) => {
                    let serialized = Self::serialize_query_result(&result);
                    eprintln!(
                        "🔍 AccTrie Query: returning proof ({} bytes)",
                        serialized.len()
                    );
                    serialized
                }
                Err(e) => {
                    eprintln!("⚠️ AccTrie Query: proof generation failed: {:?}", e);
                    Vec::new()
                }
            }
        };

        (fids, proof)
    }

    /// 从 AccTrie 中删除 (keyword, fid) 对
    /// 返回: (proof, root_hash)
    fn delete(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let value = Self::fid_to_value(fid);
        let key = keyword.as_bytes().to_vec();

        // 从存储中删除 fid
        let delete_entire = {
            let mut storage = self.fid_storage.write().unwrap();
            if let Some(fids) = storage.get_mut(keyword) {
                fids.retain(|f| f != fid);
                let is_empty = fids.is_empty();
                if is_empty {
                    storage.remove(keyword);
                }
                is_empty
            } else {
                eprintln!(
                    "⚠️ AccTrie Delete: keyword='{}' not found in storage",
                    keyword
                );
                true
            }
        };

        // 从 AccTrie 中删除
        let mut trie = self.trie.write().unwrap();

        let proof = if delete_entire {
            // 删除整个叶子节点
            eprintln!(
                "🗑️ AccTrie Delete: keyword='{}', fid='{}' (removing entire key)",
                keyword, fid
            );
            match trie.delete(&key, None) {
                Ok(proof) => Self::serialize_deletion_proof(&proof),
                Err(e) => {
                    eprintln!("⚠️ AccTrie Delete: delete entire key failed: {:?}", e);
                    Vec::new()
                }
            }
        } else {
            // 只删除特定值
            eprintln!(
                "🗑️ AccTrie Delete: keyword='{}', fid='{}' (key still has values)",
                keyword, fid
            );
            match trie.delete(&key, Some(value)) {
                Ok(proof) => Self::serialize_deletion_proof(&proof),
                Err(e) => {
                    eprintln!("⚠️ AccTrie Delete: delete specific value failed: {:?}", e);
                    Vec::new()
                }
            }
        };

        // 获取根哈希
        drop(trie);
        let root_hash = self.get_root_hash();

        eprintln!(
            "🗑️ AccTrie Delete: post-delete root_hash={:02x?}...",
            &root_hash[..8.min(root_hash.len())]
        );

        (proof, root_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acctrie_ads_basic_operations() {
        let mut ads = AccTrieAds::new();

        // Test Add
        let (proof1, root1) = ads.add("rust", "file1");
        assert!(!proof1.is_empty());
        assert_eq!(root1.len(), 32);
        assert_eq!(proof1[0], 0x01); // InsertionProof标记

        let (proof2, root2) = ads.add("rust", "file2");
        assert!(!proof2.is_empty());
        assert_ne!(root1, root2); // Root should change

        // Test Query
        let (fids, proof) = ads.query("rust");
        assert_eq!(fids.len(), 2);
        assert!(fids.contains(&"file1".to_string()));
        assert!(fids.contains(&"file2".to_string()));
        assert!(!proof.is_empty());
        assert_eq!(proof[0], 0x03); // QueryProof标记

        // Test Delete
        let (proof3, root3) = ads.delete("rust", "file1");
        assert!(!proof3.is_empty());
        assert_eq!(proof3[0], 0x02); // DeletionProof标记
        assert_ne!(root2, root3);

        let (fids2, _) = ads.query("rust");
        assert_eq!(fids2.len(), 1);
        assert_eq!(fids2[0], "file2");

        // Delete last fid
        let (proof4, root4) = ads.delete("rust", "file2");
        assert!(!proof4.is_empty());
        assert_ne!(root3, root4);

        let (fids3, _) = ads.query("rust");
        assert_eq!(fids3.len(), 0);
    }

    #[test]
    fn test_acctrie_ads_multiple_keywords() {
        let mut ads = AccTrieAds::new();

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
    fn test_acctrie_proof_structure() {
        let mut ads = AccTrieAds::new();

        // Test InsertionProof structure
        let (proof, _) = ads.add("test", "value1");
        assert!(
            proof.len() > 100,
            "InsertionProof should contain complete data"
        );
        assert_eq!(proof[0], 0x01, "Should be InsertionProof");

        // Test QueryProof structure
        let (_, proof) = ads.query("test");
        assert!(proof.len() > 10, "QueryProof should contain complete data");
        assert_eq!(proof[0], 0x03, "Should be QueryProof");
        assert_eq!(proof[1], 1, "Should indicate existence");

        // Test DeletionProof structure
        let (proof, _) = ads.delete("test", "value1");
        assert!(
            proof.len() > 50,
            "DeletionProof should contain complete data"
        );
        assert_eq!(proof[0], 0x02, "Should be DeletionProof");
    }

    #[test]
    fn test_acctrie_root_hash_changes() {
        let mut ads = AccTrieAds::new();

        // 初始根哈希
        let root0 = ads.get_root_hash();

        // 添加后根哈希应该改变
        let (_, root1) = ads.add("key1", "val1");
        assert_ne!(root0, root1, "Root should change after insertion");

        // 再次添加
        let (_, root2) = ads.add("key2", "val2");
        assert_ne!(root1, root2, "Root should change after another insertion");

        // 删除后根哈希应该改变
        let (_, root3) = ads.delete("key1", "val1");
        assert_ne!(root2, root3, "Root should change after deletion");

        // 删除所有后根哈希应该接近初始状态（但可能不完全相同）
        let (_, root4) = ads.delete("key2", "val2");
        assert_ne!(root3, root4, "Root should change after final deletion");
    }

    #[test]
    fn test_acctrie_proof_types() {
        let mut ads = AccTrieAds::new();

        // 测试插入证明类型
        let (insert_proof, _) = ads.add("item", "data");
        assert_eq!(insert_proof[0], 0x01);

        // 测试查询证明类型（存在）
        let (_, query_proof) = ads.query("item");
        assert_eq!(query_proof[0], 0x03);
        assert_eq!(query_proof[1], 1); // 存在标记

        // 测试删除证明类型
        let (delete_proof, _) = ads.delete("item", "data");
        assert_eq!(delete_proof[0], 0x02);

        // 测试查询证明类型（不存在）
        let (fids, query_proof_not_exist) = ads.query("nonexistent");
        assert_eq!(fids.len(), 0);
        // 不存在时返回空证明
        assert_eq!(query_proof_not_exist.len(), 0);
    }
}
