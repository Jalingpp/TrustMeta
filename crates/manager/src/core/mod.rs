//! Core 层模块
//!
//! 包含业务逻辑、状态管理、路由和基础算法

pub mod consistent_hash;
pub mod epring;
pub mod manager;
pub mod routing;

// 重新导出常用类型
pub use consistent_hash::ConsistentHashRing;
pub use epring::{EPRing, EPRingRoute, EPRingSplitEvent};
pub use manager::{Manager, PendingOperation};

pub use routing::{PrefixSplitPlan, RouteMode, RouteTarget, Router};
