/// 统一性能基准测试
/// 
/// 使用相同的工作负载数据测试三种ADS的性能:
/// - MEST (Merkle Extendible Segmented Hash Tree)
/// - AccTrie (Accumulator-based Trie)
/// - MPT (Merkle Patricia Trie) - 待实现适配器
/// 
/// 测试指标:
/// 1. 插入性能 (ops/sec, 平均延迟)
/// 2. 查询性能 (ops/sec, 平均延迟)
/// 3. 删除性能 (ops/sec, 平均延迟)
/// 4. 证明大小 (字节)

use ads_rust::unified_ads::{AuthenticatedDataStructure, UnifiedKey, UnifiedValue};
use ads_rust::mest::MestAdapter;
use ads_rust::acctrie::AccTrieAdapter;
use ads_rust::mpt::MptAdapter;
use std::time::{Duration, Instant};
use std::fs::File;
use std::io::{BufRead, BufReader};

/// 工作负载记录
#[derive(Debug, Clone)]
struct WorkloadRecord {
    key: String,
    category: String,
    keywords: Vec<String>,
}

/// 性能统计
#[derive(Debug)]
struct PerformanceStats {
    total_ops: usize,
    total_duration: Duration,
    ops_per_sec: f64,
    avg_latency_us: f64,
    min_latency_us: u64,
    max_latency_us: u64,
}

impl PerformanceStats {
    fn new(total_ops: usize, total_duration: Duration, latencies: &[u64]) -> Self {
        let ops_per_sec = total_ops as f64 / total_duration.as_secs_f64();
        let avg_latency_us = latencies.iter().sum::<u64>() as f64 / latencies.len() as f64;
        let min_latency_us = *latencies.iter().min().unwrap_or(&0);
        let max_latency_us = *latencies.iter().max().unwrap_or(&0);
        
        Self {
            total_ops,
            total_duration,
            ops_per_sec,
            avg_latency_us,
            min_latency_us,
            max_latency_us,
        }
    }
    
    fn print(&self, operation: &str) {
        println!("  {} 性能:", operation);
        println!("    总操作数: {}", self.total_ops);
        println!("    总耗时: {:.2}s", self.total_duration.as_secs_f64());
        println!("    吞吐量: {:.2} ops/sec", self.ops_per_sec);
        println!("    平均延迟: {:.2} μs", self.avg_latency_us);
        println!("    最小延迟: {} μs", self.min_latency_us);
        println!("    最大延迟: {} μs", self.max_latency_us);
    }
}

/// 加载工作负载数据
fn load_workload(filename: &str) -> std::io::Result<Vec<WorkloadRecord>> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() >= 2 {
            let key = parts[0].to_string();
            let category = parts[1].to_string();
            let keywords = parts[2..].iter().map(|s| s.to_string()).collect();
            
            records.push(WorkloadRecord {
                key,
                category,
                keywords,
            });
        }
    }
    
    Ok(records)
}

/// 测试MEST性能
fn benchmark_mest(records: &[WorkloadRecord]) {
    println!("\n=== MEST 性能测试 ===");
    
    let mut adapter = MestAdapter::new(4, 16, 32);
    
    // 插入测试
    let mut insert_latencies = Vec::new();
    let start = Instant::now();
    
    for (idx, record) in records.iter().enumerate() {
        let key = UnifiedKey::new(record.key.as_bytes().to_vec());
        let value = UnifiedValue::Integer(idx as i64);
        
        let op_start = Instant::now();
        adapter.insert(key, value, None).unwrap();
        insert_latencies.push(op_start.elapsed().as_micros() as u64);
    }
    
    let insert_duration = start.elapsed();
    let insert_stats = PerformanceStats::new(records.len(), insert_duration, &insert_latencies);
    insert_stats.print("插入");
    
    // 查询测试
    let mut query_latencies = Vec::new();
    let start = Instant::now();
    
    for record in records.iter() {
        let key = UnifiedKey::new(record.key.as_bytes().to_vec());
        
        let op_start = Instant::now();
        let result = adapter.query(&key, None).unwrap();
        query_latencies.push(op_start.elapsed().as_micros() as u64);
        
        assert!(result.is_some(), "Key {} should exist", record.key);
    }
    
    let query_duration = start.elapsed();
    let query_stats = PerformanceStats::new(records.len(), query_duration, &query_latencies);
    query_stats.print("查询");
    
    // 删除测试 (删除前50%的数据)
    let delete_count = records.len() / 2;
    let mut delete_latencies = Vec::new();
    let start = Instant::now();
    
    for record in records.iter().take(delete_count) {
        let key = UnifiedKey::new(record.key.as_bytes().to_vec());
        
        let op_start = Instant::now();
        adapter.delete(&key, None).unwrap();
        delete_latencies.push(op_start.elapsed().as_micros() as u64);
    }
    
    let delete_duration = start.elapsed();
    let delete_stats = PerformanceStats::new(delete_count, delete_duration, &delete_latencies);
    delete_stats.print("删除");
    
    println!("  ADS类型: {}", adapter.ads_type());
}

