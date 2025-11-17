use std::collections::HashMap;
use std::sync::{Arc, RwLock, Mutex};
use sha2::{Sha256, Digest};
use super::merkletree::{MHTProof, verify_proof as verify_bucket_merkle};

use super::bucket::Bucket;

// Minimal, type-consistent definition of the MGT structures.
// Many advanced methods were intentionally omitted to make this
// module compile cleanly with the rest of the crate as-is.

pub struct MGTNode {
    pub node_hash: [u8; 32],
    pub parent: Option<Arc<RwLock<MGTNode>>>,
    pub sub_nodes: Vec<Option<Arc<RwLock<MGTNode>>>>,
    pub data_hashes: Vec<Vec<u8>>,
    pub cached_nodes: Vec<Option<Arc<RwLock<MGTNode>>>>,
    pub cached_data_hashes: Vec<Vec<u8>>,
    pub is_leaf: bool,
    pub is_dirty: bool,
    pub bucket: Option<Arc<RwLock<Bucket>>>,
    pub bucket_key: Vec<i32>,
    pub latch: RwLock<()>,
    pub sub_nodes_latch: Vec<RwLock<()>>,
    pub cached_nodes_latch: Vec<RwLock<()>>,
}

pub struct MGT {
    pub rdx: i32,
    pub root: Option<Arc<RwLock<MGTNode>>>,
    pub mgt_root_hash: [u8; 32],
    pub cached_ln_map: RwLock<HashMap<String, bool>>,
    pub cached_in_map: RwLock<HashMap<String, bool>>,
    pub hotness_list: RwLock<HashMap<String, i32>>,
    pub access_length: RwLock<i32>,
    pub latch: RwLock<()>,
    pub update_latch: Mutex<()>,
}

impl MGT {
    pub fn new(rdx: i32) -> Self {
        Self {
            rdx,
            root: None,
            mgt_root_hash: [0; 32],
            cached_ln_map: RwLock::new(HashMap::new()),
            cached_in_map: RwLock::new(HashMap::new()),
            hotness_list: RwLock::new(HashMap::new()),
            access_length: RwLock::new(0),
            latch: RwLock::new(()),
            update_latch: Mutex::new(()),
        }
    }

    // 纯内存：根据 bucket_key 查找叶子节点，并返回从叶子到根的路径
    // 只有当 bucket_key 在 cached_ln_map 中时才会访问 cached_nodes；否则仅遍历 sub_nodes
    // 若未找到，返回 Err
    pub fn get_leaf_node_and_path(
        &self,
        bucket_key: Vec<i32>,
    ) -> Result<Vec<Arc<RwLock<MGTNode>>>, String> {
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| "root not set".to_string())?
            .clone();

        // 空 key 特判：如果根就是叶子，直接返回；否则按 DFS 搜索
        if bucket_key.is_empty() {
            let is_leaf = root.read().map_err(|_| "poisoned node lock")?.is_leaf;
            if is_leaf {
                return Ok(vec![root]);
            } else {
                return Err("leaf not found".to_string());
            }
        }

        // 是否允许访问 cached_nodes
        let allow_cached = {
            let k = key_to_string(&bucket_key);
            self.cached_ln_map.read().unwrap().get(&k).cloned().unwrap_or(false)
        };

