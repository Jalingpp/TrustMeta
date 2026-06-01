use accumulator_ads::G1Affine;
use accumulator_ads::Set;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use lru::LruCache;
use rusty_leveldb::{DB, Options};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::node::Node;
use crate::utils::Hash;

pub const ACCTREE_STORAGE_FORMAT_VERSION: u32 = 1;
pub const ACCTREE_MANIFEST_KEY: &[u8] = &[
    97, 99, 99, 116, 114, 101, 101, 58, 109, 97, 110, 105, 102, 101, 115, 116,
];
const DEFAULT_NODE_CACHE_LIMIT: usize = 256;

#[derive(Debug)]
pub enum AccTreeStorageError {
    Open(String),
    Put(String),
    Delete(String),
    Bincode(Box<bincode::ErrorKind>),
    CorruptedAccumulator,
    MissingNode(Hash),
    UnsupportedVersion(u32),
}

impl fmt::Display for AccTreeStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            std::str::from_utf8(&[
                97, 99, 99, 116, 114, 101, 101, 32, 115, 116, 111, 114, 97, 103, 101, 32, 101, 114,
                114, 111, 114,
            ])
            .unwrap(),
        )
    }
}

impl std::error::Error for AccTreeStorageError {}

impl From<Box<bincode::ErrorKind>> for AccTreeStorageError {
    fn from(value: Box<bincode::ErrorKind>) -> Self {
        Self::Bincode(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PersistedAccTreeNode {
    Leaf {
        hash: Hash,
        key: String,
        fid: String,
        level: usize,
    },
    NonLeaf {
        hash: Hash,
        keys: Vec<String>,
        acc_bytes: Vec<u8>,
        level: usize,
        left_hash: Hash,
        right_hash: Hash,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersistedAccTreeManifest {
    pub version: u32,
    pub root_hashes: Vec<Hash>,
    pub global_state_hash: Hash,
}

#[derive(Clone)]
struct CachedNode {
    node: PersistedAccTreeNode,
    dirty: bool,
}

pub struct AccTreeLevelDb {
    path: PathBuf,
}

impl AccTreeLevelDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AccTreeStorageError> {
        let this = Self {
            path: path.as_ref().to_path_buf(),
        };
        let _ = this.open_db()?;
        Ok(this)
    }

    pub fn get_raw(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        self.open_db().ok().and_then(|mut db| db.get(key))
    }

    pub fn put_raw(&mut self, key: &[u8], value: &[u8]) -> Result<(), AccTreeStorageError> {
        self.open_db()?
            .put(key, value)
            .map_err(|error| AccTreeStorageError::Put(error.to_string()))
    }

    pub fn delete_raw(&mut self, key: &[u8]) -> Result<(), AccTreeStorageError> {
        self.open_db()?
            .delete(key)
            .map_err(|error| AccTreeStorageError::Delete(error.to_string()))
    }

    fn open_db(&self) -> Result<DB, AccTreeStorageError> {
        let mut options = Options::default();
        options.create_if_missing = true;
        DB::open(&self.path, options).map_err(|error| AccTreeStorageError::Open(error.to_string()))
    }
}

pub struct AccTreeWriteBackCache {
    db: AccTreeLevelDb,
    nodes: Mutex<LruCache<Hash, CachedNode>>,
}

pub fn default_cache_limit() -> usize {
    DEFAULT_NODE_CACHE_LIMIT
}

impl PersistedAccTreeNode {
    pub fn hash(&self) -> Hash {
        match self {
            Self::Leaf { hash, .. } => *hash,
            Self::NonLeaf { hash, .. } => *hash,
        }
    }

    pub fn child_hashes(&self) -> Option<(Hash, Hash)> {
        match self {
            Self::Leaf { .. } => None,
            Self::NonLeaf {
                left_hash,
                right_hash,
                ..
            } => Some((*left_hash, *right_hash)),
        }
    }

    pub fn from_node(node: &Node) -> Result<Self, AccTreeStorageError> {
        match node {
            Node::Leaf {
                hash,
                key,
                fid,
                level,
            } => Ok(Self::Leaf {
                hash: *hash,
                key: key.clone(),
                fid: fid.clone(),
                level: *level,
            }),
            Node::NonLeaf {
                hash,
                keys,
                acc,
                level,
                left,
                right,
            } => {
                let mut acc_bytes = Vec::new();
                acc.serialize(&mut acc_bytes)
                    .map_err(|_| AccTreeStorageError::CorruptedAccumulator)?;
                let mut keys_vec = keys.iter().cloned().collect::<Vec<_>>();
                keys_vec.sort();
                Ok(Self::NonLeaf {
                    hash: *hash,
                    keys: keys_vec,
                    acc_bytes,
                    level: *level,
                    left_hash: *left.hash(),
                    right_hash: *right.hash(),
                })
            }
        }
    }
}

impl AccTreeLevelDb {
    pub fn load_manifest(
        &mut self,
    ) -> Result<Option<PersistedAccTreeManifest>, AccTreeStorageError> {
        match self.get_raw(ACCTREE_MANIFEST_KEY) {
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn store_manifest(
        &mut self,
        manifest: &PersistedAccTreeManifest,
    ) -> Result<(), AccTreeStorageError> {
        let bytes = bincode::serialize(manifest)?;
        self.put_raw(ACCTREE_MANIFEST_KEY, &bytes)
    }

    pub fn load_node(
        &mut self,
        hash: &Hash,
    ) -> Result<Option<PersistedAccTreeNode>, AccTreeStorageError> {
        match self.get_raw(hash) {
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn store_node(&mut self, node: &PersistedAccTreeNode) -> Result<Hash, AccTreeStorageError> {
        let hash = node.hash();
        let bytes = bincode::serialize(node)?;
        self.put_raw(&hash, &bytes)?;
        Ok(hash)
    }
}

impl AccTreeWriteBackCache {
    pub fn open(path: impl AsRef<Path>, cache_limit: usize) -> Result<Self, AccTreeStorageError> {
        Ok(Self {
            db: AccTreeLevelDb::open(path)?,
            nodes: Mutex::new(LruCache::new(
                NonZeroUsize::new(cache_limit.max(1)).unwrap(),
            )),
        })
    }

    pub fn cache_node(
        &self,
        node: PersistedAccTreeNode,
        dirty: bool,
    ) -> Result<Hash, AccTreeStorageError> {
        let mut nodes = self.nodes.lock().unwrap();
        let hash = node.hash();
        if let Some(cached) = nodes.get_mut(&hash) {
            cached.node = node;
            cached.dirty |= dirty;
            return Ok(hash);
        }

        self.evict_if_needed_locked(&mut nodes)?;
        nodes.put(hash, CachedNode { node, dirty });
        Ok(hash)
    }

    pub fn load_node(
        &self,
        hash: &Hash,
    ) -> Result<Option<PersistedAccTreeNode>, AccTreeStorageError> {
        {
            let mut nodes = self.nodes.lock().unwrap();
            if let Some(cached) = nodes.get(hash) {
                return Ok(Some(cached.node.clone()));
            }
        }

        let mut db = AccTreeLevelDb::open(&self.db.path)?;
        let loaded = db.load_node(hash)?;
        if let Some(node) = loaded.clone() {
            let _ = self.cache_node(node, false);
        }
        Ok(loaded)
    }

    pub fn load_manifest(&self) -> Result<Option<PersistedAccTreeManifest>, AccTreeStorageError> {
        let mut db = AccTreeLevelDb::open(&self.db.path)?;
        db.load_manifest()
    }

    pub fn flush_all(&self) -> Result<(), AccTreeStorageError> {
        let mut nodes = self.nodes.lock().unwrap();
        while let Some((_hash, cached)) = nodes.pop_lru() {
            if cached.dirty {
                let mut db = AccTreeLevelDb::open(&self.db.path)?;
                db.store_node(&cached.node)?;
            }
        }
        Ok(())
    }

    pub fn persist_manifest(
        &self,
        manifest: &PersistedAccTreeManifest,
    ) -> Result<(), AccTreeStorageError> {
        self.flush_all()?;
        let mut db = AccTreeLevelDb::open(&self.db.path)?;
        db.store_manifest(manifest)
    }

    fn evict_if_needed_locked(
        &self,
        nodes: &mut LruCache<Hash, CachedNode>,
    ) -> Result<(), AccTreeStorageError> {
        while nodes.len() >= nodes.cap().get() {
            if let Some((_hash, cached)) = nodes.pop_lru() {
                if cached.dirty {
                    let mut db = AccTreeLevelDb::open(&self.db.path)?;
                    db.store_node(&cached.node)?;
                }
            } else {
                break;
            }
        }
        Ok(())
    }
}

pub fn persisted_nodes_by_hash(
    roots: &[Box<Node>],
) -> Result<HashMap<Hash, PersistedAccTreeNode>, AccTreeStorageError> {
    let mut persisted = HashMap::new();
    for root in roots {
        collect_persisted_nodes(root, &mut persisted)?;
    }
    Ok(persisted)
}

pub fn restore_roots_from_manifest(
    cache: &mut AccTreeWriteBackCache,
    manifest: &PersistedAccTreeManifest,
) -> Result<Vec<Box<Node>>, AccTreeStorageError> {
    if manifest.version != ACCTREE_STORAGE_FORMAT_VERSION {
        return Err(AccTreeStorageError::UnsupportedVersion(manifest.version));
    }

    let mut roots = Vec::new();
    for root_hash in &manifest.root_hashes {
        roots.push(restore_node(cache, root_hash)?);
    }
    Ok(roots)
}

fn collect_persisted_nodes(
    node: &Node,
    persisted: &mut HashMap<Hash, PersistedAccTreeNode>,
) -> Result<(), AccTreeStorageError> {
    let persisted_node = PersistedAccTreeNode::from_node(node)?;
    if persisted
        .insert(persisted_node.hash(), persisted_node)
        .is_some()
    {
        return Ok(());
    }

    if let Node::NonLeaf { left, right, .. } = node {
        collect_persisted_nodes(left, persisted)?;
        collect_persisted_nodes(right, persisted)?;
    }

    Ok(())
}

fn restore_node(
    cache: &mut AccTreeWriteBackCache,
    hash: &Hash,
) -> Result<Box<Node>, AccTreeStorageError> {
    let persisted = cache
        .load_node(hash)?
        .ok_or(AccTreeStorageError::MissingNode(*hash))?;

    match persisted {
        PersistedAccTreeNode::Leaf {
            hash,
            key,
            fid,
            level,
        } => Ok(Box::new(Node::Leaf {
            hash,
            key,
            fid,
            level,
        })),
        PersistedAccTreeNode::NonLeaf {
            hash,
            keys,
            acc_bytes,
            level,
            left_hash,
            right_hash,
        } => {
            let left = restore_node(cache, &left_hash)?;
            let right = restore_node(cache, &right_hash)?;
            let acc = G1Affine::deserialize(&acc_bytes[..])
                .map_err(|_| AccTreeStorageError::CorruptedAccumulator)?;
            Ok(Box::new(Node::NonLeaf {
                hash,
                keys: Arc::new(Set::from_vec(keys)),
                acc,
                level,
                left,
                right,
            }))
        }
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
        std::env::temp_dir().join(format!("acctree-persist-{tag}-{nanos}"))
    }

    #[test]
    fn test_leveldb_manifest_roundtrip() {
        let dir = unique_temp_dir("manifest");
        let mut db = AccTreeLevelDb::open(&dir).unwrap();
        let manifest = PersistedAccTreeManifest {
            version: ACCTREE_STORAGE_FORMAT_VERSION,
            root_hashes: vec![[1u8; 32], [2u8; 32]],
            global_state_hash: [3u8; 32],
        };

        db.store_manifest(&manifest).unwrap();
        assert_eq!(db.load_manifest().unwrap(), Some(manifest));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_cache_eviction_writes_dirty_node() {
        let dir = unique_temp_dir("cache");
        let cache = AccTreeWriteBackCache::open(&dir, 1).unwrap();
        let left = PersistedAccTreeNode::Leaf {
            hash: [7u8; 32],
            key: String::from_iter(['k', '1']),
            fid: String::from_iter(['f', '1']),
            level: 0,
        };
        let right = PersistedAccTreeNode::Leaf {
            hash: [8u8; 32],
            key: String::from_iter(['k', '2']),
            fid: String::from_iter(['f', '2']),
            level: 0,
        };

        let left_hash = cache.cache_node(left.clone(), true).unwrap();
        cache.cache_node(right, true).unwrap();

        assert_eq!(cache.load_node(&left_hash).unwrap(), Some(left));
        let _ = fs::remove_dir_all(dir);
    }
}
