//! ESA (Efficient Set Accumulator) Library
//!
//! 这个库提供了多种认证数据结构 (Authenticated Data Structures) 的实现
//!
//! ## 当前实现
//! - **MPT**: Merkle Patricia Trie (以太坊风格)
//! - **MEST**: Merkle-based Extendible Segmented Hash Tree (可扩展分段哈希树)
//! - **AccTrie**: Accumulator-based Trie (基于累加器的前缀树)
//!
//! ## 架构
//! 每种 ADS 实现都在独立的模块中，共享通用的摘要和集合工具

// ========================================
// Common utilities (shared across all ADS)
// ========================================

/// Digest utilities - 通用摘要工具
pub mod digest;
pub use digest::*;

/// Set operations - 通用集合操作
pub mod set;
pub use set::*;

// ========================================
// ADS Implementations
// ========================================

/// Merkle Patricia Trie implementation
pub mod mpt;

/// Merkle-based Extendible Segmented Hash Tree implementation
pub mod mest;

/// Accumulator-based Trie implementation
pub mod acctrie;