        let mut path_root_to_leaf = Vec::new();
        let start_pos = bucket_key.len() as isize - 1;
        if dfs_find_leaf_mode(&root, &bucket_key, allow_cached, &mut path_root_to_leaf, start_pos) {
            path_root_to_leaf.reverse();
            Ok(path_root_to_leaf)
        } else {
            Err("leaf not found".to_string())
        }
    }

    // 纯内存：根据 bucket_key 查找内部节点，并返回从目标节点到根的路径
    // 只有当 bucket_key 在 cached_in_map 中时才会访问 cached_nodes；否则仅遍历 sub_nodes
    pub fn get_internal_node_and_path(
        &self,
        bucket_key: Vec<i32>,
    ) -> Result<Vec<Arc<RwLock<MGTNode>>>, String> {
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| "root not set".to_string())?
            .clone();

        // 空 key 特判：根节点即为内部节点
        if bucket_key.is_empty() {
            return Ok(vec![root]);
        }

        let allow_cached = {
            let k = key_to_string(&bucket_key);
            self.cached_in_map.read().unwrap().get(&k).cloned().unwrap_or(false)
        };

        let mut path_root_to_target = Vec::new();
        let start_pos = bucket_key.len() as isize - 1;
        if dfs_find_internal_mode(&root, &bucket_key, allow_cached, &mut path_root_to_target, start_pos) {
            path_root_to_target.reverse();
            Ok(path_root_to_target)
        } else {
            Err("internal node not found".to_string())
        }
    }

    // 基于一批“新生成的桶”更新内存中的 MGT 结构。
    // newbuckets 是二维数组：外层表示连续分裂的批次（同批次为同一层产生的兄弟桶）。
    // 实际更新按每个 bucket 的 bucket_key（叶->根）路径逐个插入/覆盖：
    // - 自根向下，按 bucket_key 的倒序（根->叶）逐层定位唯一 child index；
    // - 若沿途节点不存在则创建内部节点；
    // - 若沿途遇到叶节点但仍需向下，则升级为内部节点（清除 bucket 引用）；
    // - 末端节点设为叶节点，挂接该 bucket，并记录其 bucket_key。
    pub fn mgt_update(&mut self, newbuckets: Vec<Vec<Arc<RwLock<Bucket>>>>) {
        if self.root.is_none() {
            self.root = Some(new_internal_node(None, self.rdx));
        }
        let root = self.root.as_ref().unwrap().clone();

        for group in newbuckets.into_iter() {
            for b in group.into_iter() {
                let bkey = b.read().unwrap().get_bucket_key(); // 叶->根

                // 空 key：将根设为叶
                if bkey.is_empty() {
                    {
                        let mut r = root.write().unwrap();
                        r.is_leaf = true;
                        r.bucket = Some(b.clone());
                        r.bucket_key = bkey.clone();
                        r.is_dirty = true;
                    }
                    // 叶更新，向上刷新 hash
                    propagate_hash_up(&root);
                    continue;
                }

                let mut cur = root.clone();
                let key_len = bkey.len();
                let mut last_child: Option<Arc<RwLock<MGTNode>>> = None;
                for (step, &digit) in bkey.iter().rev().enumerate() {
                    if digit < 0 || digit >= self.rdx { break; }
                    let idx = digit as usize;
                    let is_last = step + 1 == key_len;

                    // 确保当前节点可继续扩展；必要时将叶升级为内部
                    let child = {
                        let mut c = cur.write().unwrap();
                        ensure_node_slots(&mut *c, self.rdx);
                        if c.is_leaf && !is_last {
                            c.is_leaf = false;
                            c.bucket = None;
                            c.is_dirty = true;
                        }
                        match c.sub_nodes[idx].clone() {
                            Some(ch) => ch,
                            None => {
                                let nn = if is_last {
                                    new_leaf_node(Some(cur.clone()), self.rdx, bkey.clone(), Some(b.clone()))
                                } else {
                                    new_internal_node(Some(cur.clone()), self.rdx)
                                };
                                c.sub_nodes[idx] = Some(nn.clone());
                                nn
                            }
                        }
                    };

                    if is_last {
                        let mut w = child.write().unwrap();
                        w.is_leaf = true;
                        w.bucket = Some(b.clone());
                        w.bucket_key = bkey.clone();
                        w.is_dirty = true;
                        if w.parent.is_none() { w.parent = Some(cur.clone()); }
                    } else {
                        let mut w = child.write().unwrap();
                        if w.is_leaf {
                            w.is_leaf = false;
                            w.bucket = None;
                            w.is_dirty = true;
                        }
                        if w.parent.is_none() { w.parent = Some(cur.clone()); }
                    }

                    last_child = Some(child.clone());
                    cur = child;
                }
                if let Some(ch) = last_child { propagate_hash_up(&ch); }
            }
        }
    }
}

impl Default for MGT {
    fn default() -> Self { Self::new(16) }
}

// ---- MGT Merkle-like proof (leaf->root siblings) ----
#[derive(Clone, Debug)]
pub struct MGTProofStep {
    pub idx: usize,                                 // child index in sub_nodes
    pub sub_siblings: Vec<(usize, [u8; 32])>,       // (index, hash) for other present sub_nodes
    pub cached_siblings: Vec<(usize, [u8; 32])>,    // (index, hash) for present cached_nodes
}

#[derive(Clone, Debug)]
pub struct MGTProof {
    pub route: Vec<usize>,          // root->leaf child indices (redundant; helpful for debugging)
    pub steps: Vec<MGTProofStep>,   // leaf->root proof steps
    pub root_hash: [u8; 32],        // MGT root node hash
}

