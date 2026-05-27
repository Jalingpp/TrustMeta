use super::persistence::{
    MestObjectHash, PersistedBucket, PersistedMestManifest, PersistedMestObject,
    PersistedMgtNode, PersistedSehDirectory,
};
use rusty_leveldb::{in_memory, DB, Options};
use std::fmt;
use std::path::Path;
use thiserror::Error;

pub const MEST_MANIFEST_KEY: &[u8] = &[109, 101, 115, 116, 58, 109, 97, 110, 105, 102, 101, 115, 116];

#[derive(Debug, Error)]
pub enum MestStorageError {
    Open(String),
    Put(String),
    Delete(String),
    Serialization(#[from] Box<bincode::ErrorKind>),
    ObjectTypeMismatch {
        expected: &'static str,
        actual: &'static str,
    },
}

impl fmt::Display for MestStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            _ => f.write_str(std::str::from_utf8(&[77, 101, 115, 116, 83, 116, 111, 114, 97, 103, 101, 69, 114, 114, 111, 114]).unwrap()),
        }
    }
}

pub struct LevelDbDatabase {
    db: DB,
}

impl LevelDbDatabase {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MestStorageError> {
        let mut options = Options::default();
        options.create_if_missing = true;
        let db = DB::open(path.as_ref(), options)
            .map_err(|error| MestStorageError::Open(error.to_string()))?;
        Ok(Self { db })
    }

    pub fn open_in_memory() -> Result<Self, MestStorageError> {
        let db = DB::open(String::new(), in_memory())
            .map_err(|error| MestStorageError::Open(error.to_string()))?;
        Ok(Self { db })
    }

