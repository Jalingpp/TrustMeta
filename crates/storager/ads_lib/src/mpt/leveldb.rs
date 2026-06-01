use super::error::MPTError;
use super::mpt::MPTMetadata;
use super::node::Database;
use rusty_leveldb::{in_memory, Options, DB};
use std::path::Path;
use crate::io_stats;

pub const MPT_METADATA_KEY: &[u8] = b"mpt:metadata";
pub const MPT_ROOT_HASH_KEY: &[u8] = b"mpt:root_hash";

pub struct LevelDbDatabase {
    db: DB,
}

impl LevelDbDatabase {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MPTError> {
        let mut options = Options::default();
        options.create_if_missing = true;
        let db = DB::open(path.as_ref(), options)
            .map_err(|error| MPTError::DatabaseError(format!("failed to open leveldb: {error}")))?;
        Ok(Self { db })
    }

    pub fn open_in_memory() -> Result<Self, MPTError> {
        let db = DB::open("", in_memory()).map_err(|error| {
            MPTError::DatabaseError(format!("failed to open in-memory leveldb: {error}"))
        })?;
        Ok(Self { db })
    }

    pub fn load_metadata(&mut self) -> Result<Option<MPTMetadata>, MPTError> {
        match self.get(MPT_METADATA_KEY)? {
            Some(bytes) => {
                let metadata =
                    serde_json::from_slice(&bytes).map_err(MPTError::SerializationError)?;
                Ok(Some(metadata))
            }
            None => Ok(None),
        }
    }

    pub fn store_metadata(&mut self, metadata: &MPTMetadata) -> Result<(), MPTError> {
        let bytes = serde_json::to_vec(metadata).map_err(MPTError::SerializationError)?;
        self.put(MPT_METADATA_KEY, &bytes)?;
        self.put(MPT_ROOT_HASH_KEY, &metadata.root_hash)
    }
}

impl Database for LevelDbDatabase {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, MPTError> {
        let value = self.db.get(key);
        let bytes = key.len() + value.as_ref().map(|v| v.len()).unwrap_or(0);
        io_stats::record_read(bytes);
        Ok(value)
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), MPTError> {
        io_stats::record_write(key.len() + value.len());
        self.db
            .put(key, value)
            .map_err(|error| MPTError::DatabaseError(format!("leveldb put failed: {error}")))
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), MPTError> {
        io_stats::record_write(key.len());
        self.db
            .delete(key)
            .map_err(|error| MPTError::DatabaseError(format!("leveldb delete failed: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leveldb_roundtrip() {
        let mut db = LevelDbDatabase::open_in_memory().unwrap();
        db.put(b"k", b"v").unwrap();
        assert_eq!(db.get(b"k").unwrap(), Some(b"v".to_vec()));
    }

    #[test]
    fn test_leveldb_metadata_roundtrip() {
        let mut db = LevelDbDatabase::open_in_memory().unwrap();
        let metadata = MPTMetadata::new([7u8; 32]);
        db.store_metadata(&metadata).unwrap();

        let loaded = db.load_metadata().unwrap().unwrap();
        assert_eq!(loaded.root_hash, metadata.root_hash);
        assert_eq!(
            db.get(MPT_ROOT_HASH_KEY).unwrap(),
            Some(metadata.root_hash.to_vec())
        );
    }
}
