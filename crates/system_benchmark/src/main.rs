use anyhow::{Context, Result};
use colored::Colorize;
use common::{metrics_output, AdsMode};
use std::env;
use std::path::PathBuf;
use system_benchmark::{ProcessManager, SystemReportGenerator, SystemTestRunner};

#[tokio::main]
async fn main() -> Result<()> {
    print_banner();

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

    println!("Test Configuration:");
    println!("  Workload: {}", workload_path.display());
    println!("  ADS Mode: {}", ads_mode_str.to_uppercase());
    println!("  Storager Nodes: {}", num_storagers);

    let ads_mode = ads_mode_str.parse().unwrap_or_else(|err| {
        println!("Warning: {}, defaulting to MPT", err);
        AdsMode::Mpt
    });
    let dataset_label = metrics_output::dataset_label_from_path(&workload_path);

    let storager_ports: Vec<u16> = (0..num_storagers).map(|i| 50052 + i as u16).collect();
    let mut process_manager = ProcessManager::new(storager_ports);

    process_manager
        .start_system(&ads_mode_str)
        .await
        .context("Failed to start system")?;

    println!("\nWarming up system...");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let manager_addr = process_manager.manager_addr();
    let mut runner = SystemTestRunner::new(manager_addr, ads_mode);

    println!("\n{}", "=".repeat(80).bright_cyan());
    println!("{}", "  STARTING SYSTEM BENCHMARK".bright_cyan().bold());
    println!("{}", "=".repeat(80).bright_cyan());

    let test_result = runner.run_test(&workload_path).await;

    runner.print_summary();

    if test_result.is_ok() {
        println!("\n{}", "Generating report...".bright_cyan());
        SystemReportGenerator::generate_report(
            &dataset_label,
            &ads_mode_str,
            runner.metrics(),
            &PathBuf::from("experiments/logs"),
        )
        .context("Failed to generate report")?;
    }

    println!("\n{}", "Shutting down system...".bright_yellow());
    process_manager.shutdown()?;

    if let Err(e) = test_result {
        eprintln!("\n{} {}", "Test failed:".bright_red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "\n{}",
        "System benchmark completed successfully!"
            .bright_green()
            .bold()
    );

    Ok(())
}

fn print_banner() {
    let banner = r#"
================================================================================
  System-Level Performance Benchmark
  Testing Complete Architecture: Client -> Manager -> Storager(s)
================================================================================
"#;

    println!("{}", banner.bright_cyan().bold());
}
