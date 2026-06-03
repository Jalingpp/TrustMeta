//! AccTrie (Accumulator-based Trie) ADS 瀹炵幇
//!
//! AccTrie 鏄竴涓粨鍚堝瘑鐮佸绱姞鍣ㄧ殑鍓嶇紑鏍戞暟鎹粨鏋?
//! 姣忎釜鍙跺瓙鑺傜偣缁存姢涓€涓€奸泦鍚堝強鍏跺搴旂殑瀵嗙爜瀛︾疮鍔犲櫒锛屾敮鎸侀珮鏁堢殑鎴愬憳璇佹槑鍜岄泦鍚堟搷浣?

// 鏉′欢鏃ュ織瀹?- 鍙湪闈炲畨闈欐ā寮忎笅鎵撳嵃
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if std::env::var("ADS_QUIET_MODE").is_err() {
            eprintln!($($arg)*);
        }
    };
}

use super::AdsOperations;
use ads_rust::io_stats;
use ads_rust::mpt::node::Database;
use common::{directory_size_bytes, RootHash};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

// 寮曞叆 acctrie 搴?
use ads_rust::acctrie::{AccTrie, DeletionProof, InsertionProof, PersistedRecord, QueryResult};
use ads_rust::mpt::LevelDbDatabase;

const MIGRATION_FORMAT_VERSION: u32 = 1;
const DEFAULT_PAGE_RECORD_LIMIT: usize = 256;
const DEFAULT_MAX_CACHED_PAGES: usize = 64;
const DEFAULT_MANIFEST_PERSIST_INTERVAL: u64 = 32;
const KVDB_MANIFEST_KEY: &[u8] = b"acctrie:manifest";
const KVDB_SHARD_KEY_PREFIX: &str = "acctrie:shard:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PersistenceMode {
    Page,
    KvDb,
}

