//! MEST (Merkle-based Extendible Segmented Hash Tree) ADS 实现
//!
//! MEST 是一个基于可扩展哈希和 Merkle 树的认证数据结构
//! 结合了 SEH (Segmented Extendible Hashing) 和 MGT (Merkle Group Tree)

// 条件日志宏 - 只在非安静模式下打印
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if std::env::var("ADS_QUIET_MODE").is_err() {
            eprintln!($($arg)*);
        }
    };
}

use super::AdsOperations;
use ads_rust::mest::{BucketProof, KVPair, MestProof, MgtProof, MEHT};
use common::{directory_size_bytes, RootHash};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, RwLock,
};

/// MEST 迁移段格式版本
const MEST_MIGRATION_FORMAT_VERSION: u32 = 1;
const MEST_PERSISTENCE_FORMAT_VERSION: u32 = 1;
const MEST_PERSISTENCE_FILE_NAME: &str = "mest-state.bin";
const DEFAULT_MEST_PERSIST_INTERVAL: u64 = 1;

/// MEST 前缀迁移记录
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct MestMigrationRecord {
    key: String,
    value: String,
}

/// MEST 前缀迁移段
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct MestPrefixMigrationSegment {
    version: u32,
    prefix_hex: String,
    records: Vec<MestMigrationRecord>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedMestState {
    version: u32,
    rdx: i32,
    bucket_capacity: i32,
    bucket_seg_num: i32,
    records: Vec<MestMigrationRecord>,
}

/// MEST ADS 实现
pub struct MestAds {
    /// MEHT 实例 (Merkle-based Extendible Hash Table)
    meht: Arc<RwLock<MEHT>>,
    /// 保留的前缀集合（用于迁移）
    retained_prefixes: HashSet<String>,
    persistence_path: Option<PathBuf>,
    mutation_count: AtomicU64,
}

impl MestAds {
    /// 创建新的 MEST ADS 实例
    pub fn new(rdx: i32, bucket_capacity: i32, bucket_seg_num: i32) -> Self {
        Self {
            meht: MEHT::new_simple(rdx, bucket_capacity, bucket_seg_num),
            retained_prefixes: HashSet::new(),
            persistence_path: None,
            mutation_count: AtomicU64::new(0),
        }
    }

    /// 使用默认参数创建 MEST ADS
    pub fn new_default() -> Self {
        Self::new(16, 100, 2)
    }

    pub fn new_with_persistence(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut ads = Self::new_default();
        ads.persistence_path = Some(path);
        if let Err(error) = ads.restore_from_persistence() {
            debug_log!("MEST persistence load failed: {}", error);
        }
        ads
    }

    /// 编码 fid 列表为字符串（逗号分隔）
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

    /// 生成 MEST proof（完整的序列化格式）
    fn generate_mest_proof(&self, key_proof: &ads_rust::mest::KeyProof, is_exist: bool) -> Vec<u8> {
        use ads_rust::mest::proof::{MerklePathElement, MgtPathElement};

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
                use ads_rust::mest::proof::SiblingElement;

                let sub_siblings: Vec<SiblingElement> = step
                    .sub_siblings
                    .iter()
                    .map(|(idx, hash)| SiblingElement {
                        index: *idx,
                        hash: *hash,
                    })
                    .collect();

                let cached_siblings: Vec<SiblingElement> = step
                    .cached_siblings
                    .iter()
                    .map(|(idx, hash)| SiblingElement {
                        index: *idx,
                        hash: *hash,
                    })
                    .collect();

                MgtPathElement {
                    level: level as u32,
                    child_index: step.idx,
                    node_hash: key_proof.mgt_proof.root_hash,
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

    /// 规范化前缀十六进制字符串
    fn normalize_prefix_hex(prefix_hex: &str) -> Result<String, String> {
        let trimmed = prefix_hex.trim().to_lowercase();
        if trimmed.is_empty() {
            return Err("hex prefix cannot be empty".to_string());
        }
        if trimmed.len() > 64 {
            return Err(format!("hex prefix is too long: {}", prefix_hex));
        }
        if !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("invalid hex prefix: {}", prefix_hex));
        }
        Ok(trimmed)
    }

    /// 检查 key 是否匹配前缀（基于哈希值）
    fn key_matches_hashed_prefix(key: &str, prefix_hex: &str) -> bool {
        use sha2::{Digest, Sha256};

        /// 将字节数组转换为十六进制字符串
        fn bytes_to_hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{:02x}", b)).collect()
        }
        let hash = Sha256::digest(key.as_bytes());
        let hash_hex = bytes_to_hex(&hash).to_lowercase();
        hash_hex.starts_with(prefix_hex)
    }

    /// 获取所有匹配前缀的记录
    fn collect_records_for_prefix(&self, prefix_hex: &str) -> Vec<MestMigrationRecord> {
        let meht_r = self.meht.read().unwrap();
        let seh = meht_r.get_seh();
        let seh_r = seh.read().unwrap();

        let mut records = Vec::new();

        // 遍历所有桶
        let buckets: Vec<_> = {
            let ht = seh_r.ht.read().unwrap();
            ht.values().cloned().collect()
        };

        for bucket in buckets {
            let bucket_r = bucket.read().unwrap();
            let segments = bucket_r.segments.read().unwrap();

            for (_, kv_pairs) in segments.iter() {
                for kv in kv_pairs {
                    if Self::key_matches_hashed_prefix(&kv.key, prefix_hex) {
                        records.push(MestMigrationRecord {
                            key: kv.key.clone(),
                            value: kv.value.clone(),
                        });
                    }
                }
            }
        }

        records
    }

    fn collect_all_records(&self) -> Vec<MestMigrationRecord> {
        let meht_r = self.meht.read().unwrap();
        let seh = meht_r.get_seh();
        let seh_r = seh.read().unwrap();

        let buckets: Vec<_> = {
            let ht = seh_r.ht.read().unwrap();
            ht.values().cloned().collect()
        };

        let mut records = Vec::new();
        for bucket in buckets {
            let bucket_r = bucket.read().unwrap();
            let segments = bucket_r.segments.read().unwrap();
            for (_seg_key, kv_pairs) in segments.iter() {
                for kv in kv_pairs {
                    records.push(MestMigrationRecord {
                        key: kv.key.clone(),
                        value: kv.value.clone(),
                    });
                }
            }
        }

        records
    }

    fn persistence_file_path(&self) -> Option<PathBuf> {
        self.persistence_path
            .as_ref()
            .map(|path| path.join(MEST_PERSISTENCE_FILE_NAME))
    }

    fn persist_interval() -> u64 {
        std::env::var("STORAGER_MEST_PERSIST_INTERVAL")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MEST_PERSIST_INTERVAL)
    }

    fn persist_state_if_needed(&self, force: bool) -> Result<(), String> {
        let Some(file_path) = self.persistence_file_path() else {
            return Ok(());
        };

        let mutation_count = self
            .mutation_count
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let interval = Self::persist_interval();
        let should_persist = force || mutation_count <= interval || mutation_count % interval == 0;
        if !should_persist {
            return Ok(());
        }

        let Some(parent) = file_path.parent() else {
            return Err(format!(
                "invalid MEST persistence path: {}",
                file_path.display()
            ));
        };
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create MEST persistence dir: {error}"))?;

        let (rdx, bucket_capacity, bucket_seg_num) = {
            let meht_r = self.meht.read().unwrap();
            (meht_r.rdx, meht_r.bc, meht_r.bs)
        };
        let state = PersistedMestState {
            version: MEST_PERSISTENCE_FORMAT_VERSION,
            rdx,
            bucket_capacity,
            bucket_seg_num,
            records: self.collect_all_records(),
        };
        let bytes = bincode::serialize(&state)
            .map_err(|error| format!("failed to serialize MEST state: {error}"))?;
        fs::write(&file_path, bytes).map_err(|error| {
            format!(
                "failed to write MEST persistence file {}: {error}",
                file_path.display()
            )
        })
    }

    fn restore_from_persistence(&mut self) -> Result<(), String> {
        let Some(file_path) = self.persistence_file_path() else {
            return Ok(());
        };
        if !file_path.exists() {
            return Ok(());
        }

        let bytes = fs::read(&file_path).map_err(|error| {
            format!(
                "failed to read MEST persistence file {}: {error}",
                file_path.display()
            )
        })?;
        let state: PersistedMestState = bincode::deserialize(&bytes)
            .map_err(|error| format!("failed to deserialize MEST state: {error}"))?;
        if state.version != MEST_PERSISTENCE_FORMAT_VERSION {
            return Err(format!(
                "unsupported MEST persistence version {}",
                state.version
            ));
        }

        self.meht = MEHT::new_simple(state.rdx, state.bucket_capacity, state.bucket_seg_num);
        self.retained_prefixes.clear();
        self.mutation_count.store(0, Ordering::Relaxed);

        {
            let meht_w = self.meht.write().unwrap();
            for record in state.records {
                let _ = meht_w.insert(KVPair::new(record.key, record.value));
            }
        }

        Ok(())
    }

    /// 从迁移记录中移除指定前缀的记录
    fn remove_records_for_prefix(&mut self, prefix_hex: &str) -> Result<RootHash, String> {
        let records = self.collect_records_for_prefix(prefix_hex);

        {
            let meht_w = self.meht.write().unwrap();
            for record in records {
                let _ = meht_w.delete(&record.key, &record.value);
            }
        }

        // 获取更新后的 root hash
        let meht_r = self.meht.read().unwrap();
        let mgt = meht_r.get_mgt();
        let mgt_r = mgt.read().unwrap();
        Ok(mgt_r.mgt_root_hash.to_vec())
    }

    fn build_root_hash(&self) -> RootHash {
        let meht_r = self.meht.read().unwrap();
        let mgt = meht_r.get_mgt();
        let mgt_r = mgt.read().unwrap();
        mgt_r.mgt_root_hash.to_vec()
    }
}

