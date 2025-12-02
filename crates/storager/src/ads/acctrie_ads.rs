//! AccTrie (Accumulator-based Trie) ADS 实现
//!
//! AccTrie 是一个结合密码学累加器的前缀树数据结构
//! 每个叶子节点维护一个值集合及其对应的密码学累加器，支持高效的成员证明和集合操作

use super::AdsOperations;
use common::RootHash;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// 引入 acctrie 库
use esa_rust::acctrie::{AccTrie, DeletionProof, InsertionProof, QueryResult, Node};

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

    /// 序列化插入证明为字节数组
    fn serialize_insertion_proof(proof: &InsertionProof) -> Vec<u8> {
        // 简单的序列化：将证明转换为 JSON
        // 实际应用中应该使用更高效的二进制序列化格式
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();

        // 序列化关键字段
        bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&proof.key);
        bytes.extend_from_slice(&proof.value.to_le_bytes());

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

        bytes
    }

    /// 序列化删除证明为字节数组
    fn serialize_deletion_proof(proof: &DeletionProof) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();

        // 序列化关键字段
        bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&proof.key);
        bytes.push(if proof.delete_entire_leaf { 1 } else { 0 });

        // 序列化累加器值
        let mut acc_old_bytes = Vec::new();
        proof
            .ln_acc_old
            .serialize_uncompressed(&mut acc_old_bytes)
            .unwrap();
        bytes.extend_from_slice(&(acc_old_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&acc_old_bytes);

        if let Some(acc_new) = proof.ln_acc_new {
            bytes.push(1); // 有新累加器值
            let mut acc_new_bytes = Vec::new();
            acc_new.serialize_uncompressed(&mut acc_new_bytes).unwrap();
            bytes.extend_from_slice(&(acc_new_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&acc_new_bytes);
        } else {
            bytes.push(0); // 无新累加器值
        }

        bytes
    }

    /// 序列化查询结果为字节数组
    fn serialize_query_result(result: &QueryResult) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();

        match result {
            QueryResult::Exists(proof) => {
                bytes.push(1); // 存在标记
                bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&proof.key);
                bytes.extend_from_slice(&proof.value.to_le_bytes());

                let mut acc_bytes = Vec::new();
                proof.ln_acc.serialize_uncompressed(&mut acc_bytes).unwrap();
                bytes.extend_from_slice(&(acc_bytes.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&acc_bytes);
            }
            QueryResult::NotExists(proof) => {
                bytes.push(0); // 不存在标记
                bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&proof.key);

                // 序列化前序和后序键
                if let Some(ref key_prev) = proof.key_prev {
                    bytes.push(1);
                    bytes.extend_from_slice(&(key_prev.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(key_prev);
                } else {
                    bytes.push(0);
                }

                if let Some(ref key_next) = proof.key_next {
                    bytes.push(1);
                    bytes.extend_from_slice(&(key_next.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(key_next);
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
            Ok(proof) => Self::serialize_insertion_proof(&proof),
            Err(_) => Vec::new(), // 插入失败，返回空证明
        };

        // 获取根哈希
        drop(trie);
        let root_hash = self.get_root_hash();

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
            return (Vec::new(), Vec::new());
        }

        // 查询第一个 fid 的证明（作为代表）
        let value = Self::fid_to_value(&fids[0]);
        let key = keyword.as_bytes().to_vec();

        let proof = {
            let trie = self.trie.read().unwrap();
            match trie.query(&key, value) {
                Ok(result) => Self::serialize_query_result(&result),
                Err(_) => Vec::new(),
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
                true
            }
        };

        // 从 AccTrie 中删除
        let mut trie = self.trie.write().unwrap();

        let proof = if delete_entire {
            // 删除整个叶子节点
            match trie.delete(&key, None) {
                Ok(proof) => Self::serialize_deletion_proof(&proof),
                Err(_) => Vec::new(),
            }
        } else {
            // 只删除特定值
            match trie.delete(&key, Some(value)) {
                Ok(proof) => Self::serialize_deletion_proof(&proof),
                Err(_) => Vec::new(),
            }
        };

        // 获取根哈希
        drop(trie);
        let root_hash = self.get_root_hash();

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

        let (proof2, root2) = ads.add("rust", "file2");
        assert!(!proof2.is_empty());
        assert_ne!(root1, root2); // Root should change

        // Test Query
        let (fids, proof) = ads.query("rust");
        assert_eq!(fids.len(), 2);
        assert!(fids.contains(&"file1".to_string()));
        assert!(fids.contains(&"file2".to_string()));
        assert!(!proof.is_empty());

        // Test Delete
        let (proof3, root3) = ads.delete("rust", "file1");
        assert!(!proof3.is_empty());
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
}
