//! Merkle Patricia Trie (MPT) ADS Implementation
//!
//! 使用以太坊风格的 Merkle Patricia Trie 作为认证数据结构
//! 支持内存模式和 LevelDB 持久化模式

use super::AdsOperations;
use ads_rust::acctrie::AccTrie;
use ads_rust::mpt::{
    node::Database, FullNode, KVPair, LevelDbDatabase, MPTError, NodeCache, ShortNode, MPT,
};
use common::RootHash;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

const MPT_MIGRATION_FORMAT_VERSION: u32 = 1;
const DEFAULT_MPT_SHORT_NODE_CACHE_CAPACITY: usize = 1024;
const DEFAULT_MPT_FULL_NODE_CACHE_CAPACITY: usize = 1024;

macro_rules! debug_log {
    ($($arg:tt)*) => {
        if std::env::var("ADS_QUIET_MODE").is_err() {
            eprintln!($($arg)*);
        }
    };
}

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

enum MptDb {
    Memory(MemoryDb),
    LevelDb(LevelDbDatabase),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct MptPersistedRecord {
    key: String,
    value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MptPrefixMigrationSegment {
    version: u32,
    prefix_hex: String,
    records: Vec<MptPersistedRecord>,
}

impl Database for MptDb {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, MPTError> {
        match self {
            MptDb::Memory(db) => db.get(key),
            MptDb::LevelDb(db) => db.get(key),
        }
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), MPTError> {
        match self {
            MptDb::Memory(db) => db.put(key, value),
            MptDb::LevelDb(db) => db.put(key, value),
        }
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), MPTError> {
        match self {
            MptDb::Memory(db) => db.delete(key),
            MptDb::LevelDb(db) => db.delete(key),
        }
    }
}

struct MptState {
    trie: MPT,
    db: MptDb,
    retained_prefixes: HashSet<String>,
    mutation_count: u64,
}

impl MptState {
    fn new(trie: MPT, db: MptDb) -> Self {
        Self {
            trie,
            db,
            retained_prefixes: HashSet::new(),
            mutation_count: 0,
        }
    }
}

pub struct MptAds {
    state: Mutex<MptState>,
    persistence_path: Option<PathBuf>,
}

impl MptAds {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MptState::new(
                MPT::new(None),
                MptDb::Memory(MemoryDb::new()),
            )),
            persistence_path: None,
        }
    }

    pub fn new_with_persistence(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut db = LevelDbDatabase::open(&path).unwrap_or_else(|error| {
            panic!(
                "failed to open MPT LevelDB at {}: {}",
                path.display(),
                error
            )
        });
        let cache = Some(NodeCache::new(
            DEFAULT_MPT_SHORT_NODE_CACHE_CAPACITY,
            DEFAULT_MPT_FULL_NODE_CACHE_CAPACITY,
        ));
        let trie = MPT::restore_from_db(&mut db, cache).unwrap_or_else(|error| {
            panic!(
                "failed to restore persistent MPT from {}: {}",
                path.display(),
                error
            )
        });

        debug_log!(
            "MPT persistence loaded from '{}' with root {:02x?}...",
            path.display(),
            &trie.root_hash[..8]
        );

        Self {
            state: Mutex::new(MptState::new(trie, MptDb::LevelDb(db))),
            persistence_path: Some(path),
        }
    }

    fn encode_fids(fids: &[String]) -> String {
        fids.join(",")
    }

    fn decode_fids(data: &str) -> Vec<String> {
        if data.is_empty() {
            Vec::new()
        } else {
            data.split(',').map(|s| s.to_string()).collect()
        }
    }

    fn normalize_prefix_hex(prefix_hex: &str) -> Result<String, String> {
        let normalized = prefix_hex.trim().to_lowercase();
        if normalized.is_empty() {
            return Err("hex prefix cannot be empty".to_string());
        }
        if normalized.len() > 64 {
            return Err(format!("hex prefix is too long: {prefix_hex}"));
        }
        if !normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(format!("invalid hex prefix: {prefix_hex}"));
        }
        Ok(normalized)
    }

    fn persist_state(state: &mut MptState) {
        state.mutation_count = state.mutation_count.saturating_add(1);
        if let MptDb::LevelDb(_) = state.db {
            let persist_interval = std::env::var("STORAGER_MPT_PERSIST_INTERVAL")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(32);
            let should_full_persist =
                state.mutation_count <= persist_interval
                    || state.mutation_count % persist_interval == 0;
            let result = if should_full_persist {
                state.trie.persist_to_db(&mut state.db)
            } else {
                state.trie.persist_metadata_only(&mut state.db)
            };
            if let Err(error) = result {
                debug_log!("MPT persistence update failed: {}", error);
            }
        }
    }

    fn collect_all_records(state: &mut MptState) -> Result<Vec<MptPersistedRecord>, String> {
        let Some(root) = state
            .trie
            .get_root(&mut state.db)
            .map_err(|error| error.to_string())?
        else {
            return Ok(Vec::new());
        };

        let mut records = Vec::new();
        Self::collect_full_node_records(&root, &mut state.db, &mut Vec::new(), &mut records)
            .map_err(|error| error.to_string())?;
        records.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(records)
    }

    fn collect_full_node_records(
        full_node: &Arc<RwLock<FullNode>>,
        db: &mut dyn Database,
        path: &mut Vec<u8>,
        records: &mut Vec<MptPersistedRecord>,
    ) -> Result<(), MPTError> {
        let (children, value): (Vec<(usize, Arc<RwLock<ShortNode>>)>, Option<Vec<u8>>) = {
            let guard = full_node.read().map_err(|_| {
                MPTError::LockError("Failed to read FullNode during export".to_string())
            })?;
            let children = guard
                .children
                .iter()
                .enumerate()
                .filter_map(|(index, child)| child.clone().map(|child| (index, child)))
                .collect();
            (children, guard.value.clone())
        };

        if let Some(value) = value {
            let key = ads_rust::mpt::utils::hex_path_to_key(path);
            let value = String::from_utf8_lossy(&value).to_string();
            records.push(MptPersistedRecord { key, value });
        }

        for (index, child) in children {
            path.push(index as u8);
            Self::collect_short_node_records(&child, db, path, records)?;
            path.pop();
        }

        Ok(())
    }

    fn collect_short_node_records(
        short_node: &Arc<RwLock<ShortNode>>,
        db: &mut dyn Database,
        path: &mut Vec<u8>,
        records: &mut Vec<MptPersistedRecord>,
    ) -> Result<(), MPTError> {
        let (is_leaf, suffix, value, next_node, next_node_hash) = {
            let guard = short_node.read().map_err(|_| {
                MPTError::LockError("Failed to read ShortNode during export".to_string())
            })?;
            (
                guard.is_leaf,
                guard.suffix.clone(),
                guard.value.clone(),
                guard.next_node.clone(),
                guard.next_node_hash,
            )
        };

        let base_len = path.len();
        for ch in suffix.chars() {
            let digit = ch
                .to_digit(16)
                .ok_or_else(|| MPTError::InvalidData(format!("invalid suffix nibble: {suffix}")))?;
            path.push(digit as u8);
        }

        if is_leaf {
            if let Some(value) = value {
                let key = ads_rust::mpt::utils::hex_path_to_key(path);
                let value = String::from_utf8_lossy(&value).to_string();
                records.push(MptPersistedRecord { key, value });
            }
        } else {
            let next = if let Some(next_node) = next_node {
                next_node
            } else if next_node_hash != [0u8; 32] {
                let Some(data) = db.get(&next_node_hash)? else {
                    path.truncate(base_len);
                    return Err(MPTError::NodeNotFound);
                };
                Arc::new(RwLock::new(FullNode::deserialize(&data)?))
            } else {
                path.truncate(base_len);
                return Ok(());
            };

            Self::collect_full_node_records(&next, db, path, records)?;
        }

        path.truncate(base_len);
        Ok(())
    }

    fn records_for_prefix(
        state: &mut MptState,
        prefix_hex: &str,
    ) -> Result<Vec<MptPersistedRecord>, String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        Ok(Self::collect_all_records(state)?
            .into_iter()
            .filter(|record| {
                AccTrie::hashed_key_hex(record.key.as_bytes()).starts_with(&normalized)
            })
            .collect())
    }

    fn apply_import_records(
        state: &mut MptState,
        records: Vec<MptPersistedRecord>,
    ) -> Result<RootHash, String> {
        for record in records {
            let kv = KVPair::new(record.key, record.value);
            state
                .trie
                .insert(kv, &mut state.db, true, false)
                .map_err(|error| error.to_string())?;
        }
        Self::persist_state(state);
        Ok(state.trie.root_hash.to_vec())
    }

    fn remove_prefix_records(state: &mut MptState, prefix_hex: &str) -> Result<RootHash, String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let records = Self::records_for_prefix(state, &normalized)?;
        for record in records {
            state
                .trie
                .delete(&record.key, &mut state.db)
                .map_err(|error| error.to_string())?;
        }
        Self::persist_state(state);
        Ok(state.trie.root_hash.to_vec())
    }
}