impl PersistenceMode {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "page" => Some(Self::Page),
            "kvdb" => Some(Self::KvDb),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedShard {
    version: u32,
    root_prefix: Vec<u8>,
    records: Vec<PersistedRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedPage {
    version: u32,
    root_prefix: Vec<u8>,
    page_index: u32,
    records: Vec<PersistedRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedPageManifest {
    page_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedManifest {
    version: u32,
    root_hash: RootHash,
    root_accumulator: Vec<u8>,
    shard_prefixes: Vec<Vec<u8>>,
    #[serde(default)]
    sorted_keys: Vec<Vec<u8>>,
    #[serde(default)]
    accumulator_snapshot: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PrefixMigrationSegment {
    version: u32,
    prefix_hex: String,
    root_prefix: Vec<u8>,
    records: Vec<PersistedRecord>,
}

/// ????????????
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PageMigrationSegment {
    version: u32,
    prefix_hex: String,
    root_prefix: Vec<u8>,
    /// ????????????
    pages: Vec<(u32, Vec<u8>)>, // (page_index, raw_page_bytes)
    /// ????
    manifest: PersistedPageManifest,
    /// ???????????
    root_accumulator: Vec<u8>,
}

#[derive(Clone, Debug)]
struct PersistenceLayout {
    base_dir: PathBuf,
    segments_dir: PathBuf,
    manifest_path: PathBuf,
}

struct KvDbPersistence {
    base_dir: PathBuf,
    db: Mutex<LevelDbDatabase>,
}

enum PersistenceBackend {
    Page(PersistenceLayout),
    KvDb(KvDbPersistence),
}

#[derive(Clone, Debug)]
struct CachedPage {
    records: Vec<PersistedRecord>,
    dirty: bool,
    access_tick: u64,
}

#[derive(Clone, Debug)]
struct PageCacheState {
    pages: HashMap<(String, u32), CachedPage>,
    manifest: HashMap<String, PersistedPageManifest>,
    dirty_page_manifests: HashSet<String>,
    access_tick: u64,
    page_record_limit: usize,
    max_cached_pages: usize,
}

#[derive(Clone, Debug, Default)]
struct PersistenceRuntimeState {
    fully_loaded: bool,
    manifest_dirty: bool,
    manifest_mutation_count: u64,
}

impl Default for PageCacheState {
    fn default() -> Self {
        Self {
            pages: HashMap::new(),
            manifest: HashMap::new(),
            dirty_page_manifests: HashSet::new(),
            access_tick: 0,
            page_record_limit: DEFAULT_PAGE_RECORD_LIMIT,
            max_cached_pages: DEFAULT_MAX_CACHED_PAGES,
        }
    }
}

impl PersistenceLayout {
    fn new(base_dir: PathBuf) -> Self {
        Self {
            segments_dir: base_dir.join("segments"),
            manifest_path: base_dir.join("manifest.bin"),
            base_dir,
        }
    }

    fn storage_bytes(&self) -> u64 {
        directory_size_bytes(&self.base_dir).unwrap_or(0)
    }

    fn ensure_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(&self.base_dir)
            .and_then(|_| fs::create_dir_all(&self.segments_dir))
            .map_err(|error| format!("failed to create persistence directories: {error}"))
    }

    fn shard_path(&self, root_prefix: &[u8]) -> Result<PathBuf, String> {
        let hex = AccTrie::root_prefix_hex(root_prefix)?;
        Ok(self.segments_dir.join(format!("{hex}.bin")))
    }

    fn page_manifest_path(&self, root_prefix: &[u8]) -> Result<PathBuf, String> {
        let hex = AccTrie::root_prefix_hex(root_prefix)?;
        let mut name = hex;
        for ch in ['.', 'p', 'a', 'g', 'e', 's', '.', 'b', 'i', 'n'] {
            name.push(ch);
        }
        Ok(self.segments_dir.join(name))
    }

    fn page_path(&self, root_prefix: &[u8], page_index: u32) -> Result<PathBuf, String> {
        let hex = AccTrie::root_prefix_hex(root_prefix)?;
        let mut name = hex;
        name.push('.');
        for ch in ['p', 'a', 'g', 'e', '.'] {
            name.push(ch);
        }
        name.push_str(&page_index.to_string());
        for ch in ['.', 'b', 'i', 'n'] {
            name.push(ch);
        }
        Ok(self.segments_dir.join(name))
    }

    fn load_page_manifest(
        &self,
        root_prefix: &[u8],
    ) -> Result<Option<PersistedPageManifest>, String> {
        let path = self.page_manifest_path(root_prefix)?;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        io_stats::record_read(bytes.len());
        let manifest = bincode::deserialize(&bytes).map_err(|error| error.to_string())?;
        Ok(Some(manifest))
    }

    fn persist_page_manifest(
        &self,
        root_prefix: &[u8],
        manifest: &PersistedPageManifest,
    ) -> Result<(), String> {
        let path = self.page_manifest_path(root_prefix)?;
        self.ensure_dirs()?;
        let bytes = bincode::serialize(manifest).map_err(|error| error.to_string())?;
        io_stats::record_write(bytes.len());
        fs::write(&path, bytes).map_err(|error| error.to_string())
    }

    fn load_page(&self, root_prefix: &[u8], page_index: u32) -> Result<PersistedPage, String> {
        let path = self.page_path(root_prefix, page_index)?;
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        io_stats::record_read(bytes.len());
        let page: PersistedPage =
            bincode::deserialize(&bytes).map_err(|error| error.to_string())?;
        Ok(page)
    }

    fn persist_page(
        &self,
        root_prefix: &[u8],
        page_index: u32,
        records: Vec<PersistedRecord>,
    ) -> Result<(), String> {
        let path = self.page_path(root_prefix, page_index)?;
        self.ensure_dirs()?;
        let page = PersistedPage {
            version: MIGRATION_FORMAT_VERSION,
            root_prefix: root_prefix.to_vec(),
            page_index,
            records,
        };
        let bytes = bincode::serialize(&page).map_err(|error| error.to_string())?;
        io_stats::record_write(bytes.len());
        fs::write(&path, bytes).map_err(|error| error.to_string())
    }

    fn remove_page(&self, root_prefix: &[u8], page_index: u32) -> Result<(), String> {
        let path = self.page_path(root_prefix, page_index)?;
        if path.exists() {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn remove_page_manifest(&self, root_prefix: &[u8]) -> Result<(), String> {
        let path = self.page_manifest_path(root_prefix)?;
        if path.exists() {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// ???????????????????
    fn read_page_raw(
        &self,
        root_prefix: &[u8],
        page_index: u32,
    ) -> Result<Option<Vec<u8>>, String> {
        let path = self.page_path(root_prefix, page_index)?;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).map_err(|e| format!("failed to read page file: {e}"))?;
        io_stats::record_read(bytes.len());
        Ok(Some(bytes))
    }

    /// ??????????????????
    fn write_page_raw(
        &self,
        root_prefix: &[u8],
        page_index: u32,
        data: &[u8],
    ) -> Result<(), String> {
        let path = self.page_path(root_prefix, page_index)?;
        self.ensure_dirs()?;
        io_stats::record_write(data.len());
        fs::write(&path, data).map_err(|e| format!("failed to write page file: {e}"))
    }

    fn load_into(&self, trie: &mut AccTrie) -> Result<(), String> {
        self.ensure_dirs()?;

        let mut records = Vec::new();
        let Ok(entries) = fs::read_dir(&self.segments_dir) else {
            return Ok(());
        };

        let mut paths = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("bin"))
            .collect::<Vec<_>>();
        paths.sort();
        let mut loaded_page_prefixes = HashSet::new();

        for path in &paths {
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.ends_with(['.', 'p', 'a', 'g', 'e', 's', '.', 'b', 'i', 'n']) {
                continue;
            }
            let prefix_hex = &name[..name.len() - 10];
            let root_prefix = AccTrie::root_prefix_from_hex_prefix(prefix_hex)?;
            let Some(manifest) = self.load_page_manifest(&root_prefix)? else {
                continue;
            };
            for page_index in 0..manifest.page_count {
                let page = self.load_page(&root_prefix, page_index)?;
                records.extend(page.records);
            }
            loaded_page_prefixes.insert(prefix_hex.to_string());
        }

        for path in &paths {
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.ends_with(".bin")
                || name.ends_with(".pages.bin")
                || name.contains(".page.")
                || name == "manifest.bin"
            {
                continue;
            }
            let prefix_hex = &name[..name.len() - 4];
            if loaded_page_prefixes.contains(prefix_hex) {
                continue;
            }
            let bytes = fs::read(&path)
                .map_err(|error| format!("failed to read shard {}: {error}", path.display()))?;
            io_stats::record_read(bytes.len());
            let shard: PersistedShard = bincode::deserialize(&bytes).map_err(|error| {
                format!("failed to deserialize shard {}: {error}", path.display())
            })?;
            if shard.version != MIGRATION_FORMAT_VERSION {
                return Err(format!(
                    "unsupported shard format version {} in {}",
                    shard.version,
                    path.display()
                ));
            }
            records.extend(shard.records);
        }

        trie.restore_from_records(records)
    }

    #[allow(dead_code)]
    fn persist_shard(
        &self,
        root_prefix: &[u8],
        records: Vec<PersistedRecord>,
    ) -> Result<(), String> {
        self.ensure_dirs()?;
        let path = self.shard_path(root_prefix)?;
        if records.is_empty() {
            if path.exists() {
                fs::remove_file(&path).map_err(|error| {
                    format!("failed to remove shard {}: {error}", path.display())
                })?;
            }
            return Ok(());
        }

        let shard = PersistedShard {
            version: MIGRATION_FORMAT_VERSION,
            root_prefix: root_prefix.to_vec(),
            records,
        };
        let bytes = bincode::serialize(&shard)
            .map_err(|error| format!("failed to serialize shard {}: {error}", path.display()))?;
        io_stats::record_write(bytes.len());
        fs::write(&path, bytes)
            .map_err(|error| format!("failed to write shard {}: {error}", path.display()))
    }

    fn persist_manifest(
        &self,
        root_hash: RootHash,
        root_accumulator: Vec<u8>,
        shard_prefixes: Vec<Vec<u8>>,
        sorted_keys: Vec<Vec<u8>>,
        accumulator_snapshot: Vec<Vec<u8>>,
    ) -> Result<(), String> {
        self.ensure_dirs()?;
        let manifest = PersistedManifest {
            version: MIGRATION_FORMAT_VERSION,
            root_hash,
            root_accumulator,
            shard_prefixes,
            sorted_keys,
            accumulator_snapshot,
        };
        let bytes = bincode::serialize(&manifest)
            .map_err(|error| format!("failed to serialize manifest: {error}"))?;
        io_stats::record_write(bytes.len());
        fs::write(&self.manifest_path, bytes).map_err(|error| {
            format!(
                "failed to write manifest {}: {error}",
                self.manifest_path.display()
            )
        })
    }
}

impl KvDbPersistence {
    fn new(base_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&base_dir).map_err(|error| {
            format!(
                "failed to create kvdb directory {}: {error}",
                base_dir.display()
            )
        })?;
        let db = LevelDbDatabase::open(&base_dir).map_err(|error| error.to_string())?;
        Ok(Self {
            base_dir,
            db: Mutex::new(db),
        })
    }

    fn storage_bytes(&self) -> u64 {
        directory_size_bytes(&self.base_dir).unwrap_or(0)
    }

    fn shard_key(root_prefix: &[u8]) -> Result<Vec<u8>, String> {
        let hex = AccTrie::root_prefix_hex(root_prefix)?;
        Ok(format!("{KVDB_SHARD_KEY_PREFIX}{hex}").into_bytes())
    }

    fn load_into(&self, trie: &mut AccTrie) -> Result<(), String> {
        let mut db = self.db.lock().unwrap();
        let Some(manifest_bytes) = db
            .get(KVDB_MANIFEST_KEY)
            .map_err(|error| error.to_string())?
        else {
            return Ok(());
        };

        let manifest: PersistedManifest = bincode::deserialize(&manifest_bytes)
            .map_err(|error| format!("failed to deserialize kvdb manifest: {error}"))?;
        if manifest.version != MIGRATION_FORMAT_VERSION {
            return Err(format!(
                "unsupported kvdb manifest version {}",
                manifest.version
            ));
        }

        let mut records = Vec::new();
        for root_prefix in &manifest.shard_prefixes {
            let shard_key = Self::shard_key(root_prefix)?;
            let Some(bytes) = db.get(&shard_key).map_err(|error| error.to_string())? else {
                continue;
            };
            let shard: PersistedShard = bincode::deserialize(&bytes).map_err(|error| {
                format!(
                    "failed to deserialize kvdb shard {}: {error}",
                    AccTrie::root_prefix_hex(root_prefix).unwrap_or_default()
                )
            })?;
            if shard.version != MIGRATION_FORMAT_VERSION {
                return Err(format!("unsupported kvdb shard version {}", shard.version));
            }
            records.extend(shard.records);
        }

        trie.restore_from_records(records)
    }

    fn persist_manifest(&self, manifest: &PersistedManifest) -> Result<(), String> {
        let bytes = bincode::serialize(manifest)
            .map_err(|error| format!("failed to serialize kvdb manifest: {error}"))?;
        let mut db = self.db.lock().unwrap();
        db.put(KVDB_MANIFEST_KEY, &bytes)
            .map_err(|error| error.to_string())
    }

    fn persist_shard(
        &self,
        root_prefix: &[u8],
        records: Vec<PersistedRecord>,
    ) -> Result<(), String> {
        let mut db = self.db.lock().unwrap();
        let shard_key = Self::shard_key(root_prefix)?;
        if records.is_empty() {
            db.delete(&shard_key).map_err(|error| error.to_string())?;
            return Ok(());
        }

        let shard = PersistedShard {
            version: MIGRATION_FORMAT_VERSION,
            root_prefix: root_prefix.to_vec(),
            records,
        };
        let bytes = bincode::serialize(&shard)
            .map_err(|error| format!("failed to serialize kvdb shard: {error}"))?;
        db.put(&shard_key, &bytes)
            .map_err(|error| error.to_string())
    }

    fn load_shard_records(
        &self,
        root_prefix: &[u8],
    ) -> Result<Option<Vec<PersistedRecord>>, String> {
        let mut db = self.db.lock().unwrap();
        let shard_key = Self::shard_key(root_prefix)?;
        let Some(bytes) = db.get(&shard_key).map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        let shard: PersistedShard = bincode::deserialize(&bytes)
            .map_err(|error| format!("failed to deserialize kvdb shard: {error}"))?;
        if shard.version != MIGRATION_FORMAT_VERSION {
            return Err(format!("unsupported kvdb shard version {}", shard.version));
        }
        Ok(Some(shard.records))
    }
}

impl PersistenceBackend {
    fn storage_bytes(&self) -> u64 {
        match self {
            Self::Page(layout) => layout.storage_bytes(),
            Self::KvDb(layout) => layout.storage_bytes(),
        }
    }

    fn load_into(&self, trie: &mut AccTrie) -> Result<(), String> {
        match self {
            Self::Page(layout) => layout.load_into(trie),
            Self::KvDb(layout) => layout.load_into(trie),
        }
    }
}

/// AccTrie ADS 瀹炵幇
pub struct AccTrieAds {
    /// AccTrie 瀹炰緥
    trie: Arc<RwLock<AccTrie>>,
    persistence: Option<PersistenceBackend>,
    page_cache: RwLock<PageCacheState>,
    runtime: RwLock<PersistenceRuntimeState>,
    retained_prefixes: HashSet<String>,
    persistence_path: Option<PathBuf>,
}

impl AccTrieAds {
    /// 鍒涘缓鏂扮殑 AccTrie ADS 瀹炰緥
    pub fn new() -> Self {
        Self {
            trie: Arc::new(RwLock::new(AccTrie::new())),
            persistence: None,
            page_cache: RwLock::new(PageCacheState::default()),
            runtime: RwLock::new(PersistenceRuntimeState::default()),
            retained_prefixes: HashSet::new(),
            persistence_path: None,
        }
    }

    pub fn new_with_persistence(path: impl Into<PathBuf>) -> Self {
        Self::new_with_page_persistence(path)
    }

    pub fn new_with_page_persistence(path: impl Into<PathBuf>) -> Self {
        let layout = PersistenceLayout::new(path.into());
        let persistence_path = layout.base_dir.clone();
        let backend = PersistenceBackend::Page(layout);
        let mut trie = AccTrie::new();
        if let Err(error) = backend.load_into(&mut trie) {
            debug_log!("AccTrie persistence load failed: {}", error);
        }

        Self {
            trie: Arc::new(RwLock::new(trie)),
            persistence: Some(backend),
            page_cache: RwLock::new(PageCacheState::default()),
            runtime: RwLock::new(PersistenceRuntimeState {
                fully_loaded: true,
                ..PersistenceRuntimeState::default()
            }),
            retained_prefixes: HashSet::new(),
            persistence_path: Some(persistence_path),
        }
    }

    pub fn new_with_kvdb_persistence(path: impl Into<PathBuf>) -> Self {
        let layout = match KvDbPersistence::new(path.into()) {
            Ok(layout) => layout,
            Err(error) => panic!("failed to open AccTrie kvdb persistence: {error}"),
        };
        let persistence_path = layout.base_dir.clone();
        let backend = PersistenceBackend::KvDb(layout);
        let mut trie = AccTrie::new();
        if let Err(error) = backend.load_into(&mut trie) {
            debug_log!("AccTrie kvdb persistence load failed: {}", error);
        }

        Self {
            trie: Arc::new(RwLock::new(trie)),
            persistence: Some(backend),
            page_cache: RwLock::new(PageCacheState::default()),
            runtime: RwLock::new(PersistenceRuntimeState {
                fully_loaded: true,
                ..PersistenceRuntimeState::default()
            }),
            retained_prefixes: HashSet::new(),
            persistence_path: Some(persistence_path),
        }
    }

    pub fn new_with_persistence_mode(path: impl Into<PathBuf>, mode: impl AsRef<str>) -> Self {
        let parsed_mode = PersistenceMode::parse(mode.as_ref());
        match parsed_mode {
            Some(PersistenceMode::KvDb) => Self::new_with_kvdb_persistence(path),
            Some(PersistenceMode::Page) | None => {
                if parsed_mode.is_none() {
                    debug_log!(
                        "Unknown AccTrie persistence mode '{}', falling back to page",
                        mode.as_ref()
                    );
                }
                Self::new_with_page_persistence(path)
            }
        }
    }

    fn reset_in_memory_state(&mut self) {
        self.trie = Arc::new(RwLock::new(AccTrie::new()));
        self.page_cache = RwLock::new(PageCacheState::default());
        self.runtime = RwLock::new(PersistenceRuntimeState::default());
        self.retained_prefixes.clear();
    }

    pub fn structure_summary(&self) -> String {
        let trie = self.trie.read().unwrap();
        let records = trie.records();
        let snapshot = trie.accumulator_snapshot();
        let mut prefix_counts: HashMap<String, usize> = HashMap::new();

        for record in &records {
            let prefix = AccTrie::root_prefix_hex_for_key(&record.key);
            *prefix_counts.entry(prefix).or_insert(0) += 1;
        }

        let mut prefix_lines = prefix_counts.into_iter().collect::<Vec<_>>();
        prefix_lines.sort_by(|left, right| left.0.cmp(&right.0));

        let prefix_text = if prefix_lines.is_empty() {
            "none".to_string()
        } else {
            prefix_lines
                .into_iter()
                .map(|(prefix, count)| format!("{}:{}", prefix, count))
                .collect::<Vec<_>>()
                .join(", ")
        };

        format!(
            "records={}, root_entries={}, retained_prefixes={}, prefixes=[{}]",
            records.len(),
            snapshot.len(),
            self.retained_prefixes.len(),
            prefix_text
        )
    }

    /// 鏀堕泦褰撳墠鎵€鏈夊彾瀛愮疮鍔犲櫒鐨勫簭鍒楀寲瀛楄妭锛堟寜閾捐〃椤哄簭锛?
    fn collect_accumulator_snapshot(trie: &AccTrie) -> Vec<Vec<u8>> {
        trie.accumulator_snapshot()
    }

    /// 灏嗙疮鍔犲櫒蹇収杩藉姞鍒板瓧鑺傛祦鏈熬锛堢敤浜庤瘉鏄庢惡甯︽牴鍝堝笇鎵€闇€鐨勪笂涓嬫枃锛?
    fn append_accumulator_snapshot(snapshot: &[Vec<u8>], bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(snapshot.len() as u32).to_le_bytes());
        for acc in snapshot {
            bytes.extend_from_slice(&(acc.len() as u32).to_le_bytes());
            bytes.extend_from_slice(acc);
        }
    }

    /// 鍩轰簬绱姞鍣ㄥ揩鐓ц绠楀叏灞€鏍瑰搱甯?
    #[allow(dead_code)]
    fn hash_accumulator_snapshot(snapshot: &[Vec<u8>]) -> RootHash {
        let mut hasher = Sha256::new();

        if snapshot.is_empty() {
            hasher.update(b"empty_acctrie");
        } else {
            for acc in snapshot {
                hasher.update(acc);
            }
        }

        hasher.finalize().to_vec()
    }

    fn touched_root_prefixes(trie: &AccTrie) -> Vec<Vec<u8>> {
        let snapshot = trie.accumulator_snapshot();
        snapshot
            .iter()
            .filter_map(|entry| {
                if entry.len() < 4 {
                    return None;
                }
                let prefix_len =
                    u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]) as usize;
                let end = 4 + prefix_len;
                if entry.len() < end {
                    return None;
                }
                Some(entry[4..end].to_vec())
            })
            .collect()
    }

    fn manifest_persist_interval() -> u64 {
        std::env::var("STORAGER_ACCTRIE_MANIFEST_PERSIST_INTERVAL")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MANIFEST_PERSIST_INTERVAL)
    }

    fn mark_manifest_dirty(&self) {
        if let Ok(mut runtime) = self.runtime.write() {
            runtime.manifest_dirty = true;
            runtime.manifest_mutation_count = runtime.manifest_mutation_count.saturating_add(1);
        }
    }

    fn maybe_persist_manifest_for(&self, trie: &AccTrie, force: bool) -> Result<(), String> {
        if self.persistence.is_none() {
            return Ok(());
        }

        let should_persist = {
            let mut runtime = self.runtime.write().unwrap();
            if !runtime.manifest_dirty {
                return Ok(());
            }

            let interval = Self::manifest_persist_interval();
            if !force && runtime.manifest_mutation_count % interval != 0 {
                return Ok(());
            }

            runtime.manifest_dirty = false;
            true
        };

        if !should_persist {
            return Ok(());
        }

        if let Err(error) = self.persist_manifest_for(trie) {
            if let Ok(mut runtime) = self.runtime.write() {
                runtime.manifest_dirty = true;
            }
            return Err(error);
        }

        Ok(())
    }

    fn persist_manifest_for(&self, trie: &AccTrie) -> Result<(), String> {
        let manifest = PersistedManifest {
            version: MIGRATION_FORMAT_VERSION,
            root_hash: self.get_root_hash_from_trie(trie),
            root_accumulator: trie.root_accumulator_bytes(),
            shard_prefixes: Self::touched_root_prefixes(trie),
            sorted_keys: trie
                .records()
                .into_iter()
                .map(|record| record.key)
                .collect(),
            accumulator_snapshot: Self::collect_accumulator_snapshot(trie),
        };
        match self.persistence.as_ref() {
            Some(PersistenceBackend::Page(layout)) => layout.persist_manifest(
                manifest.root_hash,
                manifest.root_accumulator,
                manifest.shard_prefixes,
                manifest.sorted_keys,
                manifest.accumulator_snapshot,
            ),
            Some(PersistenceBackend::KvDb(layout)) => layout.persist_manifest(&manifest),
            None => Ok(()),
        }
    }

    fn cache_root_prefix_records(
        &self,
        root_prefix: &[u8],
        records: Vec<PersistedRecord>,
        mark_dirty: bool,
    ) -> Result<(), String> {
        let Some(PersistenceBackend::Page(_)) = self.persistence.as_ref() else {
            return Ok(());
        };

        let root_hex = AccTrie::root_prefix_hex(root_prefix)?;
        let mut cache = self.page_cache.write().unwrap();
        let page_record_limit = cache.page_record_limit.max(1);
        let page_count = if records.is_empty() {
            0
        } else {
            records.len().div_ceil(page_record_limit) as u32
        };
        let old_manifest = cache.manifest.get(&root_hex).cloned();

        let mut offset = 0usize;
        for page_index in 0..page_count {
            let end = (offset + page_record_limit).min(records.len());
            cache.access_tick += 1;
            let access_tick = cache.access_tick;
            cache.pages.insert(
                (root_hex.clone(), page_index),
                CachedPage {
                    records: records[offset..end].to_vec(),
                    dirty: mark_dirty,
                    access_tick,
                },
            );
            offset = end;
        }

        let stale_keys = cache
            .pages
            .keys()
            .filter(|(prefix, page_index)| prefix == &root_hex && *page_index >= page_count)
            .cloned()
            .collect::<Vec<_>>();
        for key in stale_keys {
            cache.pages.remove(&key);
        }

        let manifest = PersistedPageManifest { page_count };
        if mark_dirty && old_manifest.as_ref() != Some(&manifest) {
            cache.dirty_page_manifests.insert(root_hex.clone());
        }
        cache.manifest.insert(root_hex, manifest);
        drop(cache);
        self.enforce_page_cache_limit()?;
        Ok(())
    }

    fn clear_cached_root_prefix(&self, root_prefix: &[u8]) -> Result<(), String> {
        let root_hex = AccTrie::root_prefix_hex(root_prefix)?;
        let mut cache = self.page_cache.write().unwrap();
        cache.pages.retain(|(prefix, _), _| prefix != &root_hex);
        cache.manifest.remove(&root_hex);
        cache.dirty_page_manifests.remove(&root_hex);
        Ok(())
    }

    fn flush_cached_page(&self, prefix_hex: &str, page_index: u32) -> Result<(), String> {
        let Some(PersistenceBackend::Page(layout)) = self.persistence.as_ref() else {
            return Ok(());
        };
        let root_prefix = AccTrie::root_prefix_from_hex_prefix(prefix_hex)?;

        let records = {
            let mut cache = self.page_cache.write().unwrap();
            let Some(page) = cache.pages.get_mut(&(prefix_hex.to_string(), page_index)) else {
                return Ok(());
            };
            if !page.dirty {
                return Ok(());
            }
            page.dirty = false;
            page.records.clone()
        };

        layout.persist_page(&root_prefix, page_index, records)
    }

    fn enforce_page_cache_limit(&self) -> Result<(), String> {
        loop {
            let eviction_target = {
                let cache = self.page_cache.read().unwrap();
                if cache.pages.len() <= cache.max_cached_pages.max(1) {
                    None
                } else {
                    cache
                        .pages
                        .iter()
                        .min_by_key(|(_, page)| page.access_tick)
                        .map(|(key, page)| (key.clone(), page.dirty))
                }
            };

            let Some(((prefix_hex, page_index), dirty)) = eviction_target else {
                break;
            };

            if dirty {
                self.flush_cached_page(&prefix_hex, page_index)?;
            }

            let mut cache = self.page_cache.write().unwrap();
            cache.pages.remove(&(prefix_hex, page_index));
        }

        Ok(())
    }

    fn flush_root_prefix_pages(&self, root_prefix: &[u8]) -> Result<(), String> {
        let Some(PersistenceBackend::Page(layout)) = self.persistence.as_ref() else {
            return Ok(());
        };

        let root_hex = AccTrie::root_prefix_hex(root_prefix)?;
        let (manifest, pages_to_write, stale_pages, manifest_dirty) = {
            let mut cache = self.page_cache.write().unwrap();
            let manifest = cache
                .manifest
                .get(&root_hex)
                .cloned()
                .unwrap_or(PersistedPageManifest { page_count: 0 });

            let mut pages_to_write = Vec::new();
            for page_index in 0..manifest.page_count {
                if let Some(page) = cache.pages.get_mut(&(root_hex.clone(), page_index)) {
                    if page.dirty {
                        pages_to_write.push((page_index, page.records.clone()));
                        page.dirty = false;
                    }
                }
            }

            let stale_pages = cache
                .pages
                .keys()
                .filter(|(prefix, page_index)| {
                    prefix == &root_hex && *page_index >= manifest.page_count
                })
                .map(|(_, page_index)| *page_index)
                .collect::<Vec<_>>();

            let manifest_dirty = cache.dirty_page_manifests.remove(&root_hex);

            (manifest, pages_to_write, stale_pages, manifest_dirty)
        };

        if manifest_dirty {
            if manifest.page_count == 0 {
                layout.remove_page_manifest(root_prefix)?;
            } else {
                layout.persist_page_manifest(root_prefix, &manifest)?;
            }
        }

        for (page_index, records) in pages_to_write {
            layout.persist_page(root_prefix, page_index, records)?;
        }
        for page_index in stale_pages {
            layout.remove_page(root_prefix, page_index)?;
        }

        let shard_path = layout.shard_path(root_prefix)?;
        if shard_path.exists() {
            fs::remove_file(&shard_path).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn flush_all_dirty_pages(&self) -> Result<(), String> {
        let prefixes = {
            let cache = self.page_cache.read().unwrap();
            cache.manifest.keys().cloned().collect::<Vec<_>>()
        };

        for prefix_hex in prefixes {
            let root_prefix = AccTrie::root_prefix_from_hex_prefix(&prefix_hex)?;
            self.flush_root_prefix_pages(&root_prefix)?;
        }
        Ok(())
    }

    fn load_cached_root_prefix_records(
        &self,
        root_prefix: &[u8],
    ) -> Result<Option<Vec<PersistedRecord>>, String> {
        let root_hex = AccTrie::root_prefix_hex(root_prefix)?;
        let mut cache = self.page_cache.write().unwrap();
        let Some(manifest) = cache.manifest.get(&root_hex).cloned() else {
            return Ok(None);
        };

        let mut records = Vec::new();
        for page_index in 0..manifest.page_count {
            cache.access_tick += 1;
            let tick = cache.access_tick;
            if let Some(page) = cache.pages.get_mut(&(root_hex.clone(), page_index)) {
                page.access_tick = tick;
                records.extend(page.records.clone());
            } else {
                return Ok(None);
            }
        }

        Ok(Some(records))
    }

    fn persist_root_prefix_records(
        &self,
        trie: &AccTrie,
        root_prefix: &[u8],
    ) -> Result<(), String> {
        match self.persistence.as_ref() {
            Some(PersistenceBackend::Page(_)) => self.cache_root_prefix_records(
                root_prefix,
                trie.records_for_root_prefix(root_prefix),
                true,
            ),
            Some(PersistenceBackend::KvDb(layout)) => {
                layout.persist_shard(root_prefix, trie.records_for_root_prefix(root_prefix))
            }
            None => Ok(()),
        }
    }

    fn persist_root_prefix(&self, trie: &AccTrie, root_prefix: &[u8]) -> Result<(), String> {
        self.persist_root_prefix_records(trie, root_prefix)?;
        self.mark_manifest_dirty();
        self.maybe_persist_manifest_for(trie, false)
    }

    fn load_shard_records(
        &self,
        trie: &AccTrie,
        root_prefix: &[u8],
    ) -> Result<Vec<PersistedRecord>, String> {
        match self.persistence.as_ref() {
            None => Ok(trie.records_for_root_prefix(root_prefix)),
            Some(PersistenceBackend::Page(layout)) => {
                if let Some(records) = self.load_cached_root_prefix_records(root_prefix)? {
                    return Ok(records);
                }
                if let Some(manifest) = layout.load_page_manifest(root_prefix)? {
                    let mut records = Vec::new();
                    for page_index in 0..manifest.page_count {
                        let page = layout.load_page(root_prefix, page_index)?;
                        records.extend(page.records);
                    }
                    self.cache_root_prefix_records(root_prefix, records.clone(), false)?;
                    return Ok(records);
                }
                let path = layout.shard_path(root_prefix)?;
                if !path.exists() {
                    return Ok(trie.records_for_root_prefix(root_prefix));
                }
                let bytes = fs::read(&path)
                    .map_err(|error| format!("failed to read shard {}: {error}", path.display()))?;
                io_stats::record_read(bytes.len());
                let shard: PersistedShard = bincode::deserialize(&bytes).map_err(|error| {
                    format!("failed to deserialize shard {}: {error}", path.display())
                })?;
                self.cache_root_prefix_records(root_prefix, shard.records.clone(), false)?;
                Ok(shard.records)
            }
            Some(PersistenceBackend::KvDb(layout)) => {
                if let Some(records) = layout.load_shard_records(root_prefix)? {
                    Ok(records)
                } else {
                    Ok(trie.records_for_root_prefix(root_prefix))
                }
            }
        }
    }

    /// ????????????????????
    pub fn export_prefix_segment(&self, prefix_hex: &str) -> Result<Vec<u8>, String> {
        let normalized = prefix_hex.trim().to_lowercase();
        if normalized.is_empty() {
            return Err("hex prefix cannot be empty".to_string());
        }
        if normalized.len() > 64 {
            return Err(format!("hex prefix is too long: {prefix_hex}"));
        }
        let root_prefix = AccTrie::root_prefix_from_hex_prefix(&normalized)?;

        // 1. ??????
        match self.persistence.as_ref() {
            Some(PersistenceBackend::Page(layout)) => {
                self.flush_root_prefix_pages(&root_prefix)?;

                // 2. ??????
                let manifest = layout
                    .load_page_manifest(&root_prefix)?
                    .unwrap_or(PersistedPageManifest { page_count: 0 });

                // 3. ????????????????
                let mut pages = Vec::new();
                for page_index in 0..manifest.page_count {
                    if let Some(raw_data) = layout.read_page_raw(&root_prefix, page_index)? {
                        pages.push((page_index, raw_data));
                    }
                }

                // 4. ?????????????
                let trie = self.trie.read().unwrap();
                let root_accumulator = trie.root_accumulator_bytes();

                // 5. ????????
                let segment = PageMigrationSegment {
                    version: MIGRATION_FORMAT_VERSION,
                    prefix_hex: normalized,
                    root_prefix,
                    pages,
                    manifest,
                    root_accumulator,
                };

                bincode::serialize(&segment)
                    .map_err(|error| format!("failed to serialize page segment: {error}"))
            }
            Some(PersistenceBackend::KvDb(_)) | None => {
                let trie = self.trie.read().unwrap();
                let records = trie
                    .records_for_root_prefix(&root_prefix)
                    .into_iter()
                    .filter(|record| AccTrie::key_matches_hashed_prefix(&record.key, &normalized))
                    .collect::<Vec<_>>();
                let segment = PrefixMigrationSegment {
                    version: MIGRATION_FORMAT_VERSION,
                    prefix_hex: normalized,
                    root_prefix,
                    records,
                };
                bincode::serialize(&segment)
                    .map_err(|error| format!("failed to serialize prefix segment: {error}"))
            }
        }
    }

    /// ????????????????????
    pub fn import_prefix_segment(&mut self, segment_bytes: &[u8]) -> Result<RootHash, String> {
        // ????????
        if let Ok(segment) = bincode::deserialize::<PageMigrationSegment>(segment_bytes) {
            if segment.version == MIGRATION_FORMAT_VERSION {
                return self.import_page_segment(&segment);
            }
        }

        // ???????????????/?????
        let segment: PrefixMigrationSegment = bincode::deserialize(segment_bytes)
            .map_err(|error| format!("failed to deserialize prefix segment: {error}"))?;
        if segment.version != MIGRATION_FORMAT_VERSION {
            return Err(format!(
                "unsupported prefix segment format version {}",
                segment.version
            ));
        }

        // ????????? trie
        let mut trie = self.trie.write().unwrap();
        let mut shard_records = self.load_shard_records(&trie, &segment.root_prefix)?;
        shard_records
            .retain(|record| !AccTrie::key_matches_hashed_prefix(&record.key, &segment.prefix_hex));
        shard_records.extend(segment.records);
        shard_records.sort_by(|left, right| {
            AccTrie::hashed_key_hex(&left.key).cmp(&AccTrie::hashed_key_hex(&right.key))
        });
        trie.replace_root_prefix_records(&segment.root_prefix, shard_records)?;

        if self.persistence.is_some() {
            self.persist_root_prefix(&trie, &segment.root_prefix)?;
        }
        Ok(self.get_root_hash_from_trie(&trie))
    }

    /// ???????
    fn import_page_segment(&mut self, segment: &PageMigrationSegment) -> Result<RootHash, String> {
        match self.persistence.as_ref() {
            Some(PersistenceBackend::Page(layout)) => {
                // 1. ????????????? trie?
                for (page_index, raw_data) in &segment.pages {
                    layout.write_page_raw(&segment.root_prefix, *page_index, raw_data)?;
                }

                // 2. ??????
                layout.persist_page_manifest(&segment.root_prefix, &segment.manifest)?;

                // 3. ?? trie???????????
                let mut all_records = Vec::new();
                for (_, raw_data) in &segment.pages {
                    if let Ok(page) = bincode::deserialize::<PersistedPage>(raw_data) {
                        all_records.extend(page.records);
                    }
                }

                let _ = self.cache_root_prefix_records(
                    &segment.root_prefix,
                    all_records.clone(),
                    false,
                );

                let mut trie = self.trie.write().unwrap();
                let mut existing_records = self.load_shard_records(&trie, &segment.root_prefix)?;
                existing_records
                    .retain(|r| !AccTrie::key_matches_hashed_prefix(&r.key, &segment.prefix_hex));
                existing_records.extend(all_records);
                existing_records.sort_by(|a, b| {
                    AccTrie::hashed_key_hex(&a.key).cmp(&AccTrie::hashed_key_hex(&b.key))
                });
                trie.replace_root_prefix_records(&segment.root_prefix, existing_records)?;
                self.mark_manifest_dirty();
                self.maybe_persist_manifest_for(&trie, true)?;

                Ok(self.get_root_hash_from_trie(&trie))
            }
            Some(PersistenceBackend::KvDb(_)) => {
                let mut all_records = Vec::new();
                for (_, raw_data) in &segment.pages {
                    if let Ok(page) = bincode::deserialize::<PersistedPage>(raw_data) {
                        all_records.extend(page.records);
                    }
                }

                let mut trie = self.trie.write().unwrap();
                let mut existing_records = self.load_shard_records(&trie, &segment.root_prefix)?;
                existing_records
                    .retain(|r| !AccTrie::key_matches_hashed_prefix(&r.key, &segment.prefix_hex));
                existing_records.extend(all_records);
                existing_records.sort_by(|a, b| {
                    AccTrie::hashed_key_hex(&a.key).cmp(&AccTrie::hashed_key_hex(&b.key))
                });
                trie.replace_root_prefix_records(&segment.root_prefix, existing_records)?;
                self.persist_root_prefix(&trie, &segment.root_prefix)?;
                Ok(self.get_root_hash_from_trie(&trie))
            }
            None => Err("page segment import requires persistence enabled".to_string()),
        }
    }

    /// ??? drain?????????????????
    pub fn drain_prefix_segment(
        &mut self,
        prefix_hex: &str,
    ) -> Result<(Vec<u8>, RootHash), String> {
        let normalized = prefix_hex.trim().to_lowercase();
        if normalized.is_empty() {
            return Err("hex prefix cannot be empty".to_string());
        }
        if normalized.len() > 64 {
            return Err(format!("hex prefix is too long: {prefix_hex}"));
        }
        let root_prefix = AccTrie::root_prefix_from_hex_prefix(&normalized)?;

        // 1. ???????? trie?
        let segment = self.export_prefix_segment(&normalized)?;

        // 2. ??????????????
        if let Some(PersistenceBackend::Page(layout)) = self.persistence.as_ref() {
            let manifest = layout.load_page_manifest(&root_prefix)?;
            if let Some(m) = manifest {
                for page_index in 0..m.page_count {
                    let _ = layout.remove_page(&root_prefix, page_index);
                }
                let _ = layout.remove_page_manifest(&root_prefix);
            }
            let _ = self.clear_cached_root_prefix(&root_prefix);
        }

        // 3. ? trie ?????
        let mut trie = self.trie.write().unwrap();
        let mut shard_records = trie.records_for_root_prefix(&root_prefix);
        shard_records
            .retain(|record| !AccTrie::key_matches_hashed_prefix(&record.key, &normalized));
        trie.replace_root_prefix_records(&root_prefix, shard_records)?;

        // 4. ???????????????
        if self.persistence.is_some() {
            self.persist_root_prefix(&trie, &root_prefix)?;
            self.maybe_persist_manifest_for(&trie, true)?;
        }

        let root_hash = self.get_root_hash_from_trie(&trie);
        Ok((segment, root_hash))
    }

    /// ??? prepare??? flush????????
    pub fn prepare_retain_prefix_segment(
        &mut self,
        prefix_hex: &str,
    ) -> Result<(Vec<u8>, RootHash), String> {
        let normalized = prefix_hex.trim().to_lowercase();
        if normalized.is_empty() {
            return Err("hex prefix cannot be empty".to_string());
        }
        if normalized.len() > 64 {
            return Err(format!("hex prefix is too long: {prefix_hex}"));
        }

        // 1. ???????? trie??? flush?
        let segment = self.export_prefix_segment(&normalized)?;

        // 2. ??????
        self.retained_prefixes.insert(normalized);

        // 3. ??????? flush?
        let trie = self.trie.read().unwrap();
        Ok((segment, self.get_root_hash_from_trie(&trie)))
    }

    /// ??? confirm?????????????????
    pub fn confirm_prefix_migration(&mut self, prefix_hex: &str) -> Result<RootHash, String> {
        let normalized = prefix_hex.trim().to_lowercase();
        if normalized.is_empty() {
            return Err("hex prefix cannot be empty".to_string());
        }
        if normalized.len() > 64 {
            return Err(format!("hex prefix is too long: {prefix_hex}"));
        }
        let root_prefix = AccTrie::root_prefix_from_hex_prefix(&normalized)?;

        // 1. ???????
        let retained = self.retained_prefixes.remove(&normalized);
        if !retained {
            return Err(format!(
                "prefix {normalized} was not prepared for migration"
            ));
        }

        // 2. ??????????????
        if let Some(PersistenceBackend::Page(layout)) = self.persistence.as_ref() {
            let manifest = layout.load_page_manifest(&root_prefix)?;
            if let Some(m) = manifest {
                for page_index in 0..m.page_count {
                    let _ = layout.remove_page(&root_prefix, page_index);
                }
                let _ = layout.remove_page_manifest(&root_prefix);
            }
            let _ = self.clear_cached_root_prefix(&root_prefix);
        }

        // 3. ? trie ?????
        let mut trie = self.trie.write().unwrap();
        let mut shard_records = trie.records_for_root_prefix(&root_prefix);
        shard_records
            .retain(|record| !AccTrie::key_matches_hashed_prefix(&record.key, &normalized));
        trie.replace_root_prefix_records(&root_prefix, shard_records)?;

        // 4. ???????????????
        if self.persistence.is_some() {
            self.persist_root_prefix(&trie, &root_prefix)?;
            self.maybe_persist_manifest_for(&trie, true)?;
        }

        Ok(self.get_root_hash_from_trie(&trie))
    }

    fn serialize_string(value: &str, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }

    /// 搴忓垪鍖栨垚鍛樿瘉鏄?
    fn serialize_membership_proof(
        proof: &Option<ads_rust::acctrie::MembershipProof>,
        bytes: &mut Vec<u8>,
    ) {
        use ark_serialize::CanonicalSerialize;

        if let Some(ref p) = proof {
            bytes.push(1);
            let mut witness_bytes = Vec::new();
            p.witness
                .serialize_uncompressed(&mut witness_bytes)
                .unwrap();
            bytes.extend_from_slice(&(witness_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&witness_bytes);

            let mut element_bytes = Vec::new();
            p.element
                .serialize_uncompressed(&mut element_bytes)
                .unwrap();
            bytes.extend_from_slice(&(element_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&element_bytes);
        } else {
            bytes.push(0);
        }
    }

    /// 搴忓垪鍖栨彃鍏ヨ瘉鏄庝负瀛楄妭鏁扮粍锛堝畬鏁寸増鏈級
    fn serialize_insertion_proof(
        proof: &InsertionProof,
        snapshot: &[Vec<u8>],
        include_snapshot: bool,
    ) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();

        // 璇佹槑绫诲瀷鏍囪: 0x01 = InsertionProof
        bytes.push(0x01);

        // 搴忓垪鍖栭敭
        bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&proof.key);

        // 搴忓垪鍖栧€?
        Self::serialize_string(&proof.value, &mut bytes);

        // 搴忓垪鍖栧墠搴忛敭锛堝彲閫夛級
        if let Some(ref key_prev) = proof.key_prev {
            bytes.push(1);
            bytes.extend_from_slice(&(key_prev.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key_prev);
        } else {
            bytes.push(0);
        }

        // 搴忓垪鍖栧悗搴忛敭锛堝彲閫夛級
        if let Some(ref key_next) = proof.key_next {
            bytes.push(1);
            bytes.extend_from_slice(&(key_next.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key_next);
        } else {
            bytes.push(0);
        }

        // 搴忓垪鍖栫疮鍔犲櫒鍊?
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

        // 搴忓垪鍖栧墠搴忓彾瀛愮疮鍔犲櫒锛堝彲閫夛級
        if let Some(ref ln_prev_acc) = proof.ln_prev_acc {
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

        // 搴忓垪鍖栧悗搴忓彾瀛愮疮鍔犲櫒锛堝彲閫夛級
        if let Some(ref ln_next_acc_old) = proof.ln_next_acc_old {
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

        if let Some(ref ln_next_acc_new) = proof.ln_next_acc_new {
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

        // 搴忓垪鍖栨垚鍛樿瘉鏄?
        Self::serialize_membership_proof(&proof.keyp_in_ln_next_old_proof, &mut bytes);
        Self::serialize_membership_proof(&proof.keyp_in_ln_proof, &mut bytes);
        Self::serialize_membership_proof(&proof.no_prev_in_ln_proof, &mut bytes);
        Self::serialize_membership_proof(&proof.key_in_ln_next_new_proof, &mut bytes);
        Self::serialize_membership_proof(&proof.keyp_in_ln_next_new_proof, &mut bytes);
        Self::serialize_membership_proof(&proof.value_in_ln_proof, &mut bytes);

        // 闄勫甫绱姞鍣ㄥ揩鐓э紝鐢ㄤ簬鏍瑰搱甯岄獙璇侊紙鎵归噺鍦烘櫙鍙烦杩囷級
        if include_snapshot {
            Self::append_accumulator_snapshot(snapshot, &mut bytes);
        }

        bytes
    }

    /// 搴忓垪鍖栧垹闄よ瘉鏄庝负瀛楄妭鏁扮粍锛堝畬鏁寸増鏈級
    fn query_from_trie(trie: &AccTrie, key: &[u8], snapshot: &[Vec<u8>]) -> (Vec<String>, Vec<u8>) {
        let fids = trie.map.get(key).cloned().unwrap_or_default();
        let value = fids.first().cloned().unwrap_or_default();
        let proof = match trie.query(key, &value) {
            Ok(result) => Self::serialize_query_result(&result, snapshot),
            Err(_) => Vec::new(),
        };
        (fids, proof)
    }

    fn serialize_deletion_proof(proof: &DeletionProof, snapshot: &[Vec<u8>]) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();

        // 璇佹槑绫诲瀷鏍囪: 0x02 = DeletionProof
        bytes.push(0x02);

        // 搴忓垪鍖栭敭
        bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&proof.key);

        // 鏄惁鍒犻櫎鏁翠釜鍙跺瓙
        bytes.push(if proof.delete_entire_leaf { 1 } else { 0 });

        // 搴忓垪鍖栧€硷紙鍙€夛級
        if let Some(ref value) = proof.value {
            bytes.push(1);
            Self::serialize_string(value, &mut bytes);
        } else {
            bytes.push(0);
        }

        // 搴忓垪鍖栧墠搴忛敭锛堝彲閫夛級
        if let Some(ref key_prev) = proof.key_prev {
            bytes.push(1);
            bytes.extend_from_slice(&(key_prev.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key_prev);
        } else {
            bytes.push(0);
        }

        // 搴忓垪鍖栧悗搴忛敭锛堝彲閫夛級
        if let Some(ref key_next) = proof.key_next {
            bytes.push(1);
            bytes.extend_from_slice(&(key_next.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key_next);
        } else {
            bytes.push(0);
        }

        // 搴忓垪鍖栨棫绱姞鍣ㄥ€?
        let mut acc_old_bytes = Vec::new();
        proof
            .ln_acc_old
            .serialize_uncompressed(&mut acc_old_bytes)
            .unwrap();
        bytes.extend_from_slice(&(acc_old_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&acc_old_bytes);

        // 搴忓垪鍖栨柊绱姞鍣ㄥ€硷紙鍙€夛級
        if let Some(ref acc_new) = proof.ln_acc_new {
            bytes.push(1);
            let mut acc_new_bytes = Vec::new();
            acc_new.serialize_uncompressed(&mut acc_new_bytes).unwrap();
            bytes.extend_from_slice(&(acc_new_bytes.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&acc_new_bytes);
        } else {
            bytes.push(0);
        }

        // 搴忓垪鍖栧悗搴忓彾瀛愮疮鍔犲櫒锛堝彲閫夛級
        if let Some(ref ln_next_acc_old) = proof.ln_next_acc_old {
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

        if let Some(ref ln_next_acc_new) = proof.ln_next_acc_new {
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

        // 搴忓垪鍖栨垚鍛樿瘉鏄?
        Self::serialize_membership_proof(&proof.value_in_ln_old_proof, &mut bytes);
        Self::serialize_membership_proof(&proof.keyp_in_ln_proof, &mut bytes);
        Self::serialize_membership_proof(&proof.key_in_ln_next_old_proof, &mut bytes);
        Self::serialize_membership_proof(&proof.keyp_in_ln_next_new_proof, &mut bytes);

        // 闄勫甫绱姞鍣ㄥ揩鐓э紝鐢ㄤ簬鏍瑰搱甯岄獙璇?
        Self::append_accumulator_snapshot(snapshot, &mut bytes);

        bytes
    }

    /// 搴忓垪鍖栨煡璇㈢粨鏋滀负瀛楄妭鏁扮粍锛堝畬鏁寸増鏈紝鍖呭惈鎴愬憳璇佹槑锛?
    fn serialize_query_result(result: &QueryResult, snapshot: &[Vec<u8>]) -> Vec<u8> {
        use ark_serialize::CanonicalSerialize;

        let mut bytes = Vec::new();

        match result {
            QueryResult::Exists(proof) => {
                // 璇佹槑绫诲瀷鏍囪: 0x03 = QueryProof (Exists)
                bytes.push(0x03);
                bytes.push(1); // 瀛樺湪鏍囪

                bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&proof.key);
                Self::serialize_string(&proof.value, &mut bytes);
                bytes.extend_from_slice(&proof.value_count.to_le_bytes());

                // 搴忓垪鍖栧彾瀛愮疮鍔犲櫒鍊?
                let mut acc_bytes = Vec::new();
                proof.ln_acc.serialize_uncompressed(&mut acc_bytes).unwrap();
                bytes.extend_from_slice(&(acc_bytes.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&acc_bytes);

                // 搴忓垪鍖栨垚鍛樿瘉鏄庯紙濡傛灉鏈夛級
                if let Some(ref membership_proof) = proof.membership_proof {
                    bytes.push(1);
                    // 搴忓垪鍖杦itness
                    let mut witness_bytes = Vec::new();
                    membership_proof
                        .witness
                        .serialize_uncompressed(&mut witness_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(witness_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&witness_bytes);

                    // 搴忓垪鍖杄lement
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

                Self::serialize_membership_proof(&proof.count_membership_proof, &mut bytes);

                if let Some(ref root_acc) = proof.root_acc {
                    bytes.push(1);
                    let mut root_acc_bytes = Vec::new();
                    root_acc
                        .serialize_uncompressed(&mut root_acc_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(root_acc_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&root_acc_bytes);
                } else {
                    bytes.push(0);
                }

                if let Some(ref ln_acc_in_root_proof) = proof.ln_acc_in_root_proof {
                    bytes.push(1);
                    let mut witness_bytes = Vec::new();
                    ln_acc_in_root_proof
                        .witness
                        .serialize_uncompressed(&mut witness_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(witness_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&witness_bytes);

                    let mut element_bytes = Vec::new();
                    ln_acc_in_root_proof
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
                // 璇佹槑绫诲瀷鏍囪: 0x03 = QueryProof (NotExists)
                bytes.push(0x03);
                bytes.push(0); // 涓嶅瓨鍦ㄦ爣璁?

                bytes.extend_from_slice(&(proof.key.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&proof.key);

                // 搴忓垪鍖栧墠搴忛敭
                if let Some(ref key_prev) = proof.key_prev {
                    bytes.push(1);
                    bytes.extend_from_slice(&(key_prev.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(key_prev);
                } else {
                    bytes.push(0);
                }

                // 搴忓垪鍖栧悗搴忛敭
                if let Some(ref key_next) = proof.key_next {
                    bytes.push(1);
                    bytes.extend_from_slice(&(key_next.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(key_next);
                } else {
                    bytes.push(0);
                }

                // 搴忓垪鍖栧悗搴忓彾瀛愮疮鍔犲櫒
                if let Some(ref ln_next_acc) = proof.ln_next_acc {
                    bytes.push(1);
                    let mut acc_bytes = Vec::new();
                    ln_next_acc.serialize_uncompressed(&mut acc_bytes).unwrap();
                    bytes.extend_from_slice(&(acc_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&acc_bytes);
                } else {
                    bytes.push(0);
                }

                // 搴忓垪鍖栧墠搴忓湪鍚庡簭涓殑鎴愬憳璇佹槑锛堝鏋滄湁锛?
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

                if let Some(ref next_in_next_proof) = proof.next_in_next_proof {
                    bytes.push(1);
                    let mut witness_bytes = Vec::new();
                    next_in_next_proof
                        .witness
                        .serialize_uncompressed(&mut witness_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(witness_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&witness_bytes);

                    let mut element_bytes = Vec::new();
                    next_in_next_proof
                        .element
                        .serialize_uncompressed(&mut element_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(element_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&element_bytes);
                } else {
                    bytes.push(0);
                }

                if let Some(ref root_acc) = proof.root_acc {
                    bytes.push(1);
                    let mut root_acc_bytes = Vec::new();
                    root_acc
                        .serialize_uncompressed(&mut root_acc_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(root_acc_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&root_acc_bytes);
                } else {
                    bytes.push(0);
                }

                if let Some(ref ln_next_acc_in_root_proof) = proof.ln_next_acc_in_root_proof {
                    bytes.push(1);
                    let mut witness_bytes = Vec::new();
                    ln_next_acc_in_root_proof
                        .witness
                        .serialize_uncompressed(&mut witness_bytes)
                        .unwrap();
                    bytes.extend_from_slice(&(witness_bytes.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(&witness_bytes);

                    let mut element_bytes = Vec::new();
                    ln_next_acc_in_root_proof
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

        // 闄勫甫绱姞鍣ㄥ揩鐓э紝鐢ㄤ簬鏍瑰搱甯岄獙璇?
        Self::append_accumulator_snapshot(snapshot, &mut bytes);

        bytes
    }

    /// 浠?AccTrie 鑾峰彇鏍瑰搱甯?
    /// 鐢变簬 AccTrie 浣跨敤绱姞鍣紝鎴戜滑闇€瑕佷粠鎵€鏈夊彾瀛愯妭鐐圭殑绱姞鍣ㄥ€艰绠楁牴鍝堝笇
    fn get_root_hash(&self) -> RootHash {
        let trie = self.trie.read().unwrap();
        trie.root_hash()
    }

    fn get_root_hash_from_trie(&self, trie: &AccTrie) -> RootHash {
        trie.root_hash()
    }
}

impl Default for AccTrieAds {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AccTrieAds {
    fn drop(&mut self) {
        if self.persistence.is_none() {
            return;
        }

        if let Err(error) = self.flush_all_dirty_pages() {
            debug_log!("AccTrie persistence flush failed during drop: {}", error);
        }

        if let Ok(trie) = self.trie.read() {
            if let Err(error) = self.maybe_persist_manifest_for(&trie, true) {
                debug_log!("AccTrie manifest persistence failed during drop: {}", error);
            }
        }
    }
}

impl AdsOperations for AccTrieAds {
    /// 娣诲姞 (keyword, fid) 瀵瑰埌 AccTrie
    /// 杩斿洖: (proof, root_hash)
    fn add(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        // 鎻掑叆鍒?AccTrie
        let mut trie = self.trie.write().unwrap();
        let key = keyword.as_bytes().to_vec();
        let root_prefix =
            AccTrie::root_prefix_from_hex_prefix(&AccTrie::root_prefix_hex_for_key(&key))
                .expect("keyword root prefix");

        let (proof, _snapshot) = match trie.insert(key, fid.to_string()) {
            Ok(proof) => {
                debug_log!(
                    "馃敡 AccTrie Add: keyword='{}', fid='{}' (success)",
                    keyword,
                    fid
                );
                let snapshot = Self::collect_accumulator_snapshot(&trie);
                (
                    Self::serialize_insertion_proof(&proof, &snapshot, true),
                    snapshot,
                )
            }
            Err(e) => {
                debug_log!(
                    "鉂?AccTrie Add: keyword='{}', fid='{}' failed: {:?}",
                    keyword,
                    fid,
                    e
                );
                let snapshot = Self::collect_accumulator_snapshot(&trie);
                (Vec::new(), snapshot)
            }
        };

        let root_hash = trie.root_hash();
        if let Err(error) = self.persist_root_prefix(&trie, &root_prefix) {
            debug_log!("AccTrie persistence update failed after add: {}", error);
        }

        debug_log!(
            "馃敡 AccTrie Add: proof size={} bytes, root_hash={:02x?}...",
            proof.len(),
            &root_hash[..8.min(root_hash.len())]
        );

        (proof, root_hash)
    }

    /// 鎵归噺娣诲姞 (keyword, fid) 瀵瑰埌 AccTrie
    fn add_batch(&mut self, kvs: Vec<(String, String)>) -> (Vec<u8>, RootHash) {
        // Batch add skips proof generation, but should still use incremental inserts.
        if kvs.is_empty() {
            return (Vec::new(), self.get_root_hash());
        }

        let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
        for (keyword, fid) in kvs {
            let entry = grouped.entry(keyword).or_default();
            if !entry.contains(&fid) {
                entry.push(fid);
            }
        }

        let mut trie = self.trie.write().unwrap();
        let mut touched_prefixes = HashSet::new();
        for (keyword, fids) in grouped {
            let key = keyword.into_bytes();
            let root_prefix =
                AccTrie::root_prefix_from_hex_prefix(&AccTrie::root_prefix_hex_for_key(&key))
                    .unwrap();
            touched_prefixes.insert(root_prefix);
            for fid in fids {
                if let Err(error) = trie.insert(key.clone(), fid) {
                    debug_log!("AccTrie batch add insert failed: {:?}", error);
                }
            }
        }

        for root_prefix in touched_prefixes {
            let _ = self.persist_root_prefix_records(&trie, &root_prefix);
        }
        self.mark_manifest_dirty();
        let _ = self.maybe_persist_manifest_for(&trie, false);
        (Vec::new(), self.get_root_hash_from_trie(&trie))
    }

    /// 鏌ヨ keyword 瀵瑰簲鐨勬墍鏈?fid
    /// 杩斿洖: (fids, proof)
    fn query(&self, keyword: &str) -> (Vec<String>, Vec<u8>) {
        let key = keyword.as_bytes().to_vec();
        let (fids, proof) = {
            let trie = self.trie.read().unwrap();
            let snapshot = Self::collect_accumulator_snapshot(&trie);
            let fully_loaded = self
                .runtime
                .read()
                .map(|runtime| runtime.fully_loaded)
                .unwrap_or(false);
            if fully_loaded || self.persistence.is_none() || trie.map.contains_key(&key) {
                Self::query_from_trie(&trie, &key, &snapshot)
            } else {
                let root_prefix =
                    AccTrie::root_prefix_from_hex_prefix(&AccTrie::root_prefix_hex_for_key(&key))
                        .unwrap();
                let shard_records = self
                    .load_shard_records(&trie, &root_prefix)
                    .unwrap_or_default();
                if !shard_records.iter().any(|record| record.key == key) {
                    return (Vec::new(), Vec::new());
                }
                let mut shard_trie = AccTrie::new();
                if shard_trie.restore_from_records(shard_records).is_ok() {
                    Self::query_from_trie(&shard_trie, &key, &snapshot)
                } else {
                    (Vec::new(), Vec::new())
                }
            }
        };

        if fids.is_empty() {
            debug_log!("馃攳 AccTrie Query: keyword='{}' not found", keyword);
        }

        debug_log!(
            "馃攳 AccTrie Query: keyword='{}', found {} fids",
            keyword,
            fids.len()
        );

        (fids, proof)
    }

    /* stale query proof block removed by query shard loading refactor
                match trie.query(&key, &value) {
                    Ok(result) => {
                        let serialized = Self::serialize_query_result(&result, &snapshot);
                        debug_log!(
                            "馃攳 AccTrie Query: returning proof ({} bytes)",
                            serialized.len()
                        );
                        serialized
                    }
                    Err(e) => {
                        debug_log!("鈿狅笍 AccTrie Query: proof generation failed: {:?}", e);
                        Vec::new()
                    }
                }
            };

            (fids, proof)
        }
    */

    /// 浠?AccTrie 涓垹闄?(keyword, fid) 瀵?
    /// 杩斿洖: (proof, root_hash)
    fn delete(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        let key = keyword.as_bytes().to_vec();
        let root_prefix =
            AccTrie::root_prefix_from_hex_prefix(&AccTrie::root_prefix_hex_for_key(&key))
                .expect("keyword root prefix");

        // 浠?AccTrie 涓垹闄?
        let mut trie = self.trie.write().unwrap();
        let delete_entire = trie
            .map
            .get(&key)
            .map(|values| values.len() == 1)
            .unwrap_or(true);

        let (proof, _snapshot) = if delete_entire {
            // 鍒犻櫎鏁翠釜鍙跺瓙鑺傜偣
            debug_log!(
                "馃棏锔?AccTrie Delete: keyword='{}', fid='{}' (removing entire key)",
                keyword,
                fid
            );
            match trie.delete(&key, None) {
                Ok(proof) => {
                    let snapshot = Self::collect_accumulator_snapshot(&trie);
                    (Self::serialize_deletion_proof(&proof, &snapshot), snapshot)
                }
                Err(e) => {
                    debug_log!("鈿狅笍 AccTrie Delete: delete entire key failed: {:?}", e);
                    let snapshot = Self::collect_accumulator_snapshot(&trie);
                    (Vec::new(), snapshot)
                }
            }
        } else {
            // 鍙垹闄ょ壒瀹氬€?
            debug_log!(
                "馃棏锔?AccTrie Delete: keyword='{}', fid='{}' (key still has values)",
                keyword,
                fid
            );
            match trie.delete(&key, Some(fid.to_string())) {
                Ok(proof) => {
                    let snapshot = Self::collect_accumulator_snapshot(&trie);
                    (Self::serialize_deletion_proof(&proof, &snapshot), snapshot)
                }
                Err(e) => {
                    debug_log!(
                        "鈿狅笍 AccTrie Delete: delete specific value failed: {:?}",
                        e
                    );
                    let snapshot = Self::collect_accumulator_snapshot(&trie);
                    (Vec::new(), snapshot)
                }
            }
        };

        let root_hash = trie.root_hash();
        if let Err(error) = self.persist_root_prefix(&trie, &root_prefix) {
            debug_log!("AccTrie persistence update failed after delete: {}", error);
        }

        debug_log!(
            "馃棏锔?AccTrie Delete: post-delete root_hash={:02x?}...",
            &root_hash[..8.min(root_hash.len())]
        );

        (proof, root_hash)
    }

    fn root_accumulator(&self) -> Vec<u8> {
        let trie = self.trie.read().unwrap();
        trie.root_accumulator_bytes()
    }

    fn record_count(&self) -> usize {
        let trie = self.trie.read().unwrap();
        trie.records().len()
    }

    fn storage_bytes(&self) -> u64 {
        match &self.persistence {
            Some(backend) => backend.storage_bytes(),
            None => {
                let trie = self.trie.read().unwrap();
                let records = trie.records();
                bincode::serialize(&records)
                    .map(|bytes| bytes.len() as u64)
                    .unwrap_or(0)
            }
        }
    }

    fn current_root_hash(&self) -> RootHash {
        self.get_root_hash()
    }

    fn export_prefix_segment(&self, prefix_hex: &str) -> Result<Vec<u8>, String> {
        AccTrieAds::export_prefix_segment(self, prefix_hex)
    }

    fn import_prefix_segment(&mut self, segment: &[u8]) -> Result<RootHash, String> {
        AccTrieAds::import_prefix_segment(self, segment)
    }

    fn drain_prefix_segment(&mut self, prefix_hex: &str) -> Result<(Vec<u8>, RootHash), String> {
        AccTrieAds::drain_prefix_segment(self, prefix_hex)
    }

    fn prepare_retain_prefix_segment(
        &mut self,
        prefix_hex: &str,
    ) -> Result<(Vec<u8>, RootHash), String> {
        AccTrieAds::prepare_retain_prefix_segment(self, prefix_hex)
    }

    fn confirm_prefix_migration(&mut self, prefix_hex: &str) -> Result<RootHash, String> {
        AccTrieAds::confirm_prefix_migration(self, prefix_hex)
    }

    fn reset(&mut self) -> Result<(), String> {
        if let Some(path) = self.persistence_path.clone() {
            let backend = match self.persistence.as_ref() {
                Some(PersistenceBackend::Page(_)) => {
                    let _ = fs::remove_dir_all(&path);
                    PersistenceBackend::Page(PersistenceLayout::new(path))
                }
                Some(PersistenceBackend::KvDb(_)) => {
                    let _ = fs::remove_dir_all(&path);
                    PersistenceBackend::KvDb(KvDbPersistence::new(path)?)
                }
                None => {
                    self.reset_in_memory_state();
                    return Ok(());
                }
            };
            self.persistence = Some(backend);
        }
        self.reset_in_memory_state();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{AdsMode, ProofVerifier};
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("acctrie-{label}-{suffix}"))
    }

    fn keywords_sharing_hashed_prefix(prefix_len: usize, count: usize) -> (String, Vec<String>) {
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for index in 0..20_000usize {
            let keyword = format!("migration-key-{index}");
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

    fn load_minimal_dataset(trie: &mut AccTrie) {
        let dataset = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/input/testdata/records_minimal.csv"
        ));

        for line in dataset.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let mut fields = line.split(',');
            let Some(fid) = fields.next() else {
                continue;
            };

            for keyword in fields {
                trie.insert(keyword.to_string().into_bytes(), fid.to_string())
                    .expect("dataset insert");
            }
        }
    }

    fn load_minimal_dataset_without_bom(trie: &mut AccTrie) {
        let dataset = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/input/testdata/records_minimal.csv"
        ));

        for line in dataset.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let mut fields = line.split(',');
            let Some(fid) = fields.next() else {
                continue;
            };
            let fid = fid.trim_start_matches('\u{feff}').to_string();

            for keyword in fields {
                trie.insert(keyword.to_string().into_bytes(), fid.clone())
                    .expect("dataset insert");
            }
        }
    }

    #[test]
    fn test_acctrie_ads_basic_operations() {
        let mut ads = AccTrieAds::new();

        // Test Add
        let (proof1, root1) = ads.add("rust", "file1");
        assert!(!proof1.is_empty());
        assert_eq!(root1.len(), 32);
        assert_eq!(proof1[0], 0x01); // InsertionProof鏍囪

        let (proof2, root2) = ads.add("rust", "file2");
        assert!(!proof2.is_empty());
        assert_ne!(root1, root2); // Root should change

        // Test Query
        let (fids, proof) = ads.query("rust");
        assert_eq!(fids.len(), 2);
        assert!(fids.contains(&"file1".to_string()));
        assert!(fids.contains(&"file2".to_string()));
        assert!(!proof.is_empty());
        assert_eq!(proof[0], 0x03); // QueryProof鏍囪

        // Test Delete
        let (proof3, root3) = ads.delete("rust", "file1");
        assert!(!proof3.is_empty());
        assert_eq!(proof3[0], 0x02); // DeletionProof鏍囪
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
    fn test_acctrie_delete_proof_verifies_for_minimal_dataset() {
        let mut trie = AccTrie::new();
        load_minimal_dataset(&mut trie);

        let fid = "\u{feff}fid001".to_string();
        let proof = trie
            .delete(&b"rust".to_vec(), Some(fid.clone()))
            .expect("delete proof");

        assert!(proof.value_in_ln_old_proof.is_some());
        assert!(proof.keyp_in_ln_proof.is_some());
        assert!(proof.key_in_ln_next_old_proof.is_some());
        assert!(proof.keyp_in_ln_next_new_proof.is_some());
        assert!(proof
            .value_in_ln_old_proof
            .as_ref()
            .unwrap()
            .verify(proof.ln_acc_old));
        assert!(proof
            .keyp_in_ln_proof
            .as_ref()
            .unwrap()
            .verify(proof.ln_acc_old));
        assert!(proof
            .key_in_ln_next_old_proof
            .as_ref()
            .unwrap()
            .verify(proof.ln_next_acc_old.expect("old next acc")));
        assert!(proof
            .keyp_in_ln_next_new_proof
            .as_ref()
            .unwrap()
            .verify(proof.ln_next_acc_new.expect("new next acc")));

        let mut ads = AccTrieAds::new();
        {
            let mut guard = ads.trie.write().unwrap();
            load_minimal_dataset(&mut guard);
        }
        let (serialized_proof, root_hash) = ads.delete("rust", &fid);
        let verifier = ProofVerifier::new(AdsMode::AccTrie);
        assert!(
            verifier.verify(&serialized_proof, &root_hash),
            "serialized delete proof should verify"
        );
    }

    #[test]
    fn test_acctrie_delete_proof_verifies_for_minimal_dataset_without_bom() {
        let mut trie = AccTrie::new();
        load_minimal_dataset_without_bom(&mut trie);

        let proof = trie
            .delete(&b"rust".to_vec(), Some("fid001".to_string()))
            .expect("delete proof");

        assert!(proof
            .value_in_ln_old_proof
            .as_ref()
            .unwrap()
            .verify(proof.ln_acc_old));

        let mut ads = AccTrieAds::new();
        {
            let mut guard = ads.trie.write().unwrap();
            load_minimal_dataset_without_bom(&mut guard);
        }
        let (serialized_proof, root_hash) = ads.delete("rust", "fid001");
        let verifier = ProofVerifier::new(AdsMode::AccTrie);
        assert!(
            verifier.verify(&serialized_proof, &root_hash),
            "serialized delete proof should verify without BOM"
        );
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

        // 鍒濆鏍瑰搱甯?
        let root0 = ads.get_root_hash();

        // 娣诲姞鍚庢牴鍝堝笇搴旇鏀瑰彉
        let (_, root1) = ads.add("key1", "val1");
        assert_ne!(root0, root1, "Root should change after insertion");

        // 鍐嶆娣诲姞
        let (_, root2) = ads.add("key2", "val2");
        assert_ne!(root1, root2, "Root should change after another insertion");

        // 鍒犻櫎鍚庢牴鍝堝笇搴旇鏀瑰彉
        let (_, root3) = ads.delete("key1", "val1");
        assert_ne!(root2, root3, "Root should change after deletion");

        // 鍒犻櫎鎵€鏈夊悗鏍瑰搱甯屽簲璇ユ帴杩戝垵濮嬬姸鎬侊紙浣嗗彲鑳戒笉瀹屽叏鐩稿悓锛?
        let (_, root4) = ads.delete("key2", "val2");
        assert_ne!(root3, root4, "Root should change after final deletion");
    }

    #[test]
    fn test_acctrie_proof_types() {
        let mut ads = AccTrieAds::new();

        // 娴嬭瘯鎻掑叆璇佹槑绫诲瀷
        let (insert_proof, _) = ads.add("item", "data");
        assert_eq!(insert_proof[0], 0x01);

        // 娴嬭瘯鏌ヨ璇佹槑绫诲瀷锛堝瓨鍦級
        let (_, query_proof) = ads.query("item");
        assert_eq!(query_proof[0], 0x03);
        assert_eq!(query_proof[1], 1); // 瀛樺湪鏍囪

        // 娴嬭瘯鍒犻櫎璇佹槑绫诲瀷
        let (delete_proof, _) = ads.delete("item", "data");
        assert_eq!(delete_proof[0], 0x02);

        // 娴嬭瘯鏌ヨ璇佹槑绫诲瀷锛堜笉瀛樺湪锛?
        let (fids, query_proof_not_exist) = ads.query("nonexistent");
        assert_eq!(fids.len(), 0);
        // 涓嶅瓨鍦ㄦ椂杩斿洖绌鸿瘉鏄?
        assert!(!query_proof_not_exist.is_empty());
        assert_eq!(query_proof_not_exist[0], 0x03);
        assert_eq!(query_proof_not_exist[1], 0);
    }

    #[test]
    fn test_acctrie_persistence_restores_records() {
        let dir = unique_temp_dir("restore");
        let mut ads = AccTrieAds::new_with_persistence(dir.clone());

        ads.add("rust", "file1");
        ads.add("storage", "file2");

        drop(ads);

        let ads = AccTrieAds::new_with_persistence(dir.clone());
        let (rust_fids, _) = ads.query("rust");
        let (storage_fids, _) = ads.query("storage");

        assert_eq!(rust_fids, vec!["file1".to_string()]);
        assert_eq!(storage_fids, vec!["file2".to_string()]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_acctrie_kvdb_persistence_restores_records() {
        let dir = unique_temp_dir("kvdb-restore");
        let mut ads = AccTrieAds::new_with_kvdb_persistence(dir.clone());

        ads.add("rust", "file1");
        ads.add("storage", "file2");

        drop(ads);

        let ads = AccTrieAds::new_with_kvdb_persistence(dir.clone());
        let (rust_fids, _) = ads.query("rust");
        let (storage_fids, _) = ads.query("storage");

        assert_eq!(rust_fids, vec!["file1".to_string()]);
        assert_eq!(storage_fids, vec!["file2".to_string()]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_acctrie_prefix_segment_drain_and_import() {
        let source_dir = unique_temp_dir("source");
        let target_dir = unique_temp_dir("target");
        let mut source = AccTrieAds::new_with_persistence(source_dir.clone());
        let mut target = AccTrieAds::new_with_persistence(target_dir.clone());

        let (prefix_hex, keywords) = keywords_sharing_hashed_prefix(2, 2);
        let moved_a = &keywords[0];
        let moved_b = &keywords[1];

        let outsider = (0..20_000usize)
            .map(|index| format!("outsider-{index}"))
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

        assert_eq!(moved_a_on_target.len(), 2);
        assert!(moved_a_on_target.contains(&"fa".to_string()));
        assert!(moved_a_on_target.contains(&"fb".to_string()));
        assert_eq!(moved_b_on_target, vec!["fc".to_string()]);

        let _ = fs::remove_dir_all(source_dir);
        let _ = fs::remove_dir_all(target_dir);
    }

    #[test]
    fn test_acctrie_prefix_migration_rejects_empty_prefix() {
        let mut ads = AccTrieAds::new();
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
