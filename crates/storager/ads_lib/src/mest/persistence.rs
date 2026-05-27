use super::kvpair::KVPair;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const MEST_STORAGE_FORMAT_VERSION: u32 = 1;

pub type MestObjectHash = [u8; 32];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedBucketSegment {
    pub seg_key: String,
    pub kv_pairs: Vec<KVPair>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedBucket {
    pub version: u32,
    pub bucket_key: Vec<i32>,
    pub ld: i32,
    pub rdx: i32,
    pub capacity: i32,
    pub number: i32,
    pub seg_num: i32,
    pub segments: Vec<PersistedBucketSegment>,
    pub segment_roots: BTreeMap<String, MestObjectHash>,
    pub latch_timestamp: i64,
    pub delegation_list: BTreeMap<String, BTreeMap<String, bool>>,
    pub pending_num: i32,
    pub to_del_map: BTreeMap<String, BTreeMap<String, i32>>,
}

impl PersistedBucket {
    pub fn canonicalized(mut self) -> Self {
        self.segments.sort_by(|left, right| left.seg_key.cmp(&right.seg_key));
        self
    }

    pub fn object_hash(&self) -> Result<MestObjectHash, Box<bincode::ErrorKind>> {
        PersistedMestObject::Bucket(self.clone().canonicalized()).object_hash()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedMgtNode {
    pub version: u32,
    pub node_hash: MestObjectHash,
    pub is_leaf: bool,
    pub is_dirty: bool,
    pub bucket_key: Vec<i32>,
    pub bucket_hash: Option<MestObjectHash>,
    pub sub_node_hashes: Vec<Option<MestObjectHash>>,
    pub data_hashes: Vec<Vec<u8>>,
    pub cached_node_hashes: Vec<Option<MestObjectHash>>,
    pub cached_data_hashes: Vec<Vec<u8>>,
}

impl PersistedMgtNode {
    pub fn object_hash(&self) -> Result<MestObjectHash, Box<bincode::ErrorKind>> {
        PersistedMestObject::MgtNode(self.clone()).object_hash()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedSehDirectory {
    pub version: u32,
    pub gd: i32,
    pub rdx: i32,
    pub bucket_capacity: i32,
    pub bucket_seg_num: i32,
    pub buckets_number: i32,
    pub directory: BTreeMap<String, MestObjectHash>,
}

impl PersistedSehDirectory {
    pub fn object_hash(&self) -> Result<MestObjectHash, Box<bincode::ErrorKind>> {
        PersistedMestObject::SehDirectory(self.clone()).object_hash()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedMestManifest {
    pub version: u32,
    pub rdx: i32,
    pub bucket_capacity: i32,
    pub bucket_seg_num: i32,
    pub seh_directory_hash: Option<MestObjectHash>,
    pub mgt_root_object_hash: Option<MestObjectHash>,
    pub mgt_root_hash: MestObjectHash,
    pub current_root_hash: MestObjectHash,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PersistedMestObject {
    Bucket(PersistedBucket),
    MgtNode(PersistedMgtNode),
    SehDirectory(PersistedSehDirectory),
}

impl PersistedMestObject {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Bucket(_) => unsafe { std::str::from_utf8_unchecked(&[98, 117, 99, 107, 101, 116]) },
            Self::MgtNode(_) => unsafe { std::str::from_utf8_unchecked(&[109, 103, 116, 45, 110, 111, 100, 101]) },
            Self::SehDirectory(_) => unsafe { std::str::from_utf8_unchecked(&[115, 101, 104, 45, 100, 105, 114, 101, 99, 116, 111, 114, 121]) },
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, Box<bincode::ErrorKind>> {
        bincode::serialize(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Box<bincode::ErrorKind>> {
        bincode::deserialize(bytes)
    }

    pub fn object_hash(&self) -> Result<MestObjectHash, Box<bincode::ErrorKind>> {
        let bytes = self.to_bytes()?;
        Ok(Sha256::digest(&bytes).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn sample_bucket() -> PersistedBucket {
        PersistedBucket {
            version: MEST_STORAGE_FORMAT_VERSION,
            bucket_key: vec![2, 1],
            ld: 2,
            rdx: 16,
            capacity: 128,
            number: 2,
            seg_num: 2,
            segments: vec![
                PersistedBucketSegment {
                    seg_key: s(&[98]),
                    kv_pairs: vec![KVPair::new(s(&[98, 101, 116, 97]), s(&[102, 50]))],
                },
                PersistedBucketSegment {
                    seg_key: s(&[97]),
                    kv_pairs: vec![KVPair::new(s(&[97, 108, 112, 104, 97]), s(&[102, 49]))],
                },
            ],
            segment_roots: BTreeMap::from([
                (s(&[97]), [1u8; 32]),
                (s(&[98]), [2u8; 32]),
            ]),
            latch_timestamp: 123,
            delegation_list: BTreeMap::new(),
            pending_num: 0,
            to_del_map: BTreeMap::new(),
        }
    }

    #[test]
    fn test_bucket_object_hash_is_canonical() {
        let left = sample_bucket();
        let mut right = sample_bucket();
        right.segments.reverse();

        assert_eq!(left.object_hash().unwrap(), right.object_hash().unwrap());
    }

    #[test]
    fn test_persisted_object_roundtrip() {
        let object = PersistedMestObject::Bucket(sample_bucket().canonicalized());
        let bytes = object.to_bytes().unwrap();
        let restored = PersistedMestObject::from_bytes(&bytes).unwrap();

        assert_eq!(restored, object);
        assert_eq!(restored.kind_name(), unsafe { std::str::from_utf8_unchecked(&[98, 117, 99, 107, 101, 116]) });
    }
}
