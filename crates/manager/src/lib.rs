//! Manager Crate
//!
//! 分层架构：
//! - `api`: gRPC 接口层
//! - `core`: 核心业务逻辑层

pub mod api;
pub mod core;

// 重新导出常用类型，方便外部使用
pub use core::{ConsistentHashRing, EPRing, Manager, RouteTarget, Router};