impl Default for MptAds {
    fn default() -> Self {
        Self::new()
    }
}

impl AdsOperations for MptAds {
    fn add(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let mut state = self.state.lock().unwrap();
        {
            let MptState { trie, db, .. } = &mut *state;
            let mut fids = match trie.query_by_key(keyword, db) {
                Ok((val, _)) => Self::decode_fids(&val),
                Err(_) => Vec::new(),
            };

            if !fids.contains(&fid.to_string()) {
                fids.push(fid.to_string());
            }

            let value = Self::encode_fids(&fids);
            let kv = KVPair::new(keyword.to_string(), value.clone());
            let _ = trie.insert(kv, db, true, false);
        };

        Self::persist_state(&mut state);

        let (root_hash, proof) = {
            let MptState { trie, db, .. } = &mut *state;
            let root_hash = trie.root_hash.to_vec();
            let proof = match trie.query_by_key(keyword, db) {
                Ok((query_value, mpt_proof)) => {
                    let verify_result = trie.verify_query_result(&query_value, &mpt_proof);
                    debug_log!("MPT Add: local verify_query_result = {}", verify_result);
                    bincode::serialize(&mpt_proof).unwrap_or_else(|_| root_hash.clone())
                }
                Err(e) => {
                    debug_log!("MPT Add: query_by_key failed after insert: {}", e);
                    root_hash.clone()
                }
            };
            (root_hash, proof)
        };

        (proof, root_hash)
    }