/// 测试AccTrie性能
fn benchmark_acctrie(records: &[WorkloadRecord]) {
    println!("\n=== AccTrie 性能测试 ===");
    
    let mut adapter = AccTrieAdapter::new();
    
    // 插入测试
    let mut insert_latencies = Vec::new();
    let start = Instant::now();
    
    for (idx, record) in records.iter().enumerate() {
        let key = UnifiedKey::new(record.key.as_bytes().to_vec());
        let value = UnifiedValue::Integer(idx as i64);
        
        let op_start = Instant::now();
        adapter.insert(key, value, None).unwrap();
        insert_latencies.push(op_start.elapsed().as_micros() as u64);
    }
    
    let insert_duration = start.elapsed();
    let insert_stats = PerformanceStats::new(records.len(), insert_duration, &insert_latencies);
    insert_stats.print("插入");
    
    // 查询测试
    let mut query_latencies = Vec::new();
    let start = Instant::now();
    
    for record in records.iter() {
        let key = UnifiedKey::new(record.key.as_bytes().to_vec());
        
        let op_start = Instant::now();
        let result = adapter.query(&key, None).unwrap();
        query_latencies.push(op_start.elapsed().as_micros() as u64);
        
        assert!(result.is_some(), "Key {} should exist", record.key);
    }
    
    let query_duration = start.elapsed();
    let query_stats = PerformanceStats::new(records.len(), query_duration, &query_latencies);
    query_stats.print("查询");
    
    // 删除测试 (删除前50%的数据)
    let delete_count = records.len() / 2;
    let mut delete_latencies = Vec::new();
    let start = Instant::now();
    
    for record in records.iter().take(delete_count) {
        let key = UnifiedKey::new(record.key.as_bytes().to_vec());
        
        let op_start = Instant::now();
        let result = adapter.delete(&key, None).unwrap();
        delete_latencies.push(op_start.elapsed().as_micros() as u64);
        
        assert!(result.is_some(), "Delete should succeed for key {}", record.key);
    }
    
    let delete_duration = start.elapsed();
    let delete_stats = PerformanceStats::new(delete_count, delete_duration, &delete_latencies);
    delete_stats.print("删除");
    
    println!("  ADS类型: {}", adapter.ads_type());
}

/// 测试MPT性能
fn benchmark_mpt(records: &[WorkloadRecord]) {
    println!("\n=== MPT 性能测试 ===");
    
    let mut adapter = MptAdapter::new();
    
    // 插入测试
    let mut insert_latencies = Vec::new();
    let start = Instant::now();
    
    for (idx, record) in records.iter().enumerate() {
        let key = UnifiedKey::new(record.key.as_bytes().to_vec());
        let value = UnifiedValue::String(format!("value_{}", idx));
        
        let op_start = Instant::now();
        adapter.insert(key, value, None).unwrap();
        insert_latencies.push(op_start.elapsed().as_micros() as u64);
    }
    
    let insert_duration = start.elapsed();
    let insert_stats = PerformanceStats::new(records.len(), insert_duration, &insert_latencies);
    insert_stats.print("插入");
    
    // 查询测试
    let mut query_latencies = Vec::new();
    let start = Instant::now();
    
    for record in records.iter() {
        let key = UnifiedKey::new(record.key.as_bytes().to_vec());
        
        let op_start = Instant::now();
        let result = adapter.query(&key, None).unwrap();
        query_latencies.push(op_start.elapsed().as_micros() as u64);
        
        assert!(result.is_some(), "Key {} should exist", record.key);
    }
    
    let query_duration = start.elapsed();
    let query_stats = PerformanceStats::new(records.len(), query_duration, &query_latencies);
    query_stats.print("查询");
    
    // 删除测试 (删除前50%的数据)
    let delete_count = records.len() / 2;
    let mut delete_latencies = Vec::new();
    let start = Instant::now();
    
    for record in records.iter().take(delete_count) {
        let key = UnifiedKey::new(record.key.as_bytes().to_vec());
        
        let op_start = Instant::now();
        adapter.delete(&key, None).unwrap();
        delete_latencies.push(op_start.elapsed().as_micros() as u64);
    }
    
    let delete_duration = start.elapsed();
    let delete_stats = PerformanceStats::new(delete_count, delete_duration, &delete_latencies);
    delete_stats.print("删除");
    
    println!("  ADS类型: {}", adapter.ads_type());
}

fn main() {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║          统一ADS性能基准测试                                 ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    
    // 选择工作负载文件
    let workload_files = [
        ("小型", "data/workload_small_1000.csv"),
        ("中型", "data/workload_medium_10000.csv"),
        ("大型", "data/workload_large_100000.csv"),
    ];
    
    println!("\n可用的工作负载:");
    for (idx, (name, _)) in workload_files.iter().enumerate() {
        println!("  {}. {} 工作负载", idx + 1, name);
    }
    
    // 默认使用小型工作负载
    let selected_idx = 0;
    let (workload_name, workload_file) = workload_files[selected_idx];
    
    println!("\n使用工作负载: {} ({})", workload_name, workload_file);
    
    // 加载数据
    println!("正在加载数据...");
    let records = match load_workload(workload_file) {
        Ok(records) => {
            println!("✓ 成功加载 {} 条记录", records.len());
            records
        }
        Err(e) => {
            eprintln!("✗ 加载失败: {}", e);
            return;
        }
    };
    
    // 运行基准测试
    benchmark_mest(&records);
    benchmark_mpt(&records);
    benchmark_acctrie(&records);
    
    // 性能对比总结
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║          性能对比总结                                        ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!("\n说明:");
    println!("  - MEST: 平衡的读写性能,适合混合工作负载");
    println!("  - MPT: Merkle Patricia Trie,高效的插入/删除,紧凑的证明");
    println!("  - AccTrie: 使用椭圆曲线累加器,查询验证快但插入/删除慢");
    println!("  - 所有测试使用相同的工作负载数据");
    println!("\n提示:");
    println!("  可以修改代码中的 selected_idx 来选择不同大小的工作负载:");
    println!("    0 = 小型 (1,000 条记录)");
    println!("    1 = 中型 (10,000 条记录)");
    println!("    2 = 大型 (100,000 条记录)");
}