    pub fn get_raw(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, MestStorageError> {
        Ok(self.db.get(key))
    }

    pub fn put_raw(&mut self, key: &[u8], value: &[u8]) -> Result<(), MestStorageError> {
        self.db
            .put(key, value)
            .map_err(|error| MestStorageError::Put(error.to_string()))
    }

    pub fn delete_raw(&mut self, key: &[u8]) -> Result<(), MestStorageError> {
        self.db
            .delete(key)
            .map_err(|error| MestStorageError::Delete(error.to_string()))
    }

    pub fn load_manifest(&mut self) -> Result<Option<PersistedMestManifest>, MestStorageError> {
        match self.db.get(MEST_MANIFEST_KEY) {
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn store_manifest(&mut self, manifest: &PersistedMestManifest) -> Result<(), MestStorageError> {
        let bytes = bincode::serialize(manifest)?;
        self.put_raw(MEST_MANIFEST_KEY, &bytes)
    }

    pub fn delete_manifest(&mut self) -> Result<(), MestStorageError> {
        self.delete_raw(MEST_MANIFEST_KEY)
    }

    pub fn load_object(&mut self, hash: &MestObjectHash) -> Result<Option<PersistedMestObject>, MestStorageError> {
        match self.db.get(hash) {
            Some(bytes) => Ok(Some(PersistedMestObject::from_bytes(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn store_object(&mut self, object: &PersistedMestObject) -> Result<MestObjectHash, MestStorageError> {
        let hash = object.object_hash()?;
        let bytes = object.to_bytes()?;
        self.put_raw(&hash, &bytes)?;
        Ok(hash)
    }

    pub fn delete_object(&mut self, hash: &MestObjectHash) -> Result<(), MestStorageError> {
        self.delete_raw(hash)
    }

    pub fn load_bucket(&mut self, hash: &MestObjectHash) -> Result<Option<PersistedBucket>, MestStorageError> {
        match self.load_object(hash)? {
            Some(PersistedMestObject::Bucket(bucket)) => Ok(Some(bucket)),
            Some(other) => Err(MestStorageError::ObjectTypeMismatch {
                expected: unsafe { std::str::from_utf8_unchecked(&[98, 117, 99, 107, 101, 116]) },
                actual: other.kind_name(),
            }),
            None => Ok(None),
        }
    }

    pub fn store_bucket(&mut self, bucket: &PersistedBucket) -> Result<MestObjectHash, MestStorageError> {
        let object = PersistedMestObject::Bucket(bucket.clone().canonicalized());
        self.store_object(&object)
    }

    pub fn load_mgt_node(&mut self, hash: &MestObjectHash) -> Result<Option<PersistedMgtNode>, MestStorageError> {
        match self.load_object(hash)? {
            Some(PersistedMestObject::MgtNode(node)) => Ok(Some(node)),
            Some(other) => Err(MestStorageError::ObjectTypeMismatch {
                expected: unsafe { std::str::from_utf8_unchecked(&[109, 103, 116, 45, 110, 111, 100, 101]) },
                actual: other.kind_name(),
            }),
            None => Ok(None),
        }
    }

    pub fn store_mgt_node(&mut self, node: &PersistedMgtNode) -> Result<MestObjectHash, MestStorageError> {
        self.store_object(&PersistedMestObject::MgtNode(node.clone()))
    }

    pub fn load_seh_directory(
        &mut self,
        hash: &MestObjectHash,
    ) -> Result<Option<PersistedSehDirectory>, MestStorageError> {
        match self.load_object(hash)? {
            Some(PersistedMestObject::SehDirectory(directory)) => Ok(Some(directory)),
            Some(other) => Err(MestStorageError::ObjectTypeMismatch {
                expected: unsafe { std::str::from_utf8_unchecked(&[115, 101, 104, 45, 100, 105, 114, 101, 99, 116, 111, 114, 121]) },
                actual: other.kind_name(),
            }),
            None => Ok(None),
        }
    }

    pub fn store_seh_directory(
        &mut self,
        directory: &PersistedSehDirectory,
    ) -> Result<MestObjectHash, MestStorageError> {
        self.store_object(&PersistedMestObject::SehDirectory(directory.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mest::persistence::{PersistedBucketSegment, MEST_STORAGE_FORMAT_VERSION};
    use crate::mest::KVPair;
    use std::collections::BTreeMap;

    fn s(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn sample_bucket() -> PersistedBucket {
        PersistedBucket {
            version: MEST_STORAGE_FORMAT_VERSION,
            bucket_key: vec![1],
            ld: 1,
            rdx: 16,
            capacity: 100,
            number: 1,
            seg_num: 2,
            segments: vec![PersistedBucketSegment {
                seg_key: s(&[97, 98]),
                kv_pairs: vec![KVPair::new(s(&[97, 108, 112, 104, 97]), s(&[102, 49]))],
            }],
            segment_roots: BTreeMap::from([(s(&[97, 98]), [7u8; 32])]),
            latch_timestamp: 11,
            delegation_list: BTreeMap::new(),
            pending_num: 0,
            to_del_map: BTreeMap::new(),
        }
    }

    #[test]
    fn test_leveldb_bucket_roundtrip_by_object_hash() {
        let mut db = LevelDbDatabase::open_in_memory().unwrap();
        let bucket = sample_bucket();

        let hash = db.store_bucket(&bucket).unwrap();
        let restored = db.load_bucket(&hash).unwrap().unwrap();

        assert_eq!(restored, bucket);
        assert_eq!(hash, bucket.object_hash().unwrap());
    }

    #[test]
    fn test_leveldb_manifest_roundtrip() {
        let mut db = LevelDbDatabase::open_in_memory().unwrap();
        let manifest = PersistedMestManifest {
            version: MEST_STORAGE_FORMAT_VERSION,
            rdx: 16,
            bucket_capacity: 100,
            bucket_seg_num: 2,
            seh_directory_hash: Some([1u8; 32]),
            mgt_root_object_hash: Some([2u8; 32]),
            mgt_root_hash: [3u8; 32],
            current_root_hash: [4u8; 32],
        };

        db.store_manifest(&manifest).unwrap();
        let restored = db.load_manifest().unwrap().unwrap();

        assert_eq!(restored, manifest);
    }

    #[test]
    fn test_leveldb_type_checked_load() {
        let mut db = LevelDbDatabase::open_in_memory().unwrap();
        let bucket_hash = db.store_bucket(&sample_bucket()).unwrap();

        let error = db.load_mgt_node(&bucket_hash).unwrap_err();
        assert!(matches!(error, MestStorageError::ObjectTypeMismatch { .. }));
    }
}