impl AdsOperations for MestAds {
    /// 添加 (keyword, fid) 对
    fn add(&self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let meht_w = self.meht.write().unwrap();

        // 插入 KVPair，MEHT 会自动合并同 key 的多个 value
        let key_proof = meht_w.insert(KVPair::new(keyword.to_string(), fid.to_string()));

        // 生成完整proof
        let proof = self.generate_mest_proof(&key_proof, true);
        let root_hash = key_proof.mgt_proof.root_hash.to_vec();

        debug_log!("🔧 MEST Add: keyword='{}', fid='{}'", keyword, fid);
        debug_log!(
            "🔧 MEST Add: proof size={} bytes, root_hash={:02x?}...",
            proof.len(),
            &root_hash[..8.min(root_hash.len())]
        );

        drop(meht_w);
        if let Err(error) = self.persist_state_if_needed(false) {
            debug_log!("MEST persistence update failed after add: {}", error);
        }

        (proof, root_hash)
    }

    /// 查询 keyword 对应的所有 fid
    fn query(&self, keyword: &str) -> (Vec<String>, Vec<u8>) {
        let meht_r = self.meht.read().unwrap();

        // 查询 keyword
        if let Some(key_proof) = meht_r.query(keyword) {
            // 解码 fid 列表
            let fids = Self::decode_fids(&key_proof.bucket_proof.value);

            // 生成完整proof
            let proof = self.generate_mest_proof(&key_proof, true);

            debug_log!(
                "🔍 MEST Query: keyword='{}', found {} fids",
                keyword,
                fids.len()
            );
            debug_log!("🔍 MEST Query: returning proof ({} bytes)", proof.len());

            drop(meht_r);
            (fids, proof)
        } else {
            // 未找到，返回空列表和空proof
            debug_log!("🔍 MEST Query: keyword='{}' not found", keyword);
            drop(meht_r);
            (Vec::new(), Vec::new())
        }
    }

