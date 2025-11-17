use std::sync::{Arc, RwLock};
use crate::seh::SEH;
use crate::mgt::MGT;
use crate::bucket::Bucket;
use crate::kvpair::KVPair;
use crate::mgt::{build_mgt_proof, KeyProof, BucketProofOut, verify_key_proof};

// In-memory MEHT: compose SEH and MGT only; drop DB/cache responsibilities.
pub struct MEHT {
    pub rdx: i32,
    pub bc: i32,
    pub bs: i32,
    pub seh: Arc<RwLock<SEH>>,
    pub mgt: Arc<RwLock<MGT>>,
    pub latch: RwLock<()>,
}

impl MEHT {
    // Keep a constructor compatible with SEH::new's extended signature; extra args are ignored here.
    pub fn new(rdx: i32, bc: i32, bs: i32, ws: i32, stride: i32, bfsize: i32, bfhnum: i32) -> Arc<RwLock<MEHT>> {
        let seh = SEH::new(rdx, bc, bs, ws, stride, bfsize, bfhnum);
        let mgt = Arc::new(RwLock::new(MGT::new(rdx)));
        Arc::new(RwLock::new(MEHT { rdx, bc, bs, seh, mgt, latch: RwLock::new(()) }))
    }

    // Simple 3-arg constructor sugar.
    pub fn new_simple(rdx: i32, bc: i32, bs: i32) -> Arc<RwLock<MEHT>> {
        Self::new(rdx, bc, bs, 0, 0, 0, 0)
    }

    pub fn get_seh(&self) -> Arc<RwLock<SEH>> { self.seh.clone() }
    pub fn get_mgt(&self) -> Arc<RwLock<MGT>> { self.mgt.clone() }