// Build an MGT merkle-like proof from leaf (bucket_key) up to the root.
// The proof at each level contains all sibling hashes needed to reconstruct the parent hash.
pub fn build_mgt_proof(mgt: &MGT, bucket_key: &[i32]) -> Result<MGTProof, String> {
    let path_lr = mgt.get_leaf_node_and_path(bucket_key.to_vec())
        .map_err(|e| format!("leaf not found: {}", e))?; // leaf -> root

    // root hash snapshot
    let root_hash = mgt
        .root
        .as_ref()
        .ok_or_else(|| "root not set".to_string())?
        .read().map_err(|_| "poisoned root lock")?
        .node_hash;

    // route equals reversed bucket_key
    let route: Vec<usize> = bucket_key.iter().rev().map(|d| *d as usize).collect();

    let mut steps: Vec<MGTProofStep> = Vec::new();
    for w in path_lr.windows(2) { // [child, parent]
        let child = &w[0];
        let parent = &w[1];
        let child_ptr = Arc::as_ptr(child);

        let (subs, cacheds) = {
            let p = parent.read().map_err(|_| "poisoned parent lock")?;
            (p.sub_nodes.clone(), p.cached_nodes.clone())
        };

        // locate child index among sub_nodes; prefer sub_nodes (route uses sub_nodes)
        let mut idx_opt: Option<usize> = None;
        for (i, op) in subs.iter().enumerate() {
            if let Some(a) = op {
                if Arc::as_ptr(a) == child_ptr { idx_opt = Some(i); break; }
            }
        }
        let idx = idx_opt.ok_or_else(|| "child not found in parent's sub_nodes".to_string())?;

        // gather siblings
        let mut sub_siblings: Vec<(usize, [u8; 32])> = Vec::new();
        for (i, op) in subs.into_iter().enumerate() {
            if i == idx { continue; }
            if let Some(a) = op {
                let h = a.read().map_err(|_| "poisoned child lock")?.node_hash;
                sub_siblings.push((i, h));
            }
        }
        let mut cached_siblings: Vec<(usize, [u8; 32])> = Vec::new();
        for (i, op) in cacheds.into_iter().enumerate() {
            if let Some(a) = op {
                let h = a.read().map_err(|_| "poisoned cached lock")?.node_hash;
                cached_siblings.push((i, h));
            }
        }

        steps.push(MGTProofStep { idx, sub_siblings, cached_siblings });
    }

    Ok(MGTProof { route, steps, root_hash })
}

// Verify the proof from a leaf hash upward to the root.
// `leaf_roots` are the per-segment bucket Merkle roots, sorted by segment key,
// used to reconstruct the leaf node hash as sha256(concat(leaf_roots)).
pub fn verify_mgt_proof(leaf_roots: &[[u8; 32]], proof: &MGTProof) -> bool {
    // leaf node hash = sha256(concat(leaf_roots))
    let mut h = Sha256::new();
    for r in leaf_roots { h.update(r); }
    let mut cur: [u8; 32] = h.finalize().into();

    for step in &proof.steps {
        // build ordered data_hashes for sub_nodes
        let mut pairs: Vec<(usize, [u8; 32])> = Vec::with_capacity(step.sub_siblings.len() + 1);
        pairs.push((step.idx, cur));
        pairs.extend_from_slice(&step.sub_siblings);
        pairs.sort_by_key(|(i, _)| *i);

        let mut hasher = Sha256::new();
        for (_, hh) in &pairs { hasher.update(hh); }

        // then cached_data_hashes (ordered by index asc)
        if !step.cached_siblings.is_empty() {
            let mut cached = step.cached_siblings.clone();
            cached.sort_by_key(|(i, _)| *i);
            for (_, hh) in &cached { hasher.update(hh); }
        }
        cur = hasher.finalize().into();
    }

    cur == proof.root_hash
}

// ---- Combined KeyProof (bucket-level + MGT-level) ----
#[derive(Clone, Debug)]
pub struct BucketProofOut {
    pub value: String,
    pub seg_root_hash: [u8; 32],
    pub proof: MHTProof,
    pub leaf_segment_roots: Vec<[u8; 32]>,
}

