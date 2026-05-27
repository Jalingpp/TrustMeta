//! 系统级性能报告生成器

use crate::metrics::SystemMetrics;
use anyhow::{Context, Result};
use chrono::Local;
use colored::Colorize;
use serde_json;
use std::fs;
use std::path::Path;
use tabled::{builder::Builder, settings::Style};

pub struct SystemReportGenerator;

impl SystemReportGenerator {
    /// 生成系统级性能报告
    pub fn generate_report(
        dataset: &str,
        ads_mode: &str,
        metrics: &SystemMetrics,
        output_dir: &Path,
    ) -> Result<()> {
        // 创建输出目录
        fs::create_dir_all(output_dir).context("Failed to create output directory")?;

        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let report_name = format!("system_test_{}_{}_{}", dataset, ads_mode, timestamp);
        let report_dir = output_dir.join(&report_name);
        fs::create_dir_all(&report_dir)?;

        // 生成文本报告
        Self::generate_text_report(dataset, ads_mode, metrics, &report_dir)?;

        // 生成 JSON 数据
        Self::generate_json_report(metrics, &report_dir)?;

        println!("\n{}", "📝 Report Generated:".bright_green().bold());
        println!("  📁 {}", report_dir.display());

        Ok(())
    }

    /// 生成文本报告
    fn generate_text_report(
        dataset: &str,
        ads_mode: &str,
        metrics: &SystemMetrics,
        output_dir: &Path,
    ) -> Result<()> {
        let mut report = String::new();

        report.push_str(&format!(
            "# System Benchmark Report - {} (Dataset: {}, ADS: {})\n\n",
            Local::now().format("%Y-%m-%d %H:%M:%S"),
            dataset,
            ads_mode.to_uppercase()
        ));

        // 概览
        report.push_str("## Overview\n\n");
        report.push_str(&format!(
            "- Total Operations: {}\n",
            metrics.success_count + metrics.failure_count
        ));
        report.push_str(&format!("- Success Count: {}\n", metrics.success_count));
        report.push_str(&format!("- Failure Count: {}\n", metrics.failure_count));
        report.push_str(&format!("- Success Rate: {:.2}%\n", metrics.success_rate()));
        report.push_str(&format!(
            "- Total Duration: {:.2}s\n",
            metrics.total_duration.as_secs_f64()
        ));
        report.push_str(&format!(
            "- Throughput: {:.2} ops/sec\n\n",
            metrics.total_throughput
        ));

        // 操作统计
        report.push_str("## Operation Statistics\n\n");
        let mut builder = Builder::default();
        builder.push_record(["Operation", "Count"]);
        builder.push_record(["Add", &metrics.operation_stats.add_count.to_string()]);
        builder.push_record(["Query", &metrics.operation_stats.query_count.to_string()]);
        builder.push_record(["Update", &metrics.operation_stats.update_count.to_string()]);
        builder.push_record(["Delete", &metrics.operation_stats.delete_count.to_string()]);
        let table = builder.build().with(Style::markdown()).to_string();
        report.push_str(&table);
        report.push_str("\n\n");

        // 延迟统计
        report.push_str("## End-to-End Latency (ms)\n\n");
        let mut builder = Builder::default();
        builder.push_record(["Metric", "Value (ms)"]);
        builder.push_record(["Min", &format!("{:.3}", metrics.end_to_end_latency.min_ms)]);
        builder.push_record([
            "Average",
            &format!("{:.3}", metrics.end_to_end_latency.avg_ms),
        ]);
        builder.push_record(["Max", &format!("{:.3}", metrics.end_to_end_latency.max_ms)]);
        builder.push_record(["P50", &format!("{:.3}", metrics.end_to_end_latency.p50_ms)]);
        builder.push_record(["P95", &format!("{:.3}", metrics.end_to_end_latency.p95_ms)]);
        builder.push_record(["P99", &format!("{:.3}", metrics.end_to_end_latency.p99_ms)]);
        let table = builder.build().with(Style::markdown()).to_string();
        report.push_str(&table);
        report.push_str("\n");

        // 保存报告
        let report_path = output_dir.join("metrics.txt");
        fs::write(&report_path, report).context("Failed to write text report")?;

        Ok(())
    }

    /// 生成 JSON 报告
    fn generate_json_report(metrics: &SystemMetrics, output_dir: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(metrics).context("Failed to serialize metrics")?;
        let json_path = output_dir.join("metrics.json");
        fs::write(&json_path, json).context("Failed to write JSON report")?;
        Ok(())
    }
}
