use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use crate::kvpair::KVPair;
use chrono;
use crate::merkletree::{MerkleTree, MHTProof};

#[derive(Debug, Clone)]
pub struct Bucket {
    pub bucket_key: Vec<i32>,
    pub ld: i32,
    pub rdx: i32,
    pub capacity: i32,
    pub number: i32,
    pub seg_num: i32,
    pub segments: Arc<RwLock<HashMap<String, Vec<KVPair>>>>,
    pub seg_idx_maps: Arc<RwLock<HashMap<String, HashMap<String, usize>>>>,
    pub merkle_trees: Arc<RwLock<HashMap<String, MerkleTree>>>,
    pub latch_timestamp: i64,
    pub delegation_list: Arc<RwLock<HashMap<String, HashMap<String, bool>>>>,
    pub pending_num: Arc<RwLock<i32>>,
    pub to_del_map: Arc<RwLock<HashMap<String, HashMap<String, i32>>>>,
}

impl Bucket {
    pub fn new(ld: i32, rdx: i32, capacity: i32, seg_num: i32) -> Self {
        Bucket {
            bucket_key: Vec::new(),
            ld,
            rdx,
            capacity,
            number: 0,
            seg_num,
            segments: Arc::new(RwLock::new(HashMap::new())),
            seg_idx_maps: Arc::new(RwLock::new(HashMap::new())),
            merkle_trees: Arc::new(RwLock::new(HashMap::new())),
            latch_timestamp: chrono::Utc::now().timestamp(),
            delegation_list: Arc::new(RwLock::new(HashMap::new())),
            pending_num: Arc::new(RwLock::new(0)),
            to_del_map: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // in-memory only: no-op persistence API removed

    pub fn get_segment_key(&self, key: &str) -> String {
        if self.seg_num < key.len() as i32 {
            key[..self.seg_num as usize].to_string()
        } else {
            "".to_string()
        }
    }

    pub fn get_segment(&self, seg_key: String) -> Arc<Vec<KVPair>> {
        // Already loaded
        if let Some(seg) = self.segments.read().unwrap().get(&seg_key) {
            return Arc::new(seg.clone());
        }
        // Create empty segment and empty merkle tree
        self.segments.write().unwrap().insert(seg_key.clone(), Vec::new());
        self.seg_idx_maps
            .write()
            .unwrap()
            .insert(seg_key.clone(), HashMap::new());
        self.merkle_trees
            .write()
            .unwrap()
            .insert(seg_key.clone(), MerkleTree::new_empty());
        Arc::new(Vec::new())
    }

    pub fn is_in_bucket(&self, key: &str) -> bool {
        let seg_key = self.get_segment_key(key);
        let segments = self.segments.read().unwrap();
        if let Some(seg) = segments.get(&seg_key) {
            let seg_idx_map = self.seg_idx_maps.read().unwrap();
            if let Some(idx_map) = seg_idx_map.get(&seg_key) {
                return idx_map.contains_key(key);
            }
        }
        false
    }

    pub fn get_value(&self, key: &str) -> Option<String> {
        let seg_key = self.get_segment_key(key);
        let segments = self.segments.read().unwrap();
        if let Some(seg) = segments.get(&seg_key) {
            let seg_idx_map = self.seg_idx_maps.read().unwrap();
            if let Some(idx_map) = seg_idx_map.get(&seg_key) {
                if let Some(&idx) = idx_map.get(key) {
                    return Some(seg[idx].value.clone());
                }
            }
        }
        None
    }

    // 生成指定 key 的 (value, seg_root_hash, MHTProof)
    pub fn get_proof(&self, key: &str) -> Option<(String, [u8; 32], MHTProof)> {
        let seg_key = self.get_segment_key(key);
        let (seg_vec, idx) = {
            let segments = self.segments.read().ok()?;
            let seg = segments.get(&seg_key)?.clone();
            let idx_maps = self.seg_idx_maps.read().ok()?;
            let imap = idx_maps.get(&seg_key)?;
            let idx = *imap.get(key)?;
            (seg, idx)
        };
        if seg_vec.is_empty() || idx >= seg_vec.len() { return None; }

        // 确保该段的 MerkleTree 存在并与段内容一致
        let seg_root; 
        let proof;
        {
            let mut mts = self.merkle_trees.write().ok()?;
            let mt = mts.entry(seg_key.clone()).or_insert_with(|| rebuild_merkle(&seg_vec));
            // 如果当前 tree 长度与段不一致，重建
            if mt.data_len() != seg_vec.len() {
                *mt = rebuild_merkle(&seg_vec);
            }
            seg_root = mt.get_root_hash()?;
            proof = mt.get_proof_for_index(idx)?;
        }
        let value = seg_vec[idx].value.clone();
        Some((value, seg_root, proof))
    }

    pub fn insert(&self, kv_pair: KVPair) {
        let seg_key = self.get_segment_key(&kv_pair.key);
        let mut segments = self.segments.write().unwrap();
        let mut seg_idx_maps = self.seg_idx_maps.write().unwrap();
        let mut merkle_trees = self.merkle_trees.write().unwrap();

        if let Some(seg) = segments.get_mut(&seg_key) {
            if let Some(idx_map) = seg_idx_maps.get_mut(&seg_key) {
                if let Some(&idx) = idx_map.get(&kv_pair.key) {
                    // Key exists: always append with comma (no dedup). Avoid trailing comma for empty new value.
                    let old = seg[idx].value.clone();
                    let newv = kv_pair.value.clone();
                    let merged = if old.is_empty() {
                        newv
                    } else if newv.is_empty() {
                        old.clone()
                    } else {
                        format!("{},{}", old, newv)
                    };
                    if merged != old {
                        seg[idx].value = merged;
                        // Incremental update the Merkle tree at index
                        if let Some(mt) = merkle_trees.get_mut(&seg_key) {
                            mt.update_root(idx, seg[idx].value.as_bytes().to_vec());
                        }
                    }
                } else {
                    seg.push(kv_pair.clone());
                    idx_map.insert(kv_pair.key.clone(), seg.len() - 1);
                    if let Some(mt) = merkle_trees.get_mut(&seg_key) {
                        mt.insert_data(kv_pair.value.as_bytes().to_vec());
                    }
                }
            }
        } else {
            segments.insert(seg_key.clone(), vec![kv_pair.clone()]);
            seg_idx_maps.insert(seg_key.clone(), HashMap::from([(kv_pair.key.clone(), 0)]));
            merkle_trees.insert(seg_key.clone(), MerkleTree::new(vec![kv_pair.value.as_bytes().to_vec()]));
        }
        // in-memory only: do not persist or count here
    }

    // 删除指定 key 下的单个 value。如果该 key 只剩一个 value 且等于待删值，则删除整个 KVPair。
    // 返回是否发生了修改。
    pub fn delete_value(&self, key: &str, to_del: &str) -> bool {
        if to_del.is_empty() { return false; }
        let seg_key = self.get_segment_key(key);
        let mut segments = self.segments.write().unwrap();
        let mut seg_idx_maps = self.seg_idx_maps.write().unwrap();
        let mut merkle_trees = self.merkle_trees.write().unwrap();

        let seg = match segments.get_mut(&seg_key) { Some(s) => s, None => return false };
        let idx_map = match seg_idx_maps.get_mut(&seg_key) { Some(m) => m, None => return false };
        let &mut idx = match idx_map.get_mut(key) { Some(i) => i, None => return false };

        // 安全读取当前值，拆分为列表
        let cur_val = seg[idx].value.clone();
        let mut parts: Vec<String> = if cur_val.is_empty() { Vec::new() } else { cur_val.split(',').map(|s| s.to_string()).collect() };
        // 定位第一个匹配项
        let pos = match parts.iter().position(|p| p == to_del) { Some(p) => p, None => return false };
        parts.remove(pos);

        if parts.is_empty() {
            // 删除整个 KVPair
            seg.remove(idx);
            // 更新 idx_map：移除该 key，并将后续元素索引减一
            idx_map.remove(key);
            for (_k, v) in idx_map.iter_mut() {
                if *v > idx { *v -= 1; }
            }
            // 重建该段 Merkle 树
            if let Some(mt) = merkle_trees.get_mut(&seg_key) {
                *mt = rebuild_merkle(seg);
            }
            true
        } else {
            // 仅更新该元素的值
            let new_val = parts.join(",");
            if new_val == cur_val { return false; }
            seg[idx].value = new_val;
            if let Some(mt) = merkle_trees.get_mut(&seg_key) {
                mt.update_root(idx, seg[idx].value.as_bytes().to_vec());
            }
            true
        }
    }

    pub fn split_bucket(&self) -> Vec<Arc<RwLock<Bucket>>> {
        // Gather all existing kv pairs
        let mut all_kvs: Vec<KVPair> = Vec::new();
        {
            let segments = self.segments.read().unwrap();
            for (_seg_key, kvs) in segments.iter() {
                all_kvs.extend_from_slice(kvs);
            }
        }

        // Create rdx children with ld + 1 and prefixed bucket_key
        let parent_key = self.get_bucket_key();
        let new_ld = self.get_ld() + 1;
        let mut children: Vec<Arc<RwLock<Bucket>>> = Vec::with_capacity(self.rdx as usize);
        for d in 0..self.rdx {
            let mut child = Bucket::new(new_ld, self.rdx, self.capacity, self.seg_num);
            // 新的 bucket_key = 子向量([d]) + 旧的 bucket_key
            // 这里的“子向量”是索引号在 base-rdx 下的表示；
            // 在当前实现中，一个索引号 d (0..rdx-1) 本身即为 base-rdx 的一位，
            // 因此直接用 [d] 作为前缀。如果后续需要更长的前缀（例如 stride>1），
            // 可在此处将 d 拆解为多位并依次 extend。
            let mut bkey = Vec::with_capacity(parent_key.len() + 1);
            bkey.push(d);
            bkey.extend(parent_key.iter().copied());
            child.set_bucket_key(bkey);
            children.push(Arc::new(RwLock::new(child)));
        }

        // Redistribute kv pairs
        for kvp in all_kvs.into_iter() {
            let idx = digit_at(&kvp.key, self.rdx, (new_ld - 1) as usize) as usize;
            children[idx].write().unwrap().insert(kvp);
        }

        // Delegation and deletion maps are left empty for children (can be refined later)
        children
    }

    pub fn set_ld(&mut self, ld: i32) {
        self.ld = ld;
    }

    pub fn get_ld(&self) -> i32 {
        self.ld
    }

    pub fn set_bucket_key(&mut self, bucket_key: Vec<i32>) {
        self.bucket_key = bucket_key;
    }

    pub fn get_bucket_key(&self) -> Vec<i32> {
        self.bucket_key.clone()
    }

    pub fn get_segments(&self) -> Arc<RwLock<HashMap<String, Vec<KVPair>>>> {
        self.segments.clone()
    }

    // 返回按段键排序后的所有段 Merkle 树根，用于 MGT 叶子哈希的计算
    pub fn get_segment_roots_sorted(&self) -> Vec<[u8; 32]> {
        let mts = self.merkle_trees.read().unwrap();
        let mut keys: Vec<String> = mts.keys().cloned().collect();
        keys.sort();
        let mut roots: Vec<[u8; 32]> = Vec::new();
        for k in keys {
            if let Some(r) = mts.get(&k).and_then(|mt| mt.get_root_hash()) {
                roots.push(r);
            }
        }
        roots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_bucket_key_and_distribution() {
        // 准备：rdx=4，父桶 ld=0，bucket_key=[1,2] (叶->根)
        let rdx = 4;
        let parent_key = vec![1, 2];
        let mut parent = Bucket::new(0, rdx, 100, 2);
        parent.set_bucket_key(parent_key.clone());

        // 插入一些键，便于观察分裂后的重分配
        let keys = vec![
            ("a", "va"), // 'a' = 97, 97 % 4 = 1
            ("b", "vb"), // 98 % 4 = 2
            ("c", "vc"), // 99 % 4 = 3
            ("d", "vd"), // 100 % 4 = 0
            ("e", "ve"), // 101 % 4 = 1
        ];
        for (k, v) in &keys {
            parent.insert(KVPair::new((*k).to_string(), (*v).to_string()));
        }

        println!("[bucket] parent bucket_key (leaf->root) = {:?}", parent.get_bucket_key());

        // 执行分裂
        let children = parent.split_bucket();
        assert_eq!(children.len(), rdx as usize);
        let new_ld = parent.get_ld() + 1; // split_bucket 内部 new_ld = ld + 1

        // 1) 验证新的 bucket_key 规则：新 = [d] + 旧
        for d in 0..rdx {
            let c = children[d as usize].read().unwrap();
            let mut expected = vec![d];
            expected.extend(parent_key.iter().copied());
            println!("[bucket] child[{d}] bucket_key = {:?}", c.get_bucket_key());
            assert_eq!(c.get_bucket_key(), expected);
            assert_eq!(c.get_ld(), new_ld);
        }

        // 2) 验证重分配：按 digit_at(key, rdx, pos = new_ld-1) 进入对应 child
        for (k, _v) in &keys {
            let idx = super::digit_at(k, rdx, (new_ld - 1) as usize) as usize;
            let child = children[idx].read().unwrap();
            assert!(child.is_in_bucket(k), "key '{k}' should be in child[{idx}]");
            println!("[bucket] key '{k}' -> child[{idx}] OK");
        }
    }

    #[test]
    fn test_insert_same_key_appends_value() {
        let b = Bucket::new(0, 4, 100, 2);
        // First insert
        b.insert(KVPair::new("k".to_string(), "v1".to_string()));
        assert_eq!(b.get_value("k").as_deref(), Some("v1"));

        // Second insert with same key
        b.insert(KVPair::new("k".to_string(), "v2".to_string()));
        assert_eq!(b.get_value("k").as_deref(), Some("v1,v2"));

        // Third insert with duplicate value should duplicate (no dedup expected)
        b.insert(KVPair::new("k".to_string(), "v2".to_string()));
        assert_eq!(b.get_value("k").as_deref(), Some("v1,v2,v2"));

        // Insert empty new value: should keep old
        b.insert(KVPair::new("k".to_string(), "".to_string()));
        assert_eq!(b.get_value("k").as_deref(), Some("v1,v2,v2"));
    }

    #[test]
    fn test_delete_value_and_whole_kvpair() {
        let b = Bucket::new(0, 4, 100, 2);
        // Insert k -> v1,v2,v2
        b.insert(KVPair::new("k".to_string(), "v1".to_string()));
        b.insert(KVPair::new("k".to_string(), "v2".to_string()));
        b.insert(KVPair::new("k".to_string(), "v2".to_string()));
        assert_eq!(b.get_value("k").as_deref(), Some("v1,v2,v2"));

        // Delete one v2 (only one occurrence removed)
        assert!(b.delete_value("k", "v2"));
        assert_eq!(b.get_value("k").as_deref(), Some("v1,v2"));

        // Delete v1, leaving only v2
        assert!(b.delete_value("k", "v1"));
        assert_eq!(b.get_value("k").as_deref(), Some("v2"));

        // Delete the last v2 -> whole kv removed
        assert!(b.delete_value("k", "v2"));
        assert_eq!(b.get_value("k"), None);
    }
}

// 简单的分配函数：根据 key 的字节和对 rdx 取模，模拟原先 util::string_to_bucket_key_idx_with_rdx
fn string_to_bucket_key_idx_with_rdx(key: &str, _ld: i32, rdx: i32) -> i32 {
    if rdx <= 0 { return 0; }
    (key.as_bytes().iter().fold(0u64, |acc, b| acc.wrapping_add(*b as u64)) % (rdx as u64)) as i32
}

fn rebuild_merkle(seg: &Vec<KVPair>) -> MerkleTree {
    let data = seg.iter().map(|kv| kv.value.as_bytes().to_vec()).collect::<Vec<_>>();
    if data.is_empty() {
        MerkleTree::new_empty()
    } else {
        MerkleTree::new(data)
    }
}

// 取 key 在第 pos 层的路由位（带深度扰动），与 SEH 路由保持一致
fn digit_at(key: &str, rdx: i32, pos: usize) -> i32 {
    crate::util::digit_at_mixed(key, rdx, pos)
}
