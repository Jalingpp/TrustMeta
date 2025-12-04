//! AccTrie - 带密码学累加器的前缀树
//!
//! 此库实现了一个结合密码学累加器的前缀树数据结构。每个叶子节点维护一个值集合
//! 及其对应的密码学累加器，支持高效的成员证明和集合操作。

// ================================================================================================
// Public Modules
// ================================================================================================

pub mod acc;
pub mod trie;
pub mod unified_adapter;
#[cfg(test)]
mod batch_tests;


// ================================================================================================
// Re-exports
// ================================================================================================

// 累加器相关
pub use acc::{DynamicAccumulator, G1Affine};

// 前缀树结构
pub use trie::{AccTrie, InternalNode, LeafNode, Node, InsertionProof, DeletionProof, UpdateProof, QueryResult, ExistenceProof, NonExistenceProof, AuditResult};

// 统一适配器
pub use unified_adapter::AccTrieAdapter;