#[derive(Clone, Debug)]
pub struct KeyProof {
    pub key: String,            // inserted key
    pub bucket_key: Vec<i32>,   // bucket key (leaf->root)
    pub bucket_proof: BucketProofOut,
    pub mgt_proof: MGTProof,
}

// One-shot verification for KeyProof.
// 1) Verify the bucket-level Merkle proof of (key's value -> segment root)
// 2) Ensure the segment root is included in the bucket's leaf roots set
// 3) Verify the MGT proof of (leaf roots -> MGT root)
pub fn verify_key_proof(p: &KeyProof) -> bool {
    let ok_bucket = verify_bucket_merkle(
        p.bucket_proof.value.as_bytes(),
        p.bucket_proof.seg_root_hash,
        &p.bucket_proof.proof,
    );
    if !ok_bucket { return false; }
    if !p.bucket_proof.leaf_segment_roots.iter().any(|r| *r == p.bucket_proof.seg_root_hash) {
        return false;
    }
    verify_mgt_proof(&p.bucket_proof.leaf_segment_roots, &p.mgt_proof)
}

// DFS 搜索：找到叶子节点（bucket_key 完全匹配）
// 注意：不再遍历所有 sub_nodes，而是根据 bucket_key 的“路径位”选择唯一的下标。
// 具体规则：将 bucket_key 视为 base-rdx 的向量，该向量表示从叶子到根的路径；
// 自根向下查找时，第一步使用向量的最后一位，第二步使用倒数第二位，依此类推。
fn dfs_find_leaf_mode(
    node: &Arc<RwLock<MGTNode>>,
    target: &[i32],
    allow_cached: bool,
    path: &mut Vec<Arc<RwLock<MGTNode>>>,
    // 从叶子端开始的当前位置（初始为 target.len() as isize - 1，并逐层递减）
    pos_from_leaf: isize,
) -> bool {
    path.push(node.clone());

    // 避免持有锁跨递归：克隆需要的字段
    let (is_leaf, bk, subs, cacheds) = {
        let n = node.read().unwrap();
        (
            n.is_leaf,
            n.bucket_key.clone(),
            n.sub_nodes.clone(),
            n.cached_nodes.clone(),
        )
    };

    if is_leaf && bk == target {
        return true;
    }

    // 若没有更多路径位可用，则无法继续向下匹配
    if pos_from_leaf < 0 {
        path.pop();
        return false;
    }

    // 只根据路径位选择唯一 sub_node 下标
    let idx = target[pos_from_leaf as usize];
    if idx >= 0 {
        let idx_usize = idx as usize;
        if let Some(Some(child)) = subs.get(idx_usize).cloned() {
            if dfs_find_leaf_mode(&child, target, allow_cached, path, pos_from_leaf - 1) {
                return true;
            }
        }
        // 如果允许，尝试在 cached_nodes 上使用相同的下标
        if allow_cached {
            if let Some(Some(child)) = cacheds.get(idx_usize).cloned() {
                if dfs_find_leaf_mode(&child, target, allow_cached, path, pos_from_leaf - 1) {
                    return true;
                }
            }
        }
    }

    path.pop();
    false
}

// DFS 搜索：找到内部节点（非叶且 bucket_key 完全匹配）
// 遍历策略与叶子查找一致：按路径位索引 sub_nodes，而非遍历。
fn dfs_find_internal_mode(
    node: &Arc<RwLock<MGTNode>>,
    target: &[i32],
    allow_cached: bool,
    path: &mut Vec<Arc<RwLock<MGTNode>>>,
    pos_from_leaf: isize,
) -> bool {
    path.push(node.clone());

    let (is_leaf, bk, subs, cacheds) = {
        let n = node.read().unwrap();
        (
            n.is_leaf,
            n.bucket_key.clone(),
            n.sub_nodes.clone(),
            n.cached_nodes.clone(),
        )
    };

    if !is_leaf && bk == target {
        return true;
    }

    if pos_from_leaf < 0 {
        path.pop();
        return false;
    }

    let idx = target[pos_from_leaf as usize];
    if idx >= 0 {
        let idx_usize = idx as usize;
        if let Some(Some(child)) = subs.get(idx_usize).cloned() {
            if dfs_find_internal_mode(&child, target, allow_cached, path, pos_from_leaf - 1) {
                return true;
            }
        }
        if allow_cached {
            if let Some(Some(child)) = cacheds.get(idx_usize).cloned() {
                if dfs_find_internal_mode(&child, target, allow_cached, path, pos_from_leaf - 1) {
                    return true;
                }
            }
        }
    }

    path.pop();
    false
}

