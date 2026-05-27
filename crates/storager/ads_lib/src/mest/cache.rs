use super::leveldb::{LevelDbDatabase, MestStorageError};
use super::persistence::{MestObjectHash, PersistedMestManifest, PersistedMestObject};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::Path;

const DEFAULT_OBJECT_CACHE_LIMIT: usize = 128;

#[derive(Clone)]
struct CachedObject {
    object: PersistedMestObject,
    dirty: bool,
}

pub struct MestWriteBackCache {
    db: LevelDbDatabase,
    objects: LruCache<MestObjectHash, CachedObject>,
}

pub fn default_cache_limit() -> usize {
    DEFAULT_OBJECT_CACHE_LIMIT
}

impl MestWriteBackCache {
    pub fn open_in_memory(cache_limit: usize) -> Result<Self, MestStorageError> {
        Ok(Self {
            db: LevelDbDatabase::open_in_memory()?,
            objects: LruCache::new(NonZeroUsize::new(cache_limit.max(1)).unwrap()),
        })
    }

    pub fn open(path: impl AsRef<Path>, cache_limit: usize) -> Result<Self, MestStorageError> {
        Ok(Self {
            db: LevelDbDatabase::open(path)?,
            objects: LruCache::new(NonZeroUsize::new(cache_limit.max(1)).unwrap()),
        })
    }

    pub fn cache_object(
        &mut self,
        object: PersistedMestObject,
        dirty: bool,
    ) -> Result<MestObjectHash, MestStorageError> {
        let object = match object {
            PersistedMestObject::Bucket(bucket) => {
                PersistedMestObject::Bucket(bucket.canonicalized())
            }
            other => other,
        };
        let hash = object.object_hash()?;
        if let Some(cached) = self.objects.get_mut(&hash) {
            cached.object = object;
            cached.dirty |= dirty;
            return Ok(hash);
        }

        self.evict_if_needed()?;
        self.objects.put(hash, CachedObject { object, dirty });
        Ok(hash)
    }

    pub fn load_object(
        &mut self,
        hash: &MestObjectHash,
    ) -> Result<Option<PersistedMestObject>, MestStorageError> {
        if let Some(cached) = self.objects.get(hash) {
            return Ok(Some(cached.object.clone()));
        }
        self.db.load_object(hash)
    }

    pub fn load_manifest(&mut self) -> Result<Option<PersistedMestManifest>, MestStorageError> {
        self.db.load_manifest()
    }

    pub fn flush_all(&mut self) -> Result<(), MestStorageError> {
        while let Some((_hash, cached)) = self.objects.pop_lru() {
            if cached.dirty {
                self.db.store_object(&cached.object)?;
            }
        }
        Ok(())
    }

    pub fn persist_manifest(
        &mut self,
        manifest: &PersistedMestManifest,
    ) -> Result<(), MestStorageError> {
        self.flush_all()?;
        self.db.store_manifest(manifest)
    }

    fn evict_if_needed(&mut self) -> Result<(), MestStorageError> {
        while self.objects.len() >= self.objects.cap().get() {
            if let Some((_hash, cached)) = self.objects.pop_lru() {
                if cached.dirty {
                    self.db.store_object(&cached.object)?;
                }
            } else {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mest::persistence::{PersistedBucket, PersistedBucketSegment, MEST_STORAGE_FORMAT_VERSION};
    use crate::mest::KVPair;
    use std::collections::BTreeMap;

    fn s(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn sample_bucket(tag: u8) -> PersistedBucket {
        PersistedBucket {
            version: MEST_STORAGE_FORMAT_VERSION,
            bucket_key: vec![tag as i32],
            ld: 1,
            rdx: 16,
            capacity: 100,
            number: 1,
            seg_num: 2,
            segments: vec![PersistedBucketSegment {
                seg_key: s(&[tag]),
                kv_pairs: vec![KVPair::new(s(&[tag]), s(&[tag.wrapping_add(1)]))],
            }],
            segment_roots: BTreeMap::from([(s(&[tag]), [tag; 32])]),
            latch_timestamp: tag as i64,
            delegation_list: BTreeMap::new(),
            pending_num: 0,
            to_del_map: BTreeMap::new(),
        }
    }

    #[test]
    fn test_cache_eviction_writes_dirty_object_to_leveldb() {
        let mut cache = MestWriteBackCache::open_in_memory(1).unwrap();
        let left = PersistedMestObject::Bucket(sample_bucket(97));
        let right = PersistedMestObject::Bucket(sample_bucket(98));

        let left_hash = cache.cache_object(left.clone(), true).unwrap();
        let _right_hash = cache.cache_object(right, true).unwrap();

        let restored = cache.load_object(&left_hash).unwrap().unwrap();
        assert_eq!(restored, left);
    }

    #[test]
    fn test_persist_manifest_flushes_cached_objects_first() {
        let mut cache = MestWriteBackCache::open_in_memory(2).unwrap();
        let bucket = PersistedMestObject::Bucket(sample_bucket(99));
        let bucket_hash = cache.cache_object(bucket.clone(), true).unwrap();
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

        cache.persist_manifest(&manifest).unwrap();

        assert_eq!(cache.load_manifest().unwrap().unwrap(), manifest);
        assert_eq!(cache.load_object(&bucket_hash).unwrap().unwrap(), bucket);
    }
}
