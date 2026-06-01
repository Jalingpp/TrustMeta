use super::AdsOperations;
use accumulator_tree::AccumulatorTree;
use ads_rust::acctrie::AccTrie;
use common::{
    directory_size_bytes, encode_acctree_proof, AccTreeProofEnvelope, AccTreeProofKind, RootHash,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
const ACCTREE_MIGRATION_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct AccTreeMigratedRecord {
    key: String,
    fid: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccTreePrefixMigrationSegment {
    version: u32,
    prefix_hex: String,
    records: Vec<AccTreeMigratedRecord>,
}

pub struct AccTreeAds {
    tree: Arc<RwLock<AccumulatorTree>>,
    retained_prefixes: HashSet<String>,
    confirmed_prefixes: HashSet<String>,
    persistence_path: Option<PathBuf>,
}

impl AccTreeAds {
    pub fn new() -> Self {
        Self {
            tree: Arc::new(RwLock::new(AccumulatorTree::new())),
            retained_prefixes: HashSet::new(),
            confirmed_prefixes: HashSet::new(),
            persistence_path: None,
        }
    }

    pub fn new_with_persistence(path: PathBuf) -> Self {
        let persistence_path = path.clone();
        let tree = AccumulatorTree::new_with_persistence(path).expect(&String::from_iter([
            'f', 'a', 'i', 'l', 'e', 'd', ' ', 't', 'o', ' ', 'o', 'p', 'e', 'n', ' ', 'a', 'c',
            'c', 't', 'r', 'e', 'e', ' ', 'p', 'e', 'r', 's', 'i', 's', 't', 'e', 'n', 'c', 'e',
        ]));

        Self {
            tree: Arc::new(RwLock::new(tree)),
            retained_prefixes: HashSet::new(),
            confirmed_prefixes: HashSet::new(),
            persistence_path: Some(persistence_path),
        }
    }

    fn current_root_hash_internal(tree: &AccumulatorTree) -> RootHash {
        tree.global_state_hash().to_vec()
    }

    fn compute_storage_bytes(&self) -> u64 {
        if let Some(path) = &self.persistence_path {
            directory_size_bytes(path).unwrap_or(0)
        } else {
            let tree = self.tree.read().unwrap();
            bincode::serialize(&tree.records())
                .map(|bytes| bytes.len() as u64)
                .unwrap_or(0)
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

    fn records_for_prefix(&self, prefix_hex: &str) -> Result<Vec<AccTreeMigratedRecord>, String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let tree = self.tree.read().unwrap();
        let mut records = tree
            .records()
            .into_iter()
            .filter(|(key, _)| AccTrie::hashed_key_hex(key.as_bytes()).starts_with(&normalized))
            .map(|(key, fid)| AccTreeMigratedRecord { key, fid })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.key.cmp(&right.key).then(left.fid.cmp(&right.fid)));
        Ok(records)
    }

    fn rebuild_with_records(&mut self, records: Vec<AccTreeMigratedRecord>) -> RootHash {
        let mut tree = self.tree.write().unwrap();
        tree.rebuild_from_records_snapshot(
            records
                .into_iter()
                .map(|record| (record.key, record.fid))
                .collect(),
        );
        Self::current_root_hash_internal(&tree)
    }

    fn reset_in_memory_state(&mut self) {
        self.tree = Arc::new(RwLock::new(AccumulatorTree::new()));
        self.retained_prefixes.clear();
        self.confirmed_prefixes.clear();
    }
}

impl Default for AccTreeAds {
    fn default() -> Self {
        Self::new()
    }
}

impl AdsOperations for AccTreeAds {
    fn add(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let mut tree = self.tree.write().unwrap();
        let result = tree.insert_with_proof(keyword.to_string(), fid.to_string());
        let root_hash = Self::current_root_hash_internal(&tree);
        let proof = encode_acctree_proof(&AccTreeProofEnvelope {
            root_hash: root_hash.clone(),
            proof: AccTreeProofKind::Add {
                keyword: keyword.to_string(),
                fid: fid.to_string(),
                result,
            },
        })
        .unwrap_or_default();
        (proof, root_hash)
    }

    fn add_batch(&mut self, kvs: Vec<(String, String)>) -> (Vec<u8>, RootHash) {
        if kvs.is_empty() {
            return (Vec::new(), self.current_root_hash());
        }

        let mut records = {
            let tree = self.tree.read().unwrap();
            tree.records()
                .into_iter()
                .map(|(key, fid)| AccTreeMigratedRecord { key, fid })
                .collect::<Vec<_>>()
        };

        records.extend(
            kvs.into_iter()
                .map(|(key, fid)| AccTreeMigratedRecord { key, fid }),
        );
        records.sort_by(|left, right| left.key.cmp(&right.key).then(left.fid.cmp(&right.fid)));
        records.dedup();

        (Vec::new(), self.rebuild_with_records(records))
    }

    fn query(&self, keyword: &str) -> (Vec<String>, Vec<u8>) {
        let tree = self.tree.read().unwrap();
        let result = tree.select_all_with_proof(keyword);
        let fids = result.fids();
        let root_hash = Self::current_root_hash_internal(&tree);
        let proof = encode_acctree_proof(&AccTreeProofEnvelope {
            root_hash,
            proof: AccTreeProofKind::Query {
                keyword: keyword.to_string(),
                result,
            },
        })
        .unwrap_or_default();
        (fids, proof)
    }

    fn delete(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let mut tree = self.tree.write().unwrap();
        let result = tree.delete_with_proof(keyword, fid);
        let root_hash = Self::current_root_hash_internal(&tree);
        let proof = encode_acctree_proof(&AccTreeProofEnvelope {
            root_hash: root_hash.clone(),
            proof: AccTreeProofKind::Delete {
                keyword: keyword.to_string(),
                fid: fid.to_string(),
                result,
            },
        })
        .unwrap_or_default();
        (proof, root_hash)
    }

    fn current_root_hash(&self) -> RootHash {
        let tree = self.tree.read().unwrap();
        Self::current_root_hash_internal(&tree)
    }

    fn record_count(&self) -> usize {
        let tree = self.tree.read().unwrap();
        tree.records().len()
    }

    fn storage_bytes(&self) -> u64 {
        self.compute_storage_bytes()
    }

    fn export_prefix_segment(&self, prefix_hex: &str) -> Result<Vec<u8>, String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let records = self.records_for_prefix(&normalized)?;
        let segment = AccTreePrefixMigrationSegment {
            version: ACCTREE_MIGRATION_FORMAT_VERSION,
            prefix_hex: normalized,
            records,
        };
        bincode::serialize(&segment)
            .map_err(|error| format!("failed to serialize prefix segment: {error}"))
    }

    fn import_prefix_segment(&mut self, segment: &[u8]) -> Result<RootHash, String> {
        let segment: AccTreePrefixMigrationSegment = bincode::deserialize(segment)
            .map_err(|error| format!("failed to deserialize prefix segment: {error}"))?;
        if segment.version != ACCTREE_MIGRATION_FORMAT_VERSION {
            return Err(format!(
                "unsupported prefix segment format version {}",
                segment.version
            ));
        }

        let normalized = Self::normalize_prefix_hex(&segment.prefix_hex)?;
        let mut all_records = {
            let tree = self.tree.read().unwrap();
            tree.records()
                .into_iter()
                .map(|(key, fid)| AccTreeMigratedRecord { key, fid })
                .collect::<Vec<_>>()
        };

        all_records.retain(|record| {
            !AccTrie::hashed_key_hex(record.key.as_bytes()).starts_with(&normalized)
        });
        all_records.extend(segment.records.into_iter().filter(|record| {
            AccTrie::hashed_key_hex(record.key.as_bytes()).starts_with(&normalized)
        }));
        all_records.sort_by(|left, right| left.key.cmp(&right.key).then(left.fid.cmp(&right.fid)));
        all_records.dedup();

        Ok(self.rebuild_with_records(all_records))
    }

    fn drain_prefix_segment(&mut self, prefix_hex: &str) -> Result<(Vec<u8>, RootHash), String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let segment = self.export_prefix_segment(&normalized)?;

        let mut all_records = {
            let tree = self.tree.read().unwrap();
            tree.records()
                .into_iter()
                .map(|(key, fid)| AccTreeMigratedRecord { key, fid })
                .collect::<Vec<_>>()
        };
        all_records.retain(|record| {
            !AccTrie::hashed_key_hex(record.key.as_bytes()).starts_with(&normalized)
        });
        let root_hash = self.rebuild_with_records(all_records);
        Ok((segment, root_hash))
    }

    fn prepare_retain_prefix_segment(
        &mut self,
        prefix_hex: &str,
    ) -> Result<(Vec<u8>, RootHash), String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        let segment = self.export_prefix_segment(&normalized)?;
        self.retained_prefixes.insert(normalized);
        self.confirmed_prefixes.clear();
        Ok((segment, self.current_root_hash()))
    }

    fn confirm_prefix_migration(&mut self, prefix_hex: &str) -> Result<RootHash, String> {
        let normalized = Self::normalize_prefix_hex(prefix_hex)?;
        if !self.retained_prefixes.remove(&normalized) {
            return Err(format!(
                "prefix {normalized} was not prepared for retained migration"
            ));
        }

        self.confirmed_prefixes.insert(normalized);

        if !self.retained_prefixes.is_empty() {
            return Ok(self.current_root_hash());
        }

        let mut all_records = {
            let tree = self.tree.read().unwrap();
            tree.records()
                .into_iter()
                .map(|(key, fid)| AccTreeMigratedRecord { key, fid })
                .collect::<Vec<_>>()
        };
        all_records.retain(|record| {
            !self
                .confirmed_prefixes
                .iter()
                .any(|prefix| AccTrie::hashed_key_hex(record.key.as_bytes()).starts_with(prefix))
        });
        self.confirmed_prefixes.clear();
        Ok(self.rebuild_with_records(all_records))
    }

    fn reset(&mut self) -> Result<(), String> {
        if let Some(path) = &self.persistence_path {
            let _ = std::fs::remove_dir_all(path);
            self.tree = Arc::new(RwLock::new(
                AccumulatorTree::new_with_persistence(path).map_err(|error| error.to_string())?,
            ));
        } else {
            self.reset_in_memory_state();
        }
        self.retained_prefixes.clear();
        self.confirmed_prefixes.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{is_acctree_proof, AdsMode, ProofVerifier};
    use std::collections::HashMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("acctree-ads-{tag}-{nanos}"))
    }

    fn keywords_sharing_hashed_prefix(prefix_len: usize, count: usize) -> (String, Vec<String>) {
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for index in 0..20_000usize {
            let keyword = format!("acctree-migration-key-{index}");
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
    fn test_acctree_ads_basic_operations() {
        let mut ads = AccTreeAds::new();

        let (proof1, root1) = ads.add("rust", "file1");
        assert!(is_acctree_proof(&proof1));
        assert!(!root1.is_empty());

        let (_proof2, _root2) = ads.add("rust", "file2");

        let verifier = ProofVerifier::new(AdsMode::AccTree);
        let (fids, query_proof) = ads.query("rust");
        assert_eq!(fids.len(), 2);
        assert!(fids.contains(&"file1".to_string()));
        assert!(fids.contains(&"file2".to_string()));
        assert!(verifier.verify(&query_proof, &ads.current_root_hash()));
        assert!(verifier.verify_query_result_fids(&query_proof, &fids));

        let (delete_proof, delete_root) = ads.delete("rust", "file1");
        assert!(is_acctree_proof(&delete_proof));
        assert!(verifier.verify(&delete_proof, &delete_root));
    }

    #[test]
    fn test_acctree_ads_persistence_restores_records() {
        let dir = unique_temp_dir("restore");
        let mut ads = AccTreeAds::new_with_persistence(dir.clone());

        ads.add("rust", "file1");
        ads.add("storage", "file2");

        drop(ads);

        let ads = AccTreeAds::new_with_persistence(dir.clone());
        {
            let tree = ads.tree.read().unwrap();
            assert!(tree.roots.is_empty());
            assert!(tree.persisted_root_count() > 0);
        }
        let (rust_fids, _) = ads.query("rust");
        let (storage_fids, _) = ads.query("storage");

        assert_eq!(rust_fids, vec!["file1".to_string()]);
        assert_eq!(storage_fids, vec!["file2".to_string()]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_acctree_prefix_segment_drain_and_import() {
        let source_dir = unique_temp_dir("source");
        let target_dir = unique_temp_dir("target");
        let mut source = AccTreeAds::new_with_persistence(source_dir.clone());
        let mut target = AccTreeAds::new_with_persistence(target_dir.clone());

        let (prefix_hex, keywords) = keywords_sharing_hashed_prefix(2, 2);
        let moved_a = &keywords[0];
        let moved_b = &keywords[1];
        let outsider = (0..20_000usize)
            .map(|index| format!("acctree-outsider-{index}"))
            .find(|keyword| !AccTrie::hashed_key_hex(keyword.as_bytes()).starts_with(&prefix_hex))
            .expect("outsider keyword");

        source.add(moved_a, "fa");
        source.add(moved_a, "fb");
        source.add(moved_b, "fc");
        source.add(&outsider, "fd");

        let (segment, _) = source
            .drain_prefix_segment(&prefix_hex)
            .expect("drain prefix segment");

        let (moved_a_after_drain, _) = source.query(moved_a);
        let (moved_b_after_drain, _) = source.query(moved_b);
        let (outsider_after_drain, _) = source.query(&outsider);

        assert!(moved_a_after_drain.is_empty());
        assert!(moved_b_after_drain.is_empty());
        assert_eq!(outsider_after_drain, vec!["fd".to_string()]);

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
    fn test_acctree_prefix_migration_rejects_empty_prefix() {
        let mut ads = AccTreeAds::new();
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
