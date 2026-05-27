use sha2::{Digest, Sha256};

fn hash_leaf(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn hash_internal(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(left);
    buf.extend_from_slice(right);
    Sha256::digest(&buf).into()
}

#[derive(Clone, Debug)]
pub struct MerkleTree {
    root_hash: Option<[u8; 32]>,
    data_list: Vec<Vec<u8>>,
}

impl MerkleTree {
    pub fn new_empty() -> Self {
        MerkleTree {
            root_hash: None,
            data_list: Vec::new(),
        }
    }

    pub fn new(data: Vec<Vec<u8>>) -> Self {
        let mut mt = MerkleTree {
            root_hash: None,
            data_list: data,
        };
        mt.recompute();
        mt
    }

    pub fn get_root_hash(&self) -> Option<[u8; 32]> {
        self.root_hash
    }

    pub fn update_root(&mut self, i: usize, data: Vec<u8>) -> [u8; 32] {
        if i < self.data_list.len() {
            self.data_list[i] = data;
        } else {
            // 若越界则追加
            self.data_list.push(data);
        }
        self.recompute();
        self.root_hash.unwrap_or([0; 32])
    }

    pub fn insert_data(&mut self, data: Vec<u8>) -> [u8; 32] {
        self.data_list.push(data);
        self.recompute();
        self.root_hash.unwrap_or([0; 32])
    }

    fn recompute(&mut self) {
        if self.data_list.is_empty() {
            self.root_hash = None;
            return;
        }
        let mut level: Vec<[u8; 32]> = self.data_list.iter().map(|d| hash_leaf(d)).collect();
        while level.len() > 1 {
            let mut next = Vec::with_capacity((level.len() + 1) / 2);
            let mut i = 0;
            while i < level.len() {
                if i + 1 < level.len() {
                    next.push(hash_internal(&level[i], &level[i + 1]));
                } else {
                    // 若为奇数个，直接提升最后一个
                    next.push(level[i]);
                }
                i += 2;
            }
            level = next;
        }
        self.root_hash = level.first().cloned();
    }

    // 公开数据列表的只读视图（用于上层定位索引/构造证明）
    pub fn data_len(&self) -> usize {
        self.data_list.len()
    }

    pub fn get_proof_for_index(&self, idx: usize) -> Option<MHTProof> {
        if self.data_list.is_empty() || idx >= self.data_list.len() {
            return None;
        }
        let mut level: Vec<[u8; 32]> = self.data_list.iter().map(|d| hash_leaf(d)).collect();
        let mut proof_pairs: Vec<(u8, [u8; 32])> = Vec::new();
        let mut cur_idx = idx;
        while level.len() > 1 {
            let mut next: Vec<[u8; 32]> = Vec::with_capacity((level.len() + 1) / 2);
            let mut next_idx = 0usize;
            let mut j = 0usize;
            while j < level.len() {
                if j + 1 < level.len() {
                    let left = level[j];
                    let right = level[j + 1];
                    let parent = hash_internal(&left, &right);
                    if cur_idx == j {
                        proof_pairs.push((1u8, right));
                        next_idx = next.len();
                    } else if cur_idx == j + 1 {
                        proof_pairs.push((0u8, left));
                        next_idx = next.len();
                    }
                    next.push(parent);
                    j += 2;
                } else {
                    if cur_idx == j {
                        next_idx = next.len();
                    }
                    next.push(level[j]);
                    j += 1;
                }
            }
            level = next;
            cur_idx = next_idx;
        }
        Some(MHTProof { proof_pairs })
    }
}

#[derive(Clone, Debug)]
pub struct MHTProof {
    pub proof_pairs: Vec<(u8, [u8; 32])>,
}

pub fn verify_proof(leaf_data: &[u8], root: [u8; 32], proof: &MHTProof) -> bool {
    let mut cur: [u8; 32] = hash_leaf(leaf_data);
    for (dir, sib) in &proof.proof_pairs {
        cur = if *dir == 0 {
            // sibling is left
            hash_internal(sib, &cur)
        } else {
            hash_internal(&cur, sib)
        };
    }
    cur == root
}