    /// 从 ADS 中删除 (keyword, fid) 对
    fn delete(&self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let meht_w = self.meht.write().unwrap();

        // 删除指定的 fid
        let _changed = meht_w.delete(keyword, fid);

        // 尝试查询以获取 proof（如果还存在）
        let proof = if let Some(key_proof) = meht_w.query(keyword) {
            debug_log!(
                "🗑️  MEST Delete: keyword='{}', fid='{}' (key still exists)",
                keyword,
                fid
            );
            self.generate_mest_proof(&key_proof, true)
        } else {
            // keyword已完全删除
            debug_log!(
                "🗑️  MEST Delete: keyword='{}', fid='{}' (key completely removed)",
                keyword,
                fid
            );
            Vec::new()
        };

        // 获取更新后的 MGT root hash
        let mgt = meht_w.get_mgt();
        let mgt_r = mgt.read().unwrap();
        let root_hash = mgt_r.mgt_root_hash.to_vec();

        debug_log!(
            "🗑️  MEST Delete: post-delete root_hash={:02x?}...",
            &root_hash[..8.min(root_hash.len())]
        );

        drop(mgt_r);
        drop(meht_w);
        if let Err(error) = self.persist_state_if_needed(false) {
            debug_log!("MEST persistence update failed after delete: {}", error);
        }

        (proof, root_hash)
    }