fn key_to_string(key: &[i32]) -> String {
    if key.is_empty() { return String::new(); }
    key.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(".")
}

// ---- helpers for node construction and sizing ----

fn new_internal_node(parent: Option<Arc<RwLock<MGTNode>>>, rdx: i32) -> Arc<RwLock<MGTNode>> {
    Arc::new(RwLock::new(MGTNode {
        node_hash: [0; 32],
        parent,
        sub_nodes: vec![None; rdx.max(0) as usize],
        data_hashes: Vec::new(),
        cached_nodes: vec![None; rdx.max(0) as usize],
        cached_data_hashes: Vec::new(),
        is_leaf: false,
        is_dirty: true,
        bucket: None,
        bucket_key: Vec::new(),
        latch: RwLock::new(()),
        sub_nodes_latch: (0..rdx.max(0)).map(|_| RwLock::new(())).collect(),
        cached_nodes_latch: (0..rdx.max(0)).map(|_| RwLock::new(())).collect(),
    }))
}

fn new_leaf_node(
    parent: Option<Arc<RwLock<MGTNode>>>,
    rdx: i32,
    bucket_key: Vec<i32>,
    bucket: Option<Arc<RwLock<Bucket>>>,
) -> Arc<RwLock<MGTNode>> {
    Arc::new(RwLock::new(MGTNode {
        node_hash: [0; 32],
        parent,
        sub_nodes: vec![None; rdx.max(0) as usize],
        data_hashes: Vec::new(),
        cached_nodes: vec![None; rdx.max(0) as usize],
        cached_data_hashes: Vec::new(),
        is_leaf: true,
        is_dirty: true,
        bucket,
        bucket_key,
        latch: RwLock::new(()),
        sub_nodes_latch: (0..rdx.max(0)).map(|_| RwLock::new(())).collect(),
        cached_nodes_latch: (0..rdx.max(0)).map(|_| RwLock::new(())).collect(),
    }))
}

fn ensure_node_slots(n: &mut MGTNode, rdx: i32) {
    let need = rdx.max(0) as usize;
    if n.sub_nodes.len() < need { n.sub_nodes.resize_with(need, || None); }
    if n.cached_nodes.len() < need { n.cached_nodes.resize_with(need, || None); }
    if n.sub_nodes_latch.len() < need { n.sub_nodes_latch.resize_with(need, || RwLock::new(())); }
    if n.cached_nodes_latch.len() < need { n.cached_nodes_latch.resize_with(need, || RwLock::new(())); }
}

// ---- node hash maintenance ----

// Recompute a node's `data_hashes` and `cached_data_hashes` from its children (if any),
// then set `node_hash = sha256(concat(data_hashes || cached_data_hashes))`.
// Returns the freshly computed node_hash.
fn recompute_node_hash(node: &Arc<RwLock<MGTNode>>) -> [u8; 32] {
    // Check leaf first; for a leaf, data_hashes are the bucket's per-segment Merkle roots.
    let (is_leaf, bucket_opt, sub_nodes, cached_nodes) = {
        let n = node.read().unwrap();
        (n.is_leaf, n.bucket.clone(), n.sub_nodes.clone(), n.cached_nodes.clone())
    };

    let mut data_hashes: Vec<Vec<u8>> = Vec::new();
    let mut cached_data_hashes: Vec<Vec<u8>> = Vec::new();

    if is_leaf {
        if let Some(b) = bucket_opt {
            // Deterministic ordering by segment key
            let mut roots: Vec<[u8; 32]> = Vec::new();
            let b_g = b.read().unwrap();
            let mts_map = b_g.merkle_trees.read().unwrap();
            let mut keys: Vec<String> = mts_map.keys().cloned().collect();
            keys.sort();
            for k in keys.into_iter() {
                if let Some(root) = mts_map.get(&k).and_then(|mt| mt.get_root_hash()) {
                    roots.push(root);
                }
            }
            drop(mts_map);
            drop(b_g);
            for r in roots { data_hashes.push(r.to_vec()); }
        }
        // cached_data_hashes remains empty for leaves
    } else {
        // Internal node: data_hashes/cached_data_hashes are children's node_hash (sub_nodes/cached_nodes)
        for opt in sub_nodes.into_iter() {
            if let Some(child) = opt {
                let h = child.read().unwrap().node_hash;
                data_hashes.push(h.to_vec());
            }
        }
        for opt in cached_nodes.into_iter() {
            if let Some(child) = opt {
                let h = child.read().unwrap().node_hash;
                cached_data_hashes.push(h.to_vec());
            }
        }
    }

    let mut hasher = Sha256::new();
    for h in &data_hashes { hasher.update(h); }
    for h in &cached_data_hashes { hasher.update(h); }
    let digest: [u8; 32] = hasher.finalize().into();

    let mut n = node.write().unwrap();
    n.data_hashes = data_hashes;
    n.cached_data_hashes = cached_data_hashes;
    n.node_hash = digest;
    n.node_hash
}

