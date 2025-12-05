//! 系统级性能测试主程序
//!
//! 测试完整的分布式存储系统：Client → Manager → Storager(s)
//!
//! 功能：
//! 1. 自动启动 Manager 和多个 Storager 进程
//! 2. 通过 Client 执行完整的工作负载测试
//! 3. 测量端到端性能（包含网络通信、路由、证明验证等）
//! 4. 生成详细的性能报告
//!
//! 用法：
//! ```bash
//! # 使用默认配置（小规模测试，MPT模式）
//! cargo run --release --bin system_benchmark
//!
//! # 指定 workload 和 ADS 模式
//! cargo run --release --bin system_benchmark data/workload_medium_10000.csv mest
//!
//! # 指定所有参数
//! cargo run --release --bin system_benchmark <workload_path> <ads_mode> <num_storagers>
//! ```

use anyhow::{Context, Result};
use colored::Colorize;
use common::AdsMode;
use std::env;
use std::path::PathBuf;
use system_benchmark::{ProcessManager, SystemReportGenerator, SystemTestRunner};

#[tokio::main]
async fn main() -> Result<()> {
    print_banner();

    // 解析命令行参数
    let args: Vec<String> = env::args().collect();

    let workload_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from("data/workload_small_1000.csv")
    };

    let ads_mode_str = if args.len() > 2 {
        args[2].clone()
    } else {
        "mpt".to_string()
    };

    let num_storagers: usize = if args.len() > 3 {
        args[3].parse().unwrap_or(3)
    } else {
        3
    };

    println!("📋 Test Configuration:");
    println!("  Workload: {}", workload_path.display());
    println!("  ADS Mode: {}", ads_mode_str.to_uppercase());
    println!("  Storager Nodes: {}", num_storagers);

    let ads_mode = match ads_mode_str.to_lowercase().as_str() {
        "mpt" => AdsMode::Mpt,
        "mest" => AdsMode::Mest,
        "acctrie" => AdsMode::AccTrie,
        _ => {
            println!("⚠️  Unknown ADS mode '{}', defaulting to MPT", ads_mode_str);
            AdsMode::Mpt
        }
    };

    // 配置端口
    let manager_port = 50051;
    let storager_ports: Vec<u16> = (0..num_storagers)
        .map(|i| 50052 + i as u16)
        .collect();

    // 创建进程管理器
    let mut process_manager = ProcessManager::new(manager_port, storager_ports);

    // 启动系统
    process_manager
        .start_system(&ads_mode_str)
        .await
        .context("Failed to start system")?;

    // 等待系统稳定
    println!("\n⏳ Warming up system...");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // 创建测试运行器
    let manager_addr = process_manager.manager_addr();
    let mut runner = SystemTestRunner::new(manager_addr, ads_mode);

    // 运行测试
    println!("\n{}", "═".repeat(80).bright_cyan());
    println!("{}", "  STARTING SYSTEM BENCHMARK".bright_cyan().bold());
    println!("{}", "═".repeat(80).bright_cyan());

    let test_result = runner.run_test(&workload_path).await;

    // 打印摘要
    runner.print_summary();

    // 生成报告
    if test_result.is_ok() {
        println!("\n{}", "📊 Generating report...".bright_cyan());
        SystemReportGenerator::generate_report(&ads_mode_str, runner.metrics(), &PathBuf::from("logs"))
            .context("Failed to generate report")?;
    }

    // 关闭系统
    println!("\n{}", "🛑 Shutting down system...".bright_yellow());
    process_manager.shutdown()?;

    if let Err(e) = test_result {
        eprintln!("\n{} {}", "❌ Test failed:".bright_red().bold(), e);
        std::process::exit(1);
    }

    println!("\n{}", "✨ System benchmark completed successfully! ✨".bright_green().bold());

    Ok(())
}

fn print_banner() {
    let banner = r#"
    ╔═══════════════════════════════════════════════════════════════════╗
    ║                                                                   ║
    ║           System-Level Performance Benchmark                      ║
    ║                                                                   ║
    ║        Testing Complete Architecture: Client → Manager →         ║
    ║                         Storager(s)                               ║
    ║                                                                   ║
    ╚═══════════════════════════════════════════════════════════════════╝
    "#;

    println!("{}", banner.bright_cyan().bold());
}