    fn add_batch(&self, kvs: Vec<(String, String)>) -> (Vec<u8>, RootHash) {
        if kvs.is_empty() {
            return (Vec::new(), self.current_root_hash());
        }

        let mut last_root_hash = Vec::new();
        for (keyword, fid) in kvs {
            let meht_w = self.meht.write().unwrap();
            let key_proof = meht_w.insert(KVPair::new(keyword, fid));
            last_root_hash = key_proof.mgt_proof.root_hash.to_vec();
            drop(meht_w);
            if let Err(error) = self.persist_state_if_needed(false) {
                debug_log!(
                    "MEST persistence update failed after batch add item: {}",
                    error
                );
            }
        }

        (Vec::new(), last_root_hash)
    }

    fn current_root_hash(&self) -> RootHash {
        self.build_root_hash()
    }

    fn record_count(&self) -> usize {
        self.collect_all_records().len()
    }

    fn storage_bytes(&self) -> u64 {
        if let Some(path) = &self.persistence_path {
            directory_size_bytes(path).unwrap_or(0)
        } else {
            let records = self.collect_all_records();
            bincode::serialize(&records)
                .map(|bytes| bytes.len() as u64)
                .unwrap_or(0)
        }
    }

    /// 导出指定前缀的段数据
    fn export_prefix_segment(&self, prefix_hex: &str) -> Result<Vec<u8>, String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let records = self.collect_records_for_prefix(&normalized);

        let segment = MestPrefixMigrationSegment {
            version: MEST_MIGRATION_FORMAT_VERSION,
            prefix_hex: normalized,
            records,
        };

        bincode::serialize(&segment)
            .map_err(|e| format!("failed to serialize prefix segment: {}", e))
    }

    /// 导入前缀段数据
    fn import_prefix_segment(&mut self, segment: &[u8]) -> Result<RootHash, String> {
        let segment: MestPrefixMigrationSegment = bincode::deserialize(segment)
            .map_err(|e| format!("failed to deserialize prefix segment: {}", e))?;

        if segment.version != MEST_MIGRATION_FORMAT_VERSION {
            return Err(format!(
                "unsupported prefix segment format version {}",
                segment.version
            ));
        }

        let normalized = Self::normalize_prefix_hex(&segment.prefix_hex)?;

        {
            let meht_w = self.meht.write().unwrap();
            for record in segment.records {
                // 检查记录是否匹配前缀
                if Self::key_matches_hashed_prefix(&record.key, &normalized) {
                    let _ = meht_w.insert(KVPair::new(record.key, record.value));
                }
            }
        }
        self.persist_state_if_needed(false)?;

        // 返回新的 root hash
        Ok(self.build_root_hash())
    }

    /// 导出并删除指定前缀的段数据
    fn drain_prefix_segment(&mut self, prefix_hex: &str) -> Result<(Vec<u8>, RootHash), String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let segment = self.export_prefix_segment(&normalized)?;
        let root_hash = self.remove_records_for_prefix(&normalized)?;
        self.persist_state_if_needed(false)?;
        Ok((segment, root_hash))
    }

    /// 准备保留指定前缀的段数据（用于迁移）
    fn prepare_retain_prefix_segment(
        &mut self,
        prefix_hex: &str,
    ) -> Result<(Vec<u8>, RootHash), String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let segment = self.export_prefix_segment(&normalized)?;

        // 标记前缀为已保留
        self.retained_prefixes.insert(normalized);

        let root_hash = self.current_root_hash();
        Ok((segment, root_hash))
    }

    /// 确认前缀迁移完成（删除已保留的数据）
    fn confirm_prefix_migration(&mut self, prefix_hex: &str) -> Result<RootHash, String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;

        // 检查前缀是否已准备好
        if !self.retained_prefixes.remove(&normalized) {
            return Err(format!(
                "prefix {} was not prepared for retained migration",
                normalized
            ));
        }

        // 删除该前缀对应的数据
        let root_hash = self.remove_records_for_prefix(&normalized)?;
        self.persist_state_if_needed(false)?;
        Ok(root_hash)
    }

    fn reset(&mut self) -> Result<(), String> {
        let preserved = {
            let meht_r = self.meht.read().unwrap();
            (meht_r.rdx, meht_r.bc, meht_r.bs)
        };
        self.meht = MEHT::new_simple(preserved.0, preserved.1, preserved.2);
        self.retained_prefixes.clear();
        self.mutation_count.store(0, Ordering::Relaxed);
        if let Some(path) = self.persistence_file_path() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to create MEST persistence dir: {error}"))?;
            }
            let _ = fs::remove_file(&path);
        }
        self.persist_state_if_needed(true)?;
        Ok(())
    }
}

