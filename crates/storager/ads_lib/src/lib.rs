//! ADS-Rust: Authenticated Data Structures Library
//!
//! Provides multiple authenticated data structure implementations.

// ========================================
// Common utilities (shared across all ADS)
// ========================================

pub mod digest;
pub use digest::*;

pub mod set;
pub use set::*;

// ========================================
// ADS Implementations
// ========================================

pub mod acctrie;
pub mod mest;
pub mod mpt;

// ========================================
// Unified ADS Interface
// ========================================

pub mod unified_ads;
