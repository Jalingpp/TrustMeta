pub mod mgt;
pub mod bucket;
pub mod kvpair;
pub mod merkletree;
pub mod seh;
pub mod util;
// meht 依赖外部 DB 和缓存库，默认关闭，防止编译失败
#[cfg(feature = "meht")]
pub mod meht;

pub use mgt::{MGT, MGTNode};
pub use bucket::Bucket;
pub use kvpair::KVPair;
#[cfg(feature = "meht")]
pub use meht::MEHT;
pub use util::{compute_stride_by_base};
pub use merkletree::MerkleTree;
pub use mgt::{
    MGTProof, MGTProofStep, build_mgt_proof, verify_mgt_proof,
    KeyProof, BucketProofOut, verify_key_proof,
};
