pub mod db;
pub mod error;
pub mod leveldb;
pub mod mpt;
pub mod node;
pub mod proof;
pub mod unified_adapter;
pub mod utils;

pub use db::MemoryDatabase;
pub use error::MPTError;
pub use leveldb::{LevelDbDatabase, MPT_METADATA_KEY, MPT_ROOT_HASH_KEY};
pub use mpt::MPT;
pub use node::{FullNode, NodeCache, ShortNode};
pub use proof::{MPTProof, ProofElement};
pub use unified_adapter::MptAdapter;
pub use utils::KVPair;

#[cfg(test)]
mod tests {
    #[test]
    fn test_basic_operations() {
        // 基本测试将在这里添加
    }
}