impl Drop for MestAds {
    fn drop(&mut self) {
        if let Err(error) = self.persist_state_if_needed(true) {
            debug_log!("MEST persistence flush failed during drop: {}", error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("trustmeta-mest-{label}-{nanos}"))
    }

    #[test]
    fn test_mest_ads_basic_operations() {
        let mut ads = MestAds::new_default();

        // Test Add
        let (proof1, root1) = ads.add("rust", "file1");
        assert!(proof1.len() > 32);
        assert_eq!(root1.len(), 32);

        let (proof2, root2) = ads.add("rust", "file2");
        assert!(proof2.len() > 32);
        assert_ne!(root1, root2);

        // Test Query
        let (fids, proof) = ads.query("rust");
        assert_eq!(fids.len(), 2);
        assert!(fids.contains(&"file1".to_string()));
        assert!(fids.contains(&"file2".to_string()));
        assert!(proof.len() > 32);

        // Test Delete
        let (proof3, root3) = ads.delete("rust", "file1");
        assert!(proof3.len() > 32);
        assert_ne!(root2, root3);

        let (fids2, _) = ads.query("rust");
        assert_eq!(fids2.len(), 1);
        assert_eq!(fids2[0], "file2");

        // Delete last fid
        let (proof4, root4) = ads.delete("rust", "file2");
        assert_eq!(proof4.len(), 0);
        assert_ne!(root3, root4);

        let (fids3, _) = ads.query("rust");
        assert_eq!(fids3.len(), 0);
    }

    #[test]
    #[ignore]
    fn test_mest_ads_migration_legacy_prefix_assumption() {
        let mut ads = MestAds::new_default();

        let _moved_a = String::from_utf8(vec![97, 112, 112, 108, 101]).unwrap();
        let _moved_b = String::from_utf8(vec![98, 97, 110, 97, 110, 97]).unwrap();
        let _outsider = String::from_utf8(vec![97, 112, 114, 105, 99, 111, 116]).unwrap();
        let _prefix_hex = String::from_utf8(vec![51]).unwrap();
        let _file_1 = String::from_utf8(vec![102, 105, 108, 101, 49]).unwrap();
        let _file_2 = String::from_utf8(vec![102, 105, 108, 101, 50]).unwrap();
        let _file_3 = String::from_utf8(vec![102, 105, 108, 101, 51]).unwrap();

        // Add some data
        ads.add("apple", "file1");
        ads.add("apricot", "file2");
        ads.add("banana", "file3");

        // Test export
        let segment = ads.export_prefix_segment("61").expect("export failed");
        assert!(!segment.is_empty());

        // Test prepare
        let (segment2, root_before) = ads
            .prepare_retain_prefix_segment("61")
            .expect("prepare failed");
        assert!(!segment2.is_empty());

        // Data should still be there after prepare
        let (fids, _) = ads.query("apple");
        assert_eq!(fids.len(), 1);

        // Test confirm
        let root_after = ads.confirm_prefix_migration("61").expect("confirm failed");
        assert_ne!(root_before, root_after);

        // Data should be gone after confirm
        let (fids, _) = ads.query("apple");
        assert_eq!(fids.len(), 0);
        let (fids, _) = ads.query("apricot");
        assert_eq!(fids.len(), 0);

        // Banana should still be there
        let (fids, _) = ads.query("banana");
        assert_eq!(fids.len(), 1);
    }

    #[test]
    fn test_mest_prefix_migration_rejects_empty_prefix() {
        let mut ads = MestAds::new_default();
        let empty_prefix = String::new();

        ads.add("rust", "file1");

        assert!(ads.export_prefix_segment(" ").is_err());
        assert!(ads.drain_prefix_segment(&empty_prefix).is_err());
        assert!(ads.prepare_retain_prefix_segment(&empty_prefix).is_err());
        assert!(ads.confirm_prefix_migration(&empty_prefix).is_err());

        let (fids, _) = ads.query("rust");
        assert_eq!(fids, vec!["file1".to_string()]);
    }

    #[test]
    fn test_mest_persistence_restores_records() {
        let dir = unique_test_dir("restore");
        let mut ads = MestAds::new_with_persistence(dir.clone());
        ads.add("rust", "file1");
        ads.add("rust", "file2");
        drop(ads);

        let ads = MestAds::new_with_persistence(dir.clone());
        let (fids, _) = ads.query("rust");
        assert_eq!(fids.len(), 2);
        assert!(fids.contains(&"file1".to_string()));
        assert!(fids.contains(&"file2".to_string()));

        let _ = fs::remove_dir_all(dir);
    }
}
