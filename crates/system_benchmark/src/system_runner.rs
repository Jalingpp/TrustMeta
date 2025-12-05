//! 系统测试运行器 - 执行完整的端到端测试

use crate::metrics::SystemMetrics;
use anyhow::{Context, Result};
use client::Client;
use colored::Colorize;
use common::AdsMode;
use csv::ReaderBuilder;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone)]
struct WorkloadRecord {
    fid: String,
    keywords: Vec<String>,
}

/// 系统测试运行器
pub struct SystemTestRunner {
    client: Client,
    metrics: SystemMetrics,
}

impl SystemTestRunner {
    /// 创建新的测试运行器
    pub fn new(manager_addr: String, ads_mode: AdsMode) -> Self {
        Self {
            client: Client::new(manager_addr, ads_mode),
            metrics: SystemMetrics::new(),
        }
    }

    /// 运行完整的系统测试
    pub async fn run_test(&mut self, workload_path: &Path) -> Result<()> {
        println!(
            "\n{}",
            "📊 Running system-level benchmark...".bright_cyan().bold()
        );
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // 加载 workload
        let records = self.load_workload(workload_path)?;
        println!("  Loaded {} records from workload", records.len());

        let mut latencies = Vec::new();
        let start_time = Instant::now();

        let total = records.len();
        let progress_interval = total / 20;

        println!("\n▶️  Executing {} file operations...\n", total);

        // 执行每个文件的完整生命周期
        for (idx, record) in records.iter().enumerate() {
            if idx > 0 && idx % progress_interval == 0 {
                let progress = (idx as f64 / total as f64 * 100.0) as usize;
                println!("  [{}%] Processed {}/{} files", progress, idx, total);
            }

            let keywords = &record.keywords;

            if keywords.is_empty() {
                continue;
            }

            // 1. Add 操作
            let add_latency = self.execute_add(&record.fid, keywords).await?;
            latencies.push(add_latency);
            self.metrics.operation_stats.add_count += 1;

            // 2. Query 操作 (查询第一个关键词)
            if let Some(keyword) = keywords.first() {
                let query_latency = self.execute_query(keyword).await?;
                latencies.push(query_latency);
                self.metrics.operation_stats.query_count += 1;
            }

            // 3. Update 操作 (模拟更新部分关键词)
            if keywords.len() >= 2 {
                let update_latency = self
                    .execute_update(
                        &record.fid,
                        &keywords[0..1],
                        &keywords[keywords.len() - 1..],
                    )
                    .await?;
                latencies.push(update_latency);
                self.metrics.operation_stats.update_count += 1;
            }

            // 4. Delete 操作
            let delete_latency = self.execute_delete(&record.fid, keywords).await?;
            latencies.push(delete_latency);
            self.metrics.operation_stats.delete_count += 1;

            self.metrics.success_count += 4; // 4个操作
        }

        println!("  [100%] Processed {}/{} files\n", total, total);

        // 统计指标
        self.metrics.total_duration = start_time.elapsed();
        self.metrics.calculate_percentiles(&mut latencies);
        self.metrics.calculate_throughput();

        println!("{}", "✅ System test completed!".bright_green().bold());

        Ok(())
    }

    /// 加载 workload 数据
    fn load_workload(&self, path: &Path) -> Result<Vec<WorkloadRecord>> {
        let mut reader = ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_path(path)
            .context("Failed to open workload file")?;

        let mut records = Vec::new();
        for result in reader.records() {
            let record = result.context("Failed to read CSV record")?;

            if record.len() < 2 {
                continue; // 跳过无效记录
            }

            let fid = record[0].to_string();
            let keywords: Vec<String> = record
                .iter()
                .skip(1) // 跳过 fid
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();

            if !keywords.is_empty() {
                records.push(WorkloadRecord { fid, keywords });
            }
        }

        Ok(records)
    }

    /// 执行 Add 操作
    async fn execute_add(&mut self, fid: &str, keywords: &[String]) -> Result<f64> {
        let start = Instant::now();
        self.client
            .put_file(fid.to_string(), keywords.to_vec())
            .await
            .map_err(|e| anyhow::anyhow!("Add operation failed: {}", e))?;
        Ok(start.elapsed().as_secs_f64() * 1000.0)
    }

    /// 执行 Query 操作
    async fn execute_query(&mut self, keyword: &str) -> Result<f64> {
        let start = Instant::now();
        self.client
            .query_by_keyword(keyword.to_string())
            .await
            .map_err(|e| anyhow::anyhow!("Query operation failed: {}", e))?;
        Ok(start.elapsed().as_secs_f64() * 1000.0)
    }

    /// 执行 Update 操作
    async fn execute_update(
        &mut self,
        fid: &str,
        remove_keywords: &[String],
        add_keywords: &[String],
    ) -> Result<f64> {
        let start = Instant::now();
        self.client
            .update_file(
                fid.to_string(),
                remove_keywords.to_vec(),
                add_keywords.to_vec(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Update operation failed: {}", e))?;
        Ok(start.elapsed().as_secs_f64() * 1000.0)
    }

    /// 执行 Delete 操作
    async fn execute_delete(&mut self, fid: &str, keywords: &[String]) -> Result<f64> {
        let start = Instant::now();
        self.client
            .delete_file(fid.to_string(), keywords.to_vec())
            .await
            .map_err(|e| anyhow::anyhow!("Delete operation failed: {}", e))?;
        Ok(start.elapsed().as_secs_f64() * 1000.0)
    }

    /// 打印测试摘要
    pub fn print_summary(&self) {
        println!("\n{}", "═".repeat(80).bright_green());
        println!("{}", "  SYSTEM BENCHMARK SUMMARY".bright_green().bold());
        println!("{}", "═".repeat(80).bright_green());

        println!("\n📈 Operation Statistics:");
        println!(
            "  Add:    {} operations",
            self.metrics.operation_stats.add_count
        );
        println!(
            "  Query:  {} operations",
            self.metrics.operation_stats.query_count
        );
        println!(
            "  Update: {} operations",
            self.metrics.operation_stats.update_count
        );
        println!(
            "  Delete: {} operations",
            self.metrics.operation_stats.delete_count
        );

        let total_ops = self.metrics.success_count + self.metrics.failure_count;
        println!("\n✅ Success/Failure:");
        println!("  Total:   {} operations", total_ops);
        println!("  Success: {}", self.metrics.success_count);
        println!("  Failure: {}", self.metrics.failure_count);
        println!("  Success Rate: {:.2}%", self.metrics.success_rate());

        println!("\n⏱️  End-to-End Latency:");
        let lat = &self.metrics.end_to_end_latency;
        println!("  Min:    {:.3} ms", lat.min_ms);
        println!("  Avg:    {:.3} ms", lat.avg_ms);
        println!("  Max:    {:.3} ms", lat.max_ms);
        println!("  P50:    {:.3} ms", lat.p50_ms);
        println!("  P95:    {:.3} ms", lat.p95_ms);
        println!("  P99:    {:.3} ms", lat.p99_ms);

        println!("\n🚀 Throughput:");
        println!(
            "  Total Duration: {:.2}s",
            self.metrics.total_duration.as_secs_f64()
        );
        println!("  Throughput: {:.2} ops/sec", self.metrics.total_throughput);

        println!("\n{}", "═".repeat(80).bright_green());
    }

    /// 获取指标
    pub fn metrics(&self) -> &SystemMetrics {
        &self.metrics
    }
}
