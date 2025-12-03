//! ADS-Rust: 认证数据结构库 (Authenticated Data Structures Library)
//!
//! 提供三种高性能认证数据结构的 Rust 实现，支持可验证的数据操作和密码学证明
//!
//! ## 核心实现
//! - **MPT**: Merkle Patricia Trie - 基于 Merkle 树的持久化键值存储
//! - **MEST**: Merkle-based Extendible Segmented Hash Tree - 可扩展的高吞吐量哈希树
//! - **AccTrie**: Accumulator-based Trie - 基于密码学累加器的可验证前缀树
//!
//! ## 技术特性
//! - **密码学安全**: 使用 BLS12-381 椭圆曲线提供强验证保证
//! - **完整证明**: 所有操作生成可审计的密码学证明
//! - **高性能**: 并行计算优化，零成本抽象
//! - **模块化**: 共享通用摘要和集合工具，独立的 ADS 实现

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

// ========================================
// Unified ADS Interface
// ========================================

/// 统一的ADS抽象接口
pub mod unified_ads;