    // Insert a kv pair into SEH (may trigger splits), then synchronize MGT.
    // Return a combined proof consisting of:
    // - Bucket-level Merkle proof (final value, segment root, path)
    // - MGT-level path (root->leaf indices) and root hash
    pub fn insert(&self, kv_pair: KVPair) -> KeyProof {
        let res = self.seh.write().unwrap().insert_kvpair(kv_pair.clone());
        // sync MGT to ensure root hash/path reflect current buckets
        self.sync_mgt_from_seh();
        // Build MGT proof for the leaf bucket
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

    // Rebuild or update MGT from current SEH directory. This is a coarse-grained sync strategy
    // suitable for tests or pure in-memory usage.
    pub fn sync_mgt_from_seh(&self) {
        let buckets: Vec<Arc<RwLock<Bucket>>> = {
            let seh_r = self.seh.read().unwrap();
            let ht = seh_r.ht.read().unwrap();
            ht.values().cloned().collect()
        };
        if buckets.is_empty() { return; }
        let groups = vec![buckets];
        self.mgt.write().unwrap().mgt_update(groups);
    }

    // Expose proof API through SEH for convenience.
    pub fn get_proof(&self, key: &str) -> Option<(String, [u8; 32], crate::merkletree::MHTProof)> {
        self.seh.read().ok()?.get_proof(key)
    }

    // Query by key: return all values (comma-joined) and a proof
    // equivalent to the insert proof, which can be verified to the
    // current MGT root.
    pub fn query(&self, key: &str) -> Option<KeyProof> {
        // Locate bucket by key
        let bucket = {
            let seh_r = self.seh.read().ok()?;
            seh_r.get_bucket_by_key(key)?
        };
        // Bucket-level proof
        let (value, seg_root_hash, mht_proof) = {
            let b = bucket.read().ok()?;
            b.get_proof(key)?
        };
        let (bucket_key, leaf_segment_roots) = {
            let b = bucket.read().ok()?;
            (b.get_bucket_key(), b.get_segment_roots_sorted())
        };

        // Ensure MGT ready; if empty, sync once from SEH
        let need_sync = { self.mgt.read().ok()?.root.is_none() };
        if need_sync { self.sync_mgt_from_seh(); }

        let mgt_proof = {
            let mgt_r = self.mgt.read().ok()?;
            build_mgt_proof(&mgt_r, &bucket_key).ok()?
        };

        Some(KeyProof {
            key: key.to_string(),
            bucket_key,
            bucket_proof: BucketProofOut { value, seg_root_hash, proof: mht_proof, leaf_segment_roots },
            mgt_proof,
        })
    }

    // Convenience API: 删除指定 key 下的单个 value，并在成功时更新 MGT（全量同步）。
    pub fn delete(&self, key: &str, value: &str) -> bool {
        let changed = self.seh.write().unwrap().delete_kvpair_value(key, value);
        if changed { self.sync_mgt_from_seh(); }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helpers to print SEH and MGT structures for demonstration
    fn dump_seh(seh: &Arc<RwLock<SEH>>, tag: &str) {
        let seh_r = seh.read().unwrap();
        let ht = seh_r.ht.read().unwrap();
        println!("===== SEH [{}] gd={} entries={} =====", tag, seh_r.gd, ht.len());
        for (dir_key, bkt) in ht.iter() {
            let bk = bkt.read().unwrap().get_bucket_key();
            println!("  {} -> bucket_key(leaf->root)={:?}", dir_key, bk);
        }
    }

    fn dump_mgt(mgt: &Arc<RwLock<MGT>>, tag: &str) {
        println!("===== MGT [{}] =====", tag);
        let mgt_r = mgt.read().unwrap();
        if mgt_r.root.is_none() { println!("<empty root>"); return; }
        let root = mgt_r.root.as_ref().unwrap().clone();
        println!("root_hash={:02x?}", mgt_r.root.as_ref().unwrap().read().unwrap().node_hash);
        // DFS print
        fn dfs(node: &Arc<RwLock<crate::mgt::MGTNode>>, rdx: i32, path: &mut Vec<usize>) {
            let (is_leaf, bk, subs) = {
                let n = node.read().unwrap();
                (n.is_leaf, n.bucket_key.clone(), n.sub_nodes.clone())
            };
            let path_str = if path.is_empty() { "/".to_string() } else { format!("/{}", path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("/")) };
            if is_leaf {
                println!("path={}  [L] bucket_key={:?}", path_str, bk);
            } else {
                println!("path={}  [I]", path_str);
            }
            for i in 0..(rdx.max(0) as usize) {
                if let Some(Some(child)) = subs.get(i).cloned() {
                    path.push(i);
                    dfs(&child, rdx, path);
                    path.pop();
                }
            }
        }
        let mut p = Vec::new();
        dfs(&root, mgt_r.rdx, &mut p);
    }

    #[test]
    fn test_meht_insert_delete_query_bucket_details() {
        let meht = MEHT::new_simple(4, 2, 2); // small capacity -> easy splits
        let seh = meht.read().unwrap().get_seh();
        let mgt = meht.read().unwrap().get_mgt();

        // Generate more keys to observe bucket splits/SEH expansion/MGT growth
        let keys: Vec<String> = (0..16).map(|i| format!("k{:02}", i)).collect();

        // Inserts
        for k in &keys {
            let kv = KVPair::new(k.clone(), format!("v_{}", k));
            let _ = meht.read().unwrap().insert(kv);
            println!("===== after insert '{}' =====", k);
            dump_seh(&seh, "insert step");
            dump_buckets_detailed(&seh);
            dump_mgt(&mgt, "insert step");
        }

        // Deletes: remove a few keys entirely (by removing their single value)
        for &i in &[3usize, 7, 11] {
            let k = format!("k{:02}", i);
            let v = format!("v_{}", k);
            let ok = meht.read().unwrap().delete(&k, &v);
            println!("===== after delete ('{}','{}') => {} =====", k, v, ok);
            dump_seh(&seh, "delete step");
            dump_buckets_detailed(&seh);
            dump_mgt(&mgt, "delete step");
        }

        // Extra: insert more values for k05 to show multi-value accumulation
        for ev in ["w1", "w2", "w3", "w4"].iter() {
            let _ = meht.read().unwrap().insert(KVPair::new("k05".into(), (*ev).to_string()));
            println!("===== after extra insert for 'k05' value='{}' =====", ev);
            dump_seh(&seh, "extra insert k05");
            dump_buckets_detailed(&seh);
            dump_mgt(&mgt, "extra insert k05");
        }

        // Query k05 with proof and print verification
        let qk = "k05";
        if let Some(q) = meht.read().unwrap().query(qk) {
            println!("===== query '{}' all values => '{}' =====", qk, q.bucket_proof.value);
            println!("verify '{}' proof => {}", qk, verify_key_proof(&q));
        } else {
            println!("===== query '{}' => <not found> =====", qk);
        }
    }

    // Print all buckets' detailed content: bucket_key and each segment's KVPairs
    fn dump_buckets_detailed(seh: &Arc<RwLock<SEH>>) {
        let seh_r = seh.read().unwrap();
        let ht = seh_r.ht.read().unwrap();
        let mut dir_keys: Vec<String> = ht.keys().cloned().collect();
        dir_keys.sort();
        for dir_key in dir_keys {
            let b = ht.get(&dir_key).unwrap().read().unwrap();
            let bk = b.get_bucket_key();
            println!("-- bucket dir='{}' key(leaf->root)={:?}", dir_key, bk);
            let segs = b.segments.read().unwrap();
            let mut seg_keys: Vec<String> = segs.keys().cloned().collect();
            seg_keys.sort();
            if seg_keys.is_empty() {
                println!("   <empty segments>");
            }
            for sk in seg_keys {
                let kvs = segs.get(&sk).unwrap();
                let items: Vec<String> = kvs.iter().map(|p| format!("{}:{}", p.key, p.value)).collect();
                println!("   seg '{}' -> [{}]", sk, items.join(", "));
            }
        }
    }
}

// KeyProof types are defined in mgt.rs