    fn add_batch(&mut self, kvs: Vec<(String, String)>) -> (Vec<u8>, RootHash) {
        if kvs.is_empty() {
            return (Vec::new(), self.current_root_hash());
        }

        let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
        for (keyword, fid) in kvs {
            let entry = grouped.entry(keyword).or_default();
            if !entry.contains(&fid) {
                entry.push(fid);
            }
        }

        let mut state = self.state.lock().unwrap();
        let existing = Self::collect_all_records(&mut state).unwrap_or_default();
        let mut existing_by_key = HashMap::with_capacity(grouped.len());
        for record in existing {
            if grouped.contains_key(&record.key) {
                existing_by_key.insert(record.key, Self::decode_fids(&record.value));
            }
        }

        {
            let MptState { trie, db, .. } = &mut *state;
            for (keyword, incoming_fids) in grouped {
                let mut fids = existing_by_key.remove(&keyword).unwrap_or_default();
                for fid in incoming_fids {
                    if !fids.contains(&fid) {
                        fids.push(fid);
                    }
                }
                let value = Self::encode_fids(&fids);
                let kv = KVPair::new(keyword, value);
                let _ = trie.insert(kv, db, true, false);
            }
        }

        Self::persist_state(&mut state);
        (Vec::new(), state.trie.root_hash.to_vec())
    }

    fn query(&self, keyword: &str) -> (Vec<String>, Vec<u8>) {
        let mut state = self.state.lock().unwrap();
        let MptState { trie, db, .. } = &mut *state;

        match trie.query_by_key(keyword, db) {
            Ok((value, mpt_proof)) => {
                let fids = Self::decode_fids(&value);
                match bincode::serialize(&mpt_proof) {
                    Ok(proof_bytes) => (fids, proof_bytes),
                    Err(_) => (fids, trie.root_hash.to_vec()),
                }
            }
            Err(_) => (vec![], Vec::new()),
        }
    }

