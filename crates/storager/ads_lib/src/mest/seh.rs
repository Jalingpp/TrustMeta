use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use super::merkletree::MHTProof;
use super::bucket::Bucket;
use super::kvpair::KVPair;

// 纯内存版 SEH：去掉 db 和 cache 相关逻辑，仅维护内存中的 bucket 哈希表
#[derive(Clone)]
pub struct SEH {
    pub gd: i32,
    pub rdx: i32,
    pub bucket_capacity: i32,
    pub bucket_seg_num: i32,
    pub ht: Arc<RwLock<HashMap<String, Arc<RwLock<Bucket>>>>>,
    pub buckets_number: i32,
    pub latch: Arc<RwLock<()>>,
}

// 插入后用于回传的桶内证明结果
#[derive(Clone, Debug)]
pub struct SEHBucketInsertResult {
    pub bucket: Arc<RwLock<Bucket>>,            // 最终承载该 key 的桶
    pub bucket_key: Vec<i32>,                   // 桶的 bucket_key（叶->根）
    pub value: String,                          // 插入后该 key 对应的完整值（若有追加则包含追加）
    pub seg_root_hash: [u8; 32],                // 桶内对应段的 Merkle 根
    pub mht_proof: MHTProof,                    // 桶内段的 Merkle 路径证明
    pub leaf_segment_roots: Vec<[u8; 32]>,      // 该桶所有段的根（按段键排序），用于 MGT 叶子哈希
}

impl SEH {
    // 兼容 meht.rs 的构造签名（多余参数忽略）
    pub fn new(
        rdx: i32,
        bc: i32,
        bs: i32,
        _ws: i32,
        _stride: i32,
        _bfsize: i32,
        _bfhnum: i32,
    ) -> Arc<RwLock<SEH>> {
        Arc::new(RwLock::new(SEH {
            gd: 0,
            rdx,
            bucket_capacity: bc,
            bucket_seg_num: bs,
            ht: Arc::new(RwLock::new(HashMap::new())),
            buckets_number: 0,
            latch: Arc::new(RwLock::new(())),
        }))
    }

    // 仅内存：按 key 定位 bucket，不存在则返回 None
    pub fn get_bucket(&self, bucket_key: &str) -> Option<Arc<RwLock<Bucket>>> {
        self.ht.read().unwrap().get(bucket_key).cloned()
    }

    // 仅内存：根据用户 key 映射到一个桶；按 rdx 生成 gd 位前缀做查找
    pub fn get_bucket_by_key(&self, key: &str) -> Option<Arc<RwLock<Bucket>>> {
        if self.buckets_number == 0 {
            return None;
        }
        if self.gd <= 0 {
            return self.get_bucket("");
        }
        let prefix = make_prefix(key, self.rdx, self.gd as usize);
        let ht = self.ht.read().unwrap();
        if let Some(b) = ht.get(&prefix) {
            return Some(b.clone());
        }
        // 回退策略：逐步缩短前缀，直至根
        let mut parts: Vec<&str> = if prefix.is_empty() { vec![] } else { prefix.split('.').collect() };
        while !parts.is_empty() {
            parts.pop();
            let p = parts.join(".");
            if let Some(b) = ht.get(&p) {
                return Some(b.clone());
            }
        }
        ht.get("").cloned()
    }

    // 仅内存：简化插入逻辑；若无桶则创建默认根桶 ""
    pub fn insert(&mut self, key: String, value: String) -> SEHBucketInsertResult {
        self.insert_kvpair(KVPair { key, value })
    }