// Propagate hash recomputation from `start` up to the root.
fn propagate_hash_up(start: &Arc<RwLock<MGTNode>>) {
    let mut cur = Some(start.clone());
    while let Some(node) = cur {
        recompute_node_hash(&node);
        let parent = { node.read().unwrap().parent.clone() };
        cur = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    fn bucket_with_key(rdx: i32, key: Vec<i32>) -> Arc<RwLock<Bucket>> {
        let mut b = Bucket::new(0, rdx, 1024, 1);
        b.set_bucket_key(key);
        Arc::new(RwLock::new(b))
    }

    // 深度优先打印树结构，同时收集所有叶子节点的路径->bucket_key 映射。
    // 路径以根为起点，用 "/" 分隔各层的子下标（例如：/1/2 表示 root->child[1]->child[2]）。
    fn dump_tree_and_collect(
        node: &Arc<RwLock<MGTNode>>,
        rdx: i32,
        path: &mut Vec<usize>,
        lines: &mut Vec<String>,
        leaf_map: &mut Vec<(String, Vec<i32>)>,
    ) {
        // 拷贝当前节点需要的信息，避免持锁递归
        let (is_leaf, bk, has_bucket, subs) = {
            let n = node.read().unwrap();
            (n.is_leaf, n.bucket_key.clone(), n.bucket.is_some(), n.sub_nodes.clone())
        };

        let path_str = if path.is_empty() {
            "/".to_string()
        } else {
            let mut s = String::new();
            s.push('/');
            for (i, idx) in path.iter().enumerate() {
                if i > 0 { s.push('/'); }
                let _ = write!(s, "{}", idx);
            }
            s
        };

        if is_leaf {
            let mut line = format!("path={}  [L] bucket_key(leaf->root) = {:?}", path_str, bk);
            if !has_bucket {
                line.push_str("  (warn: leaf without bucket)");
            }
            lines.push(line);
            leaf_map.push((path_str.clone(), bk.clone()));
        } else {
            lines.push(format!("path={}  [I]", path_str));
        }

        // 递归子节点（仅遍历 sub_nodes，忽略 cached_nodes）
        for i in 0..(rdx.max(0) as usize) {
            if let Some(Some(child)) = subs.get(i).cloned() {
                path.push(i);
                dump_tree_and_collect(&child, rdx, path, lines, leaf_map);
                path.pop();
            }
        }
    }

    fn print_tree_and_leaves(mgt: &MGT, title: &str) -> Vec<(String, Vec<i32>)> {
        println!("===== {} =====", title);
        if let Some(root) = &mgt.root {
            let mut lines = Vec::new();
            let mut leaves = Vec::new();
            dump_tree_and_collect(root, mgt.rdx, &mut Vec::new(), &mut lines, &mut leaves);
            for l in &lines { println!("{}", l); }
            println!("-- leaves (path -> bucket_key) --");
            for (p, bk) in &leaves { println!("{} -> {:?}", p, bk); }
            leaves
        } else {
            println!("<empty tree: root=None>");
            Vec::new()
        }
    }

    #[test]
    fn test_mgt_update_print_before_after() {
        let rdx = 4;
        let mut mgt = MGT::new(rdx);

        // 初始状态：空树
        let _leaves0 = print_tree_and_leaves(&mgt, "before update (empty)");
        assert!(mgt.root.is_none());

        // 第一次更新：构建两片叶子 /1 与 /3
        let phase1 = vec![
            vec![
                bucket_with_key(rdx, vec![1]),
                bucket_with_key(rdx, vec![3]),
            ],
        ];
        mgt.mgt_update(phase1);
        let leaves1 = print_tree_and_leaves(&mgt, "after phase-1 (baseline)");
        let h1 = mgt.root.as_ref().map(|r| r.read().unwrap().node_hash).unwrap();
        // 期望：/1 -> [1]，/3 -> [3]
        assert_eq!(leaves1.len(), 2);
        assert!(leaves1.iter().any(|(p, bk)| p == "/1" && bk == &vec![1]));
        assert!(leaves1.iter().any(|(p, bk)| p == "/3" && bk == &vec![3]));

        // 第二次更新：把原 /1 这片叶子“下沉”为内部，再挂两片新叶 /1/0 与 /1/2
        let phase2 = vec![
            vec![
                bucket_with_key(rdx, vec![0, 1]), // 路径 root->1->0
                bucket_with_key(rdx, vec![2, 1]), // 路径 root->1->2
            ],
        ];
        mgt.mgt_update(phase2);
        let leaves2 = print_tree_and_leaves(&mgt, "after phase-2 (updated)");
        let h2 = mgt.root.as_ref().map(|r| r.read().unwrap().node_hash).unwrap();

        // 期望：/3 仍为 [3]；/1 不再是叶子；新增 /1/0 与 /1/2
        assert!(leaves2.iter().any(|(p, bk)| p == "/3" && bk == &vec![3]));
        assert!(!leaves2.iter().any(|(p, _)| p == "/1"));
        assert!(leaves2.iter().any(|(p, bk)| p == "/1/0" && bk == &vec![0, 1]));
        assert!(leaves2.iter().any(|(p, bk)| p == "/1/2" && bk == &vec![2, 1]));
        // 结构变化应导致根 hash 改变
        assert_ne!(h1, h2);
    }

    fn fmt_path(nodes: &Vec<Arc<RwLock<MGTNode>>>) -> String {
        let mut parts = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            let g = n.read().unwrap();
            let kind = if g.is_leaf { 'L' } else { 'I' };
            parts.push(format!("{}:{:?}", kind, g.bucket_key));
            // avoid holding lock across loop body end
            drop(g);
            if i + 1 < nodes.len() { parts.push(" -> ".to_string()); }
        }
        parts.concat()
    }

    // 将 leaf/internal->root 的节点向量，转换成“/i/j/k”形式的根->目标路径
    fn fmt_route(nodes: &Vec<Arc<RwLock<MGTNode>>>) -> String {
        if nodes.is_empty() { return "/".to_string(); }
        if nodes.len() == 1 { return "/".to_string(); }
        let mut idxs: Vec<usize> = Vec::new();
        for i in (0..nodes.len() - 1).rev() {
            let parent = &nodes[i + 1];
            let child_ptr = Arc::as_ptr(&nodes[i]);
            let (subs, cacheds) = {
                let p = parent.read().unwrap();
                (p.sub_nodes.clone(), p.cached_nodes.clone())
            };
            let mut found: Option<usize> = None;
            for (idx, op) in subs.iter().enumerate() {
                if let Some(a) = op {
                    if Arc::as_ptr(a) == child_ptr { found = Some(idx); break; }
                }
            }
            if found.is_none() {
                for (idx, op) in cacheds.iter().enumerate() {
                    if let Some(a) = op {
                        if Arc::as_ptr(a) == child_ptr { found = Some(idx); break; }
                    }
                }
            }
            if let Some(idx) = found { idxs.push(idx); } else { return "/?".to_string(); }
        }
        let mut s = String::from("/");
        for (i, idx) in idxs.iter().enumerate() {
            if i > 0 { s.push('/'); }
            s.push_str(&idx.to_string());
        }
        s
    }

    fn expected_route_from_bucket_key(bk: &Vec<i32>) -> String {
        if bk.is_empty() { return "/".to_string(); }
        let mut s = String::from("/");
        for (i, d) in bk.iter().rev().enumerate() {
            if i > 0 { s.push('/'); }
            s.push_str(&d.to_string());
        }
        s
    }

    #[test]
    fn test_mgt_get_paths_after_updates() {
        let rdx = 4;
        let mut mgt = MGT::new(rdx);

        // 构建与上一测试相同的两阶段树
        let phase1 = vec![ vec![ bucket_with_key(rdx, vec![1]), bucket_with_key(rdx, vec![3]) ] ];
        mgt.mgt_update(phase1);
        let phase2 = vec![ vec![ bucket_with_key(rdx, vec![0, 1]), bucket_with_key(rdx, vec![2, 1]) ] ];
        mgt.mgt_update(phase2);

        // 验证 get_leaf_node_and_path
        for target in [vec![3], vec![0, 1], vec![2, 1]] {
            let path = mgt.get_leaf_node_and_path(target.clone()).expect("leaf path must exist");
            let route = fmt_route(&path);
            println!("leaf {:?} path (leaf->root): {}", target, fmt_path(&path));
            println!("leaf {:?} route (root->leaf): {}", target, route);
            assert!(!path.is_empty());
            // 叶子在最前，且 bucket_key 匹配
            let first = path.first().unwrap().read().unwrap();
            assert!(first.is_leaf);
            assert_eq!(first.bucket_key, target);
            drop(first);
            // 根在最后，且无父节点
            let last = path.last().unwrap().read().unwrap();
            assert!(last.parent.is_none());
            // 检查 route == 反转 bucket_key
            assert_eq!(route, expected_route_from_bucket_key(&target));
        }

        // 原来的 [1] 叶子被下沉为内部，查询应失败
        assert!(mgt.get_leaf_node_and_path(vec![1]).is_err());

        // 验证 get_internal_node_and_path
        // 1) /1 是内部节点，bucket_key=[1]
        let ibk_1 = vec![1];
        let ipath_1 = mgt.get_internal_node_and_path(ibk_1.clone()).expect("internal [1] should exist");
        let iroute_1 = fmt_route(&ipath_1);
        println!("internal {:?} path (node->root): {}", ibk_1, fmt_path(&ipath_1));
        println!("internal {:?} route (root->node): {}", ibk_1, iroute_1);
        assert_eq!(ipath_1.len(), 2); // [/1, /]
        let first = ipath_1.first().unwrap().read().unwrap();
        assert!(!first.is_leaf);
        assert_eq!(first.bucket_key, ibk_1);
        drop(first);
        let last = ipath_1.last().unwrap().read().unwrap();
        assert!(last.parent.is_none());
        drop(last);
        assert_eq!(iroute_1, expected_route_from_bucket_key(&ibk_1));

        // 2) 根作为内部节点（bucket_key=[]）
        let ibk_root: Vec<i32> = Vec::new();
        let ipath_root = mgt.get_internal_node_and_path(ibk_root.clone()).expect("root internal exists");
        let iroute_root = fmt_route(&ipath_root);
        println!("internal root [] path (node->root): {}", fmt_path(&ipath_root));
        println!("internal root [] route (root->node): {}", iroute_root);
        assert_eq!(ipath_root.len(), 1);
        let root = ipath_root[0].read().unwrap();
        assert!(root.parent.is_none());
        assert!(!root.is_leaf);
        drop(root);
        assert_eq!(iroute_root, expected_route_from_bucket_key(&ibk_root));

        // 3) 不存在的内部节点
        assert!(mgt.get_internal_node_and_path(vec![2]).is_err());
    }

    #[test]
    fn test_leaf_hash_from_bucket_merkle_roots() {
        use crate::ads::mest::kvpair::KVPair;
        // Prepare a bucket with data so its merkle tree has a non-empty root
        let rdx = 4;
        let mut b = Bucket::new(0, rdx, 100, 2);
        b.set_bucket_key(vec![2]);
        b.insert(KVPair::new("ka".to_string(), "va".to_string()));
        b.insert(KVPair::new("kb".to_string(), "vb".to_string()));
        let b = Arc::new(RwLock::new(b));

        let mut mgt = MGT::new(rdx);
        mgt.mgt_update(vec![vec![b.clone()]]);

        // locate leaf [2]
        let path = mgt.get_leaf_node_and_path(vec![2]).expect("leaf [2] must exist");
        let leaf = path.first().unwrap().read().unwrap();
        assert!(leaf.is_leaf);
        // data_hashes should equal the roots of all merkle trees in the bucket (sorted by seg key)
        let b_g = b.read().unwrap();
        let mts = b_g.merkle_trees.read().unwrap();
        let mut keys: Vec<String> = mts.keys().cloned().collect();
        keys.sort();
        let mut expect: Vec<Vec<u8>> = Vec::new();
        for k in keys {
            if let Some(r) = mts.get(&k).and_then(|mt| mt.get_root_hash()) {
                expect.push(r.to_vec());
            }
        }
        assert_eq!(leaf.data_hashes, expect);
    }
}