    fn delete(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                debug_log!("MPT Delete: recovering from poisoned lock");
                poisoned.into_inner()
            }
        };
        {
            let MptState { trie, db, .. } = &mut *state;

            let mut fids = match trie.query_by_key(keyword, db) {
                Ok((val, _)) => Self::decode_fids(&val),
                Err(_) => Vec::new(),
            };

            if fids.is_empty() {
                let root_hash = trie.root_hash.to_vec();
                match trie.query_by_key(keyword, db) {
                    Ok((_, mpt_proof)) => match bincode::serialize(&mpt_proof) {
                        Ok(proof_bytes) => return (proof_bytes, root_hash),
                        Err(_) => return (root_hash.clone(), root_hash),
                    },
                    Err(_) => return (root_hash.clone(), root_hash),
                }
            }

            fids.retain(|f| f != fid);

            if fids.is_empty() {
                if let Err(e) = trie.delete(keyword, db) {
                    debug_log!("MPT Delete: trie.delete failed: {}", e);
                }
            } else {
                let value = Self::encode_fids(&fids);
                let kv = KVPair::new(keyword.to_string(), value);
                if let Err(e) = trie.insert(kv, db, true, false) {
                    debug_log!("MPT Delete: trie.insert failed: {}", e);
                }
            }
        }

        Self::persist_state(&mut state);

        let (post_delete_root_hash, post_delete_proof) = {
            let MptState { trie, db, .. } = &mut *state;
            let root_hash = trie.root_hash.to_vec();
            let proof = match trie.query_by_key(keyword, db) {
                Ok((_, mpt_proof)) => bincode::serialize(&mpt_proof).unwrap_or_else(|_| Vec::new()),
                Err(e) => {
                    debug_log!("MPT Delete: failed to get post-delete proof: {}", e);
                    Vec::new()
                }
            };
            (root_hash, proof)
        };

        (post_delete_proof, post_delete_root_hash)
    }

    fn current_root_hash(&self) -> RootHash {
        let state = self.state.lock().unwrap();
        state.trie.root_hash.to_vec()
    }

    fn record_count(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        Self::collect_all_records(&mut state)
            .map(|records| records.len())
            .unwrap_or(0)
    }

    fn storage_bytes(&self) -> u64 {
        let mut state = self.state.lock().unwrap();
        Self::collect_all_records(&mut state)
            .ok()
            .and_then(|records| bincode::serialize(&records).ok())
            .map(|bytes| bytes.len() as u64)
            .unwrap_or(0)
    }

    fn export_prefix_segment(&self, prefix_hex: &str) -> Result<Vec<u8>, String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let mut state = self.state.lock().unwrap();
        let records = Self::records_for_prefix(&mut state, &normalized)?;
        let segment = MptPrefixMigrationSegment {
            version: MPT_MIGRATION_FORMAT_VERSION,
            prefix_hex: normalized,
            records,
        };
        bincode::serialize(&segment)
            .map_err(|error| format!("failed to serialize prefix segment: {error}"))
    }

    fn import_prefix_segment(&mut self, segment: &[u8]) -> Result<RootHash, String> {
        let segment: MptPrefixMigrationSegment = bincode::deserialize(segment)
            .map_err(|error| format!("failed to deserialize prefix segment: {error}"))?;
        if segment.version != MPT_MIGRATION_FORMAT_VERSION {
            return Err(format!(
                "unsupported prefix segment format version {}",
                segment.version
            ));
        }

        let normalized = Self::normalize_prefix_hex(&segment.prefix_hex)?;
        let filtered_records = segment
            .records
            .into_iter()
            .filter(|record| {
                AccTrie::hashed_key_hex(record.key.as_bytes()).starts_with(&normalized)
            })
            .collect::<Vec<_>>();

        let mut state = self.state.lock().unwrap();
        Self::apply_import_records(&mut state, filtered_records)
    }

    fn drain_prefix_segment(&mut self, prefix_hex: &str) -> Result<(Vec<u8>, RootHash), String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let segment = self.export_prefix_segment(&normalized)?;
        let mut state = self.state.lock().unwrap();
        let root_hash = Self::remove_prefix_records(&mut state, &normalized)?;
        Ok((segment, root_hash))
    }

    fn prepare_retain_prefix_segment(
        &mut self,
        prefix_hex: &str,
    ) -> Result<(Vec<u8>, RootHash), String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let segment = self.export_prefix_segment(&normalized)?;
        let mut state = self.state.lock().unwrap();
        state.retained_prefixes.insert(normalized);
        Ok((segment, state.trie.root_hash.to_vec()))
    }

    fn confirm_prefix_migration(&mut self, prefix_hex: &str) -> Result<RootHash, String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let mut state = self.state.lock().unwrap();
        if !state.retained_prefixes.remove(&normalized) {
            return Err(format!(
                "prefix {normalized} was not prepared for retained migration"
            ));
        }
        Self::remove_prefix_records(&mut state, &normalized)
    }

    fn reset(&mut self) -> Result<(), String> {
        if let Some(path) = &self.persistence_path {
            let _ = fs::remove_dir_all(path);
            let mut db = LevelDbDatabase::open(path)
                .map_err(|error| format!("failed to reopen MPT persistence: {error}"))?;
            let cache = Some(NodeCache::new(
                DEFAULT_MPT_SHORT_NODE_CACHE_CAPACITY,
                DEFAULT_MPT_FULL_NODE_CACHE_CAPACITY,
            ));
            let trie = MPT::restore_from_db(&mut db, cache)
                .map_err(|error| format!("failed to reset MPT persistence: {error}"))?;
            let mut state = self.state.lock().unwrap();
            state.trie = trie;
            state.db = MptDb::LevelDb(db);
        } else {
            let mut state = self.state.lock().unwrap();
            state.trie = MPT::new(None);
            state.db = MptDb::Memory(MemoryDb::new());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mpt-ads-{tag}-{nanos}"))
    }

    fn keywords_sharing_hashed_prefix(prefix_len: usize, count: usize) -> (String, Vec<String>) {
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for index in 0..20_000usize {
            let keyword = format!("mpt-migration-key-{index}");
            let digest = AccTrie::hashed_key_hex(keyword.as_bytes());
            let prefix = digest[..prefix_len].to_string();
            let group = groups.entry(prefix.clone()).or_default();
            group.push(keyword);
            if group.len() >= count {
                return (prefix, group.clone());
            }
        }

        panic!("failed to find enough keywords sharing a hashed prefix");
    }

    #[test]
    fn test_mpt_ads_persistence_restores_records() {
        let dir = unique_temp_dir("restore");
        let mut ads = MptAds::new_with_persistence(dir.clone());

        {
            let state = ads.state.lock().unwrap();
            assert!(state.trie.cache.is_some());
        }

        ads.add("rust", "file1");
        ads.add("storage", "file2");

        drop(ads);

        let ads = MptAds::new_with_persistence(dir.clone());

        {
            let state = ads.state.lock().unwrap();
            assert!(state.trie.cache.is_some());
        }

        let (rust_fids, _) = ads.query("rust");
        let (storage_fids, _) = ads.query("storage");

        assert_eq!(rust_fids, vec!["file1".to_string()]);
        assert_eq!(storage_fids, vec!["file2".to_string()]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_mpt_prefix_segment_export_import() {
        let source_dir = unique_temp_dir("export-source");
        let target_dir = unique_temp_dir("export-target");
        let mut source = MptAds::new_with_persistence(source_dir.clone());
        let mut target = MptAds::new_with_persistence(target_dir.clone());

        let (prefix_hex, keywords) = keywords_sharing_hashed_prefix(2, 2);
        let moved_a = &keywords[0];
        let moved_b = &keywords[1];
        let outsider = (0..20_000usize)
            .map(|index| format!("mpt-outsider-{index}"))
            .find(|keyword| !AccTrie::hashed_key_hex(keyword.as_bytes()).starts_with(&prefix_hex))
            .expect("outsider keyword");

        source.add(moved_a, "fa");
        source.add(moved_a, "fb");
        source.add(moved_b, "fc");
        source.add(&outsider, "fd");

        let segment = source
            .export_prefix_segment(&prefix_hex)
            .expect("export prefix segment");
        target
            .import_prefix_segment(&segment)
            .expect("import prefix segment");

        let (moved_a_on_target, _) = target.query(moved_a);
        let (moved_b_on_target, _) = target.query(moved_b);
        let (outsider_on_target, _) = target.query(&outsider);

        assert_eq!(moved_a_on_target.len(), 2);
        assert!(moved_a_on_target.contains(&"fa".to_string()));
        assert!(moved_a_on_target.contains(&"fb".to_string()));
        assert_eq!(moved_b_on_target, vec!["fc".to_string()]);
        assert!(outsider_on_target.is_empty());

        let _ = fs::remove_dir_all(source_dir);
        let _ = fs::remove_dir_all(target_dir);
    }

    #[test]
    fn test_mpt_prefix_segment_prepare_import_confirm() {
        let source_dir = unique_temp_dir("retain-source");
        let target_dir = unique_temp_dir("retain-target");
        let mut source = MptAds::new_with_persistence(source_dir.clone());
        let mut target = MptAds::new_with_persistence(target_dir.clone());

        let (prefix_hex, keywords) = keywords_sharing_hashed_prefix(2, 2);
        let moved_a = &keywords[0];
        let moved_b = &keywords[1];
        let outsider = (0..20_000usize)
            .map(|index| format!("mpt-retain-outsider-{index}"))
            .find(|keyword| !AccTrie::hashed_key_hex(keyword.as_bytes()).starts_with(&prefix_hex))
            .expect("outsider keyword");

        source.add(moved_a, "fa");
        source.add(moved_a, "fb");
        source.add(moved_b, "fc");
        source.add(&outsider, "fd");

        let root_before_prepare = source.current_root_hash();
        let (segment, prepared_root) = source
            .prepare_retain_prefix_segment(&prefix_hex)
            .expect("prepare retain prefix segment");

        assert_eq!(prepared_root, root_before_prepare);

        let (moved_a_after_prepare, _) = source.query(moved_a);
        let (moved_b_after_prepare, _) = source.query(moved_b);
        let (outsider_after_prepare, _) = source.query(&outsider);

        assert_eq!(moved_a_after_prepare.len(), 2);
        assert!(moved_a_after_prepare.contains(&"fa".to_string()));
        assert!(moved_a_after_prepare.contains(&"fb".to_string()));
        assert_eq!(moved_b_after_prepare, vec!["fc".to_string()]);
        assert_eq!(outsider_after_prepare, vec!["fd".to_string()]);

        target
            .import_prefix_segment(&segment)
            .expect("import prefix segment");

        let (moved_a_on_target, _) = target.query(moved_a);
        let (moved_b_on_target, _) = target.query(moved_b);
        assert_eq!(moved_a_on_target.len(), 2);
        assert!(moved_a_on_target.contains(&"fa".to_string()));
        assert!(moved_a_on_target.contains(&"fb".to_string()));
        assert_eq!(moved_b_on_target, vec!["fc".to_string()]);

        source
            .confirm_prefix_migration(&prefix_hex)
            .expect("confirm prefix migration");

        let (moved_a_after_confirm, _) = source.query(moved_a);
        let (moved_b_after_confirm, _) = source.query(moved_b);
        let (outsider_after_confirm, _) = source.query(&outsider);

        assert!(moved_a_after_confirm.is_empty());
        assert!(moved_b_after_confirm.is_empty());
        assert_eq!(outsider_after_confirm, vec!["fd".to_string()]);

        let _ = fs::remove_dir_all(source_dir);
        let _ = fs::remove_dir_all(target_dir);
    }

    #[test]
    fn test_mpt_prefix_migration_rejects_empty_prefix() {
        let mut ads = MptAds::new();
        let empty_prefix = String::new();
        ads.add("rust", "file1");

        assert!(ads.export_prefix_segment(" ").is_err());
        assert!(ads.drain_prefix_segment(&empty_prefix).is_err());
        assert!(ads.prepare_retain_prefix_segment(&empty_prefix).is_err());
        assert!(ads.confirm_prefix_migration(&empty_prefix).is_err());

        let (fids, _) = ads.query("rust");
        assert_eq!(fids, vec!["file1".to_string()]);
    }
}