    // 仅内存：按 KVPair 插入，测试可直接传入 KVPair。
    // 当 key 已存在于目标桶中时，旧值将与新值用逗号连接（具体在 Bucket::insert 中实现）。
    pub fn insert_kvpair(&mut self, kv: KVPair) -> SEHBucketInsertResult {
        // Ensure at least a root bucket exists
        if self.buckets_number == 0 {
            let root = Arc::new(RwLock::new(Bucket::new(
                0,
                self.rdx,
                self.bucket_capacity,
                self.bucket_seg_num,
            )));
            self.ht.write().unwrap().insert(String::new(), root.clone());
            self.buckets_number = 1;
        }

        // Route to target bucket by key
        let mut current = match self.get_bucket_by_key(&kv.key) {
            Some(b) => b,
            None => self.ht.read().unwrap().get("").unwrap().clone(),
        };

        // Insert key/value
        current.write().unwrap().insert(kv.clone());

        // Check split condition: if target's size > capacity, split and update directory
        loop {
            // Compute size as total kvs across all segments
            let size = {
                let b = current.read().unwrap();
                let segs = b.segments.read().unwrap();
                segs.values().map(|v| v.len()).sum::<usize>() as i32
            };
            let capacity = current.read().unwrap().capacity;
            if size <= capacity { break; }

            // Perform split
            let parent_key_vec = current.read().unwrap().get_bucket_key();
            let parent_prefix = bucket_key_to_prefix(&parent_key_vec);
            let children = current.read().unwrap().split_bucket();

            // Update directory: remove parent mapping; insert children mappings
            {
                let mut ht = self.ht.write().unwrap();
                ht.remove(&parent_prefix);
                for ch in &children {
                    let k = bucket_key_to_prefix(&ch.read().unwrap().get_bucket_key());
                    ht.insert(k, ch.clone());
                }
            }

            // Expand global depth if needed
            let new_ld = children.first().unwrap().read().unwrap().get_ld();
            if self.gd < new_ld { self.gd = new_ld; }

            // Re-route target bucket for this key
            current = self.get_bucket_by_key(&kv.key).unwrap();
            // loop and check next split if still over capacity
        }

        // 在最终桶上生成桶内证明（值、段 Merkle 根与路径）
        let (value, seg_root_hash, mht_proof, leaf_segment_roots) = {
            let b_r = current.read().unwrap();
            let (v, r, p) = b_r.get_proof(&kv.key).expect("proof must exist after insert");
            let all_roots = b_r.get_segment_roots_sorted();
            (v, r, p, all_roots)
        };
        let bucket_key = current.read().unwrap().get_bucket_key();
        SEHBucketInsertResult { bucket: current, bucket_key, value, seg_root_hash, mht_proof, leaf_segment_roots }
    }

    // 仅内存：序列化 SEH 元数据（不包含桶内容）
    pub fn serialize_seh(&self) -> Result<String, serde_json::Error> {
        #[derive(serde::Serialize)]
        struct SeSEH {
            gd: i32,
            rdx: i32,
            bucket_capacity: i32,
            bucket_seg_num: i32,
            hash_table_keys: Vec<String>,
            buckets_number: i32,
        }

        let keys: Vec<String> = self
            .ht
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let se = SeSEH {
            gd: self.gd,
            rdx: self.rdx,
            bucket_capacity: self.bucket_capacity,
            bucket_seg_num: self.bucket_seg_num,
            hash_table_keys: keys,
            buckets_number: self.buckets_number,
        };
        serde_json::to_string(&se)
    }
}

// 将 key 转为 base-rdx 的 gd 位前缀，使用 '.' 连接成字符串
fn make_prefix(key: &str, rdx: i32, gd: usize) -> String {
    if gd == 0 || rdx <= 0 { return String::new(); }
    let digits = super::util::digits_prefix_mixed(key, rdx, gd);
    digits.into_iter().map(|d| d.to_string()).collect::<Vec<_>>().join(".")
}

fn bucket_key_to_prefix(bk: &[i32]) -> String {
    if bk.is_empty() { return String::new(); }
    // bucket_key 内部采用 叶->根 的顺序存储；目录键需要 根->叶 的顺序以与 make_prefix 对齐
    bk.iter().rev().map(|d| d.to_string()).collect::<Vec<_>>().join(".")
}

impl SEH {
    // 输入用户 key，返回 (value, seg_root_hash, mhtproof)
    pub fn get_proof(&self, key: &str) -> Option<(String, [u8; 32], MHTProof)> {
        let bucket = self.get_bucket_by_key(key)?;
        let b = bucket.read().ok()?;
        b.get_proof(key)
    }

