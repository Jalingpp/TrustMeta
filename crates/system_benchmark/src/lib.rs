//! 系统级性能测试框架
//!
//! 测试完整的分布式存储系统架构：
//! Client → Manager → Storager(s)

pub mod metrics;
pub mod process_manager;
pub mod report;
pub mod system_runner;

pub use metrics::SystemMetrics;
pub use process_manager::ProcessManager;
pub use report::SystemReportGenerator;
pub use system_runner::SystemTestRunner;
