//! Merkle Patricia Trie (MPT) ADS Implementation
//!
//! 使用以太坊风格的 Merkle Patricia Trie 作为认证数据结构
//! 支持高效的键值存储和成员资格证明

use super::AdsOperations;
use common::RootHash;
use ads_rust::mpt::{node::Database, KVPair, MPTError, MPT};
use std::collections::HashMap;
use std::sync::RwLock;

// 条件日志宏 - 只在非安静模式下打印
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if std::env::var("ADS_QUIET_MODE").is_err() {
            eprintln!($($arg)*);
        }
    };
}

/// 简单的内存数据库实现
#[derive(Clone)]
struct MemoryDb {
    data: HashMap<Vec<u8>, Vec<u8>>,
}

impl MemoryDb {
    fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }
}

impl Database for MemoryDb {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, MPTError> {
        Ok(self.data.get(key).cloned())
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), MPTError> {
        self.data.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), MPTError> {
        self.data.remove(key);
        Ok(())
    }
}

/// MPT ADS 实现
pub struct MptAds {
    /// 单个 MPT 实例存储所有关键字
    /// 使用 RwLock 允许内部可变性
    /// (MPT, Database)
    state: RwLock<(MPT, MemoryDb)>,
}

impl MptAds {
    pub fn new() -> Self {
        MptAds {
            state: RwLock::new((MPT::new(None), MemoryDb::new())),
        }
    }

    /// 将 fid 列表编码为字符串
    fn encode_fids(fids: &[String]) -> String {
        fids.join(",")
    }

    /// 从字符串解码 fid 列表
    fn decode_fids(data: &str) -> Vec<String> {
        if data.is_empty() {
            Vec::new()
        } else {
            data.split(',').map(|s| s.to_string()).collect()
        }
    }
}

impl Default for MptAds {
    fn default() -> Self {
        Self::new()
    }
}

impl AdsOperations for MptAds {
    fn add(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let mut state = self.state.write().unwrap();
        let (ref mut trie, ref mut db) = *state;

        // 获取当前 FIDs
        let mut fids = match trie.query_by_key(keyword, db) {
            Ok((val, _)) => Self::decode_fids(&val),
            Err(_) => Vec::new(),
        };

        // 添加 fid
        if !fids.contains(&fid.to_string()) {
            fids.push(fid.to_string());
        }

        // 更新 MPT
        let value = Self::encode_fids(&fids);
        let kv = KVPair::new(keyword.to_string(), value.clone());

        let _ = trie.insert(kv, db, true, false);

        // 获取根哈希
        let root_hash = trie.root_hash.to_vec();

        debug_log!(
            "🔧 Storager Add: keyword='{}', inserted_value='{}' (len={})",
            keyword,
            value,
            value.len()
        );
        debug_log!(
            "🔧 Storager Add: root_hash after insert: {:02x?}...",
            &root_hash[..8]
        );

        //生成完整证明
        let proof = match trie.query_by_key(keyword, db) {
            Ok((query_value, mpt_proof)) => {
                debug_log!(
                    "🔧 Storager Add: query returned value='{}' (len={})",
                    query_value,
                    query_value.len()
                );
                debug_log!(
                    "🔧 Storager Add: proof is_exist={}, levels={}",
                    mpt_proof.get_is_exist(),
                    mpt_proof.get_levels()
                );

                // Verify proof locally before sending to manager
                let verify_result = trie.verify_query_result(&query_value, &mpt_proof);
                debug_log!(
                    "🔧 Storager Add: local verify_query_result = {}",
                    verify_result
                );

                bincode::serialize(&mpt_proof).unwrap_or_else(|_| root_hash.clone())
            }
            Err(e) => {
                debug_log!("❌ Storager Add: query_by_key failed after insert: {}", e);
                root_hash.clone()
            }
        };

        (proof, root_hash)
    }

