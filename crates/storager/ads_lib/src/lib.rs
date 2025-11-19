//! ESA (Efficient Set Accumulator) Library
//!
//! 这个库提供了多种认证数据结构 (Authenticated Data Structures) 的实现
//!
//! ## 当前实现
//! - **MPT**: Merkle Patricia Trie
//!
//! ## 未来扩展
//! 可以添加其他 ADS 实现，例如:
//! - Merkle Tree
//! - Vector Commitment
//! 等等

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