    // 删除指定 key 下的单个 value；若删除后该 key 无值，则移除整个 KVPair。
    // 返回是否发生修改。
    pub fn delete_kvpair_value(&mut self, key: &str, value: &str) -> bool {
        // 空表或无桶
        if self.buckets_number == 0 {
            return false;
        }
        // 找到目标桶
        if let Some(bucket) = self.get_bucket_by_key(key) {
            let changed = bucket.write().unwrap().delete_value(key, value);
            // 不在此处更新目录结构（例如合并空桶），保持简洁；MGT 由上层触发同步。
            return changed;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};
    use crate::mest::util;

    fn dump_ht(seh: &Arc<RwLock<SEH>>, tag: &str) {
        let seh_r = seh.read().unwrap();
        let ht = seh_r.ht.read().unwrap();
        println!("[seh] HT dump {tag}: gd={}, entries={}", seh_r.gd, ht.len());
        for (dir_key, bkt) in ht.iter() {
            let bk = bkt.read().unwrap().get_bucket_key();
            println!("  {} -> bucket_key={:?}", dir_key, bk);
        }
    }

    #[test]
    fn test_seh_insert_split_and_lookup() {
        // rdx=4，容量=2，尽快触发分裂
        let seh = SEH::new(4, 2, 2, 0, 0, 0, 0);

        // 插入若干键，触发分裂并打印目录变化
        let test_keys = vec!["AA", "BB", "CC", "DD", "EE","FF","GG","HH","II","JJ"]; // 不同首字节

        for k in &test_keys {
            // 插入
            let kv = KVPair::new((*k).to_string(), format!("v_{k}"));
            let _res = seh.write().unwrap().insert_kvpair(kv);

            // 每次插入后输出当前目录
            dump_ht(&seh, &format!("after insert '{k}'"));

            // 基础校验：能查回同一个桶且命中
            let routed = seh.read().unwrap().get_bucket_by_key(k).unwrap();
            assert!(routed.read().unwrap().is_in_bucket(k));
            println!("[seh] lookup '{k}' ok");

            // 验证目录键和 bucket_key 的映射一致性
            for (dir_key, bkt) in seh.read().unwrap().ht.read().unwrap().iter() {
                let bk = bkt.read().unwrap().get_bucket_key();
                let expect = super::bucket_key_to_prefix(&bk);
                assert_eq!(&expect, dir_key);
            }
        }
    }

    // 说明 bucket_key 与用户 key 的关系：
    // - 以 rdx=4 为例，用户 key 的每一位（按字节）对 4 取模得到一位“路径数字”。
    // - 目录前缀使用根->叶顺序的前缀（第 0 位、第 1 位、...）。
    // - bucket_key 采用叶->根顺序存储，所以其反转等于目录前缀。
    // - 分裂后，bucket_key 在前面新增一位，该位等于用户 key 在相应深度上的路径数字。
    #[test]
    fn test_key_to_bucket_key_relation() {
        let seh = SEH::new(4, 1, 2, 0, 0, 0, 0); // 容量设为 1，快速分裂

        // 构造一组键，观察不同深度的路由位（采用带扰动的 digits_prefix_mixed）
        let keys = vec!["AA", "EE", "II", "MM"];

        // 计算前缀数字的本地辅助函数
        fn digits_prefix(key: &str, rdx: i32, len: usize) -> Vec<i32> {
            let bytes = key.as_bytes();
            let mut v = Vec::with_capacity(len);
            for i in 0..len {
                let d = (bytes[i % bytes.len()] as i32).rem_euclid(rdx);
                v.push(d);
            }
            v
        }

        for k in &keys {
            let kv = KVPair::new((*k).to_string(), format!("v_{k}"));
            let _ = seh.write().unwrap().insert_kvpair(kv);
            dump_ht(&seh, &format!("after insert '{k}'"));

            // 路由并校验：反转后的 bucket_key 等于该 key 在当前桶深度上的前缀数字
            let routed = seh.read().unwrap().get_bucket_by_key(k).unwrap();
            let bk = routed.read().unwrap().get_bucket_key(); // 叶->根
            let bk_len = bk.len();
            let dir_digits = util::digits_prefix_mixed(k, seh.read().unwrap().rdx, bk_len);
            let mut bk_rev = bk.clone();
            bk_rev.reverse(); // 根->叶

            println!("[rel] key='{}' -> digits(prefix,len={})={:?}; bucket_key(leaf->root)={:?}; reversed={:?}",
                     k, bk_len, dir_digits, bk, bk_rev);
            assert_eq!(bk_rev, dir_digits);
        }
    }

    fn bucket_total_len(b: &Bucket) -> usize {
        let segs = b.segments.read().unwrap();
        segs.values().map(|v| v.len()).sum::<usize>()
    }

    #[test]
    fn test_seh_deep_splits_distribution_rdx16() {
        // rdx=16，容量=2；插入 32 个不同 key，观察多层分裂与稳定性
        let seh = SEH::new(16, 2, 2, 0, 0, 0, 0);
        let keys: Vec<String> = (0..32).map(|i| format!("key_{:02}", i)).collect();

        for k in &keys {
            let kv = KVPair::new(k.clone(), format!("v_{k}"));
            let _ = seh.write().unwrap().insert_kvpair(kv);
            dump_ht(&seh, &format!("rdx16 after insert '{k}'"));

            // 路由命中校验
            let routed = seh.read().unwrap().get_bucket_by_key(k).unwrap();
            assert!(routed.read().unwrap().is_in_bucket(k));
        }

        // 目录键与 bucket_key 映射一致，且每个桶的大小不超过容量
        let gd = seh.read().unwrap().gd;
        println!("[seh] final gd={gd}");
        assert!(gd >= 2, "expected gd >= 2, got {gd}");

        let capacity = seh.read().unwrap().bucket_capacity as usize;
        for (dir_key, bkt) in seh.read().unwrap().ht.read().unwrap().iter() {
            let bk = bkt.read().unwrap().get_bucket_key();
            let expect = super::bucket_key_to_prefix(&bk);
            assert_eq!(&expect, dir_key);
            let size = bucket_total_len(&bkt.read().unwrap());
            println!("[seh] bucket dir_key='{}' size={} (cap={})", dir_key, size, capacity);
            assert!(size <= capacity, "bucket '{}' size {} exceeds capacity {}", dir_key, size, capacity);
        }
    }

    #[test]
    fn test_delete_via_seh_updates_bucket() {
        let seh = SEH::new(4, 8, 2, 0, 0, 0, 0);
        // insert k -> v1,v2
        let _ = seh.write().unwrap().insert_kvpair(KVPair::new("k".into(), "v1".into()));
        let _ = seh.write().unwrap().insert_kvpair(KVPair::new("k".into(), "v2".into()));
        assert_eq!(seh.read().unwrap().get_bucket_by_key("k").unwrap().read().unwrap().get_value("k").as_deref(), Some("v1,v2"));

        // delete v2 -> k -> v1
        assert!(seh.write().unwrap().delete_kvpair_value("k", "v2"));
        assert_eq!(seh.read().unwrap().get_bucket_by_key("k").unwrap().read().unwrap().get_value("k").as_deref(), Some("v1"));

        // delete v1 -> key removed
        assert!(seh.write().unwrap().delete_kvpair_value("k", "v1"));
        assert_eq!(seh.read().unwrap().get_bucket_by_key("k").unwrap().read().unwrap().get_value("k"), None);
    }

    #[test]
    fn test_insert_same_key_appends_value_via_seh() {
        let seh = SEH::new(4, 8, 2, 0, 0, 0, 0);
        let _ = seh.write().unwrap().insert_kvpair(KVPair::new("k".into(), "v1".into()));
        let b = seh.read().unwrap().get_bucket_by_key("k").unwrap();
        assert_eq!(b.read().unwrap().get_value("k").as_deref(), Some("v1"));

        let _ = seh.write().unwrap().insert_kvpair(KVPair::new("k".into(), "v2".into()));
        let b = seh.read().unwrap().get_bucket_by_key("k").unwrap();
        assert_eq!(b.read().unwrap().get_value("k").as_deref(), Some("v1,v2"));

        // Duplicate value should NOT be appended (dedup enabled)
        let _ = seh.write().unwrap().insert_kvpair(KVPair::new("k".into(), "v2".into()));
        let b = seh.read().unwrap().get_bucket_by_key("k").unwrap();
        assert_eq!(b.read().unwrap().get_value("k").as_deref(), Some("v1,v2"));
        
        // Add a new unique value
        let _ = seh.write().unwrap().insert_kvpair(KVPair::new("k".into(), "v3".into()));
        let b = seh.read().unwrap().get_bucket_by_key("k").unwrap();
        assert_eq!(b.read().unwrap().get_value("k").as_deref(), Some("v1,v2,v3"));
    }
}