    fn query(&self, keyword: &str) -> (Vec<String>, Vec<u8>) {
        // MPT的query_by_key需要可变引用database，因此仍然需要写锁
        // 这是MPT实现的限制，无法改用读锁
        let mut state = self.state.write().unwrap();
        let (ref mut trie, ref mut db) = *state;

        match trie.query_by_key(keyword, db) {
            Ok((value, mpt_proof)) => {
                let fids = Self::decode_fids(&value);

                debug_log!(
                    "🔍 MPT Query: keyword='{}', found {} fids",
                    keyword,
                    fids.len()
                );

                // 序列化完整的 MPT Proof
                match bincode::serialize(&mpt_proof) {
                    Ok(proof_bytes) => {
                        debug_log!(
                            "🔍 MPT Query: returning proof ({} bytes)",
                            proof_bytes.len()
                        );
                        (fids, proof_bytes)
                    }
                    Err(_) => {
                        // 降级到简单证明
                        debug_log!("⚠️ MPT Query: proof serialization failed, returning root hash");
                        (fids, trie.root_hash.to_vec())
                    }
                }
            }
            Err(_) => {
                // 关键字不存在，返回空列表和空proof（与MEST/AccTrie对齐）
                debug_log!("🔍 MPT Query: keyword='{}' not found", keyword);
                (vec![], Vec::new())
            }
        }
    }

    fn delete(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        // 获取state的写锁(整个操作在一个锁作用域内完成)
        let mut state = match self.state.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                debug_log!("⚠️ MPT Delete: recovering from poisoned lock");
                poisoned.into_inner()
            }
        };
        let (ref mut trie, ref mut db) = *state;

        // 获取当前 FIDs
        let mut fids = match trie.query_by_key(keyword, db) {
            Ok((val, _)) => Self::decode_fids(&val),
            Err(_) => Vec::new(),
        };

        if fids.is_empty() {
            // 关键字不存在,返回当前状态的证明（非存在性证明）
            let root_hash = trie.root_hash.to_vec();
            debug_log!("⚠️ MPT Delete: keyword '{}' not found", keyword);

            // 生成非存在性证明
            match trie.query_by_key(keyword, db) {
                Ok((_, mpt_proof)) => match bincode::serialize(&mpt_proof) {
                    Ok(proof_bytes) => return (proof_bytes, root_hash),
                    Err(_) => return (root_hash.clone(), root_hash),
                },
                Err(_) => return (root_hash.clone(), root_hash),
            }
        }

        // 移除 fid
        fids.retain(|f| f != fid);

        if fids.is_empty() {
            // 如果列表为空，从 MPT 中删除整个键
            if let Err(e) = trie.delete(keyword, db) {
                debug_log!("❌ MPT Delete: trie.delete failed: {}", e);
            }
        } else {
            // 更新 MPT
            let value = Self::encode_fids(&fids);
            let kv = KVPair::new(keyword.to_string(), value);

            if let Err(e) = trie.insert(kv, db, true, false) {
                debug_log!("❌ MPT Delete: trie.insert failed: {}", e);
            }
        }

        // Generate POST-delete proof (consistent with Manager's expectation)
        let post_delete_root_hash = trie.root_hash.to_vec();
        let post_delete_proof = match trie.query_by_key(keyword, db) {
            Ok((_, mpt_proof)) => {
                debug_log!(
                    "🔧 MPT Delete: generated post-delete proof, is_exist={}",
                    mpt_proof.get_is_exist()
                );
                bincode::serialize(&mpt_proof).unwrap_or_else(|_| Vec::new())
            }
            Err(e) => {
                debug_log!("❌ MPT Delete: failed to get post-delete proof: {}", e);
                Vec::new()
            }
        };

        debug_log!(
            "✅ MPT Delete: returning post-delete proof ({} bytes) + post-delete root_hash",
            post_delete_proof.len()
        );
        (post_delete_proof, post_delete_root_hash)
    }
}
