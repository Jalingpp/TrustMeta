use crate::acctree::accumulator_ads::{DynamicAccumulator, G1Affine, Set};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use ark_serialize::CanonicalSerialize;

pub type Hash = [u8; 32];

pub static EMPTY_HASH: LazyLock<Hash> = LazyLock::new(|| {
    leaf_hash("", "")
});

pub static EMPTY_ACC: LazyLock<G1Affine> = LazyLock::new(|| {
    DynamicAccumulator::empty_commitment()
});

pub fn empty_hash() -> Hash {
    *EMPTY_HASH
}

pub fn empty_acc() -> G1Affine {
    *EMPTY_ACC
}

pub fn leaf_hash(key: &str, fid: &str) -> Hash {
    let mut hasher = Sha256::new();
    
    hasher.update(&[0u8]); 

    hasher.update((key.len() as u32).to_be_bytes());
    hasher.update(key.as_bytes());
    
    hasher.update((1u32).to_be_bytes()); // fid count = 1
    hasher.update((fid.len() as u32).to_be_bytes());
    hasher.update(fid.as_bytes());

    hasher.finalize().into()
}

pub fn nonleaf_hash(left: Hash, right: Hash, keys: &Set<String>, acc: &G1Affine) -> Hash {
    let mut hasher = Sha256::new();
    
    hasher.update(&[1u8]); 
    
    hasher.update(left);
    hasher.update(right);

    // Keys (Sorted)
    let mut keys_refs: Vec<&String> = keys.iter().collect();
    keys_refs.sort();
    hasher.update((keys_refs.len() as u32).to_be_bytes());
    for k in keys_refs {
        hasher.update((k.len() as u32).to_be_bytes());
        hasher.update(k.as_bytes());
    }

    // Acc - using ark_serialize::CanonicalSerialize
    let mut acc_bytes = Vec::new();
    acc.serialize(&mut acc_bytes).expect("failed to serialize acc");
    
    hasher.update((acc_bytes.len() as u32).to_be_bytes());
    hasher.update(&acc_bytes);

    hasher.finalize().into()
}
