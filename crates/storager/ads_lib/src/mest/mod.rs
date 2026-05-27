pub mod bucket;
pub mod kvpair;
pub mod merkletree;
pub mod mgt;
pub mod proof;
pub mod seh;
pub mod unified_adapter;
pub mod util;

pub use bucket::Bucket;
pub use kvpair::KVPair;
pub use merkletree::MerkleTree;
pub use mgt::{
    build_mgt_proof, verify_key_proof, verify_mgt_proof, BucketProofOut, KeyProof, MGTProof,
    MGTProofStep,
};
pub use mgt::{MGTNode, MGT};
pub use proof::{verify_mest_proof, BucketProof, MestProof, MgtProof};
pub use unified_adapter::MestAdapter;
pub use util::compute_stride_by_base;

// MEHT 实现
use seh::SEH;
use std::sync::{Arc, RwLock};

pub struct MEHT {
    pub rdx: i32,
    pub bc: i32,
    pub bs: i32,
    pub seh: Arc<RwLock<SEH>>,
    pub mgt: Arc<RwLock<MGT>>,
    pub latch: RwLock<()>,
}

impl MEHT {
    pub fn new(
        rdx: i32,
        bc: i32,
        bs: i32,
        ws: i32,
        stride: i32,
        bfsize: i32,
        bfhnum: i32,
    ) -> Arc<RwLock<MEHT>> {
        let seh = SEH::new(rdx, bc, bs, ws, stride, bfsize, bfhnum);
        let mgt = Arc::new(RwLock::new(MGT::new(rdx)));
        Arc::new(RwLock::new(MEHT {
            rdx,
            bc,
            bs,
            seh,
            mgt,
            latch: RwLock::new(()),
        }))
    }

    pub fn new_simple(rdx: i32, bc: i32, bs: i32) -> Arc<RwLock<MEHT>> {
        Self::new(rdx, bc, bs, 0, 0, 0, 0)
    }

    pub fn get_seh(&self) -> Arc<RwLock<SEH>> {
        self.seh.clone()
    }
    pub fn get_mgt(&self) -> Arc<RwLock<MGT>> {
        self.mgt.clone()
    }

    pub fn insert(&self, kv_pair: KVPair) -> KeyProof {
        let res = self.seh.write().unwrap().insert_kvpair(kv_pair.clone());
        self.sync_mgt_from_seh();
        let mgt_proof = {
            let mgt_r = self.mgt.read().unwrap();
            build_mgt_proof(&mgt_r, &res.bucket_key).expect("build mgt proof")
        };
        KeyProof {
            key: kv_pair.key,
            bucket_key: res.bucket_key,
            bucket_proof: BucketProofOut {
                value: res.value,
                seg_root_hash: res.seg_root_hash,
                proof: res.mht_proof,
                leaf_segment_roots: res.leaf_segment_roots,
            },
            mgt_proof,
        }
    }

    pub fn sync_mgt_from_seh(&self) {
        let buckets: Vec<Arc<RwLock<Bucket>>> = {
            let seh_r = self.seh.read().unwrap();
            let ht = seh_r.ht.read().unwrap();
            let mut seen = std::collections::HashSet::new();
            let mut buckets = Vec::new();
            for bucket in ht.values() {
                let bucket_ptr = Arc::as_ptr(bucket) as usize;
                if seen.insert(bucket_ptr) {
                    buckets.push(bucket.clone());
                }
            }
            buckets
        };
        if buckets.is_empty() {
            return;
        }
        let groups = vec![buckets];
        let mut mgt_w = self.mgt.write().unwrap();
        *mgt_w = MGT::new(self.rdx);
        mgt_w.mgt_update(groups);
    }

    pub fn query(&self, key: &str) -> Option<KeyProof> {
        let bucket = {
            let seh_r = self.seh.read().ok()?;
            seh_r.get_bucket_by_key(key)?
        };
        let (value, seg_root_hash, mht_proof) = {
            let b = bucket.read().ok()?;
            b.get_proof(key)?
        };
        let (bucket_key, leaf_segment_roots) = {
            let b = bucket.read().ok()?;
            (b.get_bucket_key(), b.get_segment_roots_sorted())
        };

        let need_sync = { self.mgt.read().ok()?.root.is_none() };
        if need_sync {
            self.sync_mgt_from_seh();
        }

        let mgt_proof = {
            let mgt_r = self.mgt.read().ok()?;
            build_mgt_proof(&mgt_r, &bucket_key).ok()?
        };

        Some(KeyProof {
            key: key.to_string(),
            bucket_key: bucket_key.to_vec(),
            bucket_proof: BucketProofOut {
                value,
                seg_root_hash,
                proof: mht_proof,
                leaf_segment_roots,
            },
            mgt_proof,
        })
    }

    pub fn delete(&self, key: &str, value: &str) -> bool {
        let changed = self.seh.write().unwrap().delete_kvpair_value(key, value);
        if changed {
            self.sync_mgt_from_seh();
        }
        changed
    }
}
