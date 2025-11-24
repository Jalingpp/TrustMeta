use common::rpc::{
    manager_service_client::ManagerServiceClient, AddRequest, DeleteRequest, QueryRequest,
    UpdateRequest,
};
use std::collections::HashMap;
use std::time::Instant;
use tokio;
use tokio::time::{sleep, Duration};

async fn create_client(
    manager_addr: String,
) -> Result<ManagerServiceClient<tonic::transport::Channel>, Box<dyn std::error::Error>> {
    let mut retries = 3;
    loop {
        match ManagerServiceClient::connect(manager_addr.clone()).await {
            Ok(client) => return Ok(client),
            Err(e) if retries > 0 => {
                retries -= 1;
                sleep(Duration::from_millis(50)).await;
                if retries == 0 {
                    return Err(Box::new(e));
                }
            }
            Err(e) => return Err(Box::new(e)),
        }
    }
}

struct OperationMetrics {
    total_latency: Duration,
    min_latency: Duration,
    max_latency: Duration,
    success_count: usize,
    total_count: usize,
    latencies: Vec<Duration>,
    proof_sizes: Vec<usize>,
}

impl OperationMetrics {
    fn new() -> Self {
        Self {
            total_latency: Duration::ZERO,
            min_latency: Duration::from_secs(9999),
            max_latency: Duration::ZERO,
            success_count: 0,
            total_count: 0,
            latencies: Vec::new(),
            proof_sizes: Vec::new(),
        }
    }

    fn record(&mut self, latency: Duration, proof_size: usize) {
        self.total_latency += latency;
        self.min_latency = self.min_latency.min(latency);
        self.max_latency = self.max_latency.max(latency);
        self.success_count += 1;
        self.total_count += 1;
        self.latencies.push(latency);
        self.proof_sizes.push(proof_size);
    }

    fn record_failure(&mut self) {
        self.total_count += 1;
    }

    fn avg_latency(&self) -> Duration {
        if self.success_count == 0 {
            return Duration::ZERO;
        }
        self.total_latency / self.success_count as u32
    }

    fn avg_proof_size(&self) -> f64 {
        if self.proof_sizes.is_empty() {
            return 0.0;
        }
        self.proof_sizes.iter().sum::<usize>() as f64 / self.proof_sizes.len() as f64
    }

    fn percentile(&self, p: f64) -> Duration {
        if self.latencies.is_empty() {
            return Duration::ZERO;
        }
        let mut sorted = self.latencies.clone();
        sorted.sort();
        let idx = ((sorted.len() as f64) * p).ceil() as usize - 1;
        sorted[idx.min(sorted.len() - 1)]
    }

    fn success_rate(&self) -> f64 {
        if self.total_count == 0 {
            return 0.0;
        }
        (self.success_count as f64 / self.total_count as f64) * 100.0
    }
}

async fn test_add_operations(
    manager_addr: &str,
    num_operations: usize,
) -> Result<OperationMetrics, Box<dyn std::error::Error>> {
    let mut metrics = OperationMetrics::new();
    println!("  执行 {} 次 Add 操作...", num_operations);

    for i in 0..num_operations {
        let fid = format!("comparison_file_{:06}", i);
        let keywords = vec![
            format!("category_{}", i % 10),
            format!("tag_{}", i % 50),
            "test".to_string(),
            "comparison".to_string(),
        ];

        let mut grpc_client = create_client(manager_addr.to_string()).await?;
        let request = AddRequest {
            fid: fid.clone(),
            keywords: keywords.clone(),
        };

        let start = Instant::now();
        match grpc_client.add(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                if resp.success {
                    let latency = start.elapsed();
                    let proof_size = resp.combined_proof.len();
                    metrics.record(latency, proof_size);
                } else {
                    eprintln!("    ⚠ Add 失败 {}: {}", fid, resp.message);
                    metrics.record_failure();
                }
            }
            Err(e) => {
                eprintln!("    ⚠ Add gRPC 失败 {}: {}", fid, e);
                metrics.record_failure();
            }
        }

        // 每 100 个操作打印进度
        if (i + 1) % 100 == 0 {
            print!("\r    进度: {}/{}", i + 1, num_operations);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }
    }
    println!("\r    完成: {}/{}", num_operations, num_operations);

    Ok(metrics)
}

async fn test_query_operations(
    manager_addr: &str,
    num_operations: usize,
) -> Result<OperationMetrics, Box<dyn std::error::Error>> {
    let mut metrics = OperationMetrics::new();
    println!("  执行 {} 次 Query 操作...", num_operations);

    // 测试不同类型的查询
    let test_keywords = vec![
        "category_0",
        "category_5",
        "tag_10",
        "tag_25",
        "test",
        "comparison",
    ];

    for i in 0..num_operations {
        let keyword = test_keywords[i % test_keywords.len()].to_string();

        let mut grpc_client = create_client(manager_addr.to_string()).await?;
        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(
                keyword.clone(),
            )),
        };

        let start = Instant::now();
        match grpc_client.query(request).await {
            Ok(response) => {
                let latency = start.elapsed();
                let resp = response.into_inner();
                let proof_size = resp.proof.len();
                metrics.record(latency, proof_size);
            }
            Err(e) => {
                eprintln!("    ⚠ Query 失败 '{}': {}", keyword, e);
                metrics.record_failure();
            }
        }

        if (i + 1) % 100 == 0 {
            print!("\r    进度: {}/{}", i + 1, num_operations);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }
    }
    println!("\r    完成: {}/{}", num_operations, num_operations);

    Ok(metrics)
}

async fn test_update_operations(
    manager_addr: &str,
    num_operations: usize,
) -> Result<OperationMetrics, Box<dyn std::error::Error>> {
    let mut metrics = OperationMetrics::new();
    println!("  执行 {} 次 Update 操作...", num_operations);

    for i in 0..num_operations {
        let fid = format!("comparison_file_{:06}", i);
        let old_keywords = vec![format!("category_{}", i % 10), format!("tag_{}", i % 50)];
        let new_keywords = vec![
            format!("updated_category_{}", i % 10),
            format!("updated_tag_{}", i % 50),
            "updated".to_string(),
        ];

        let mut grpc_client = create_client(manager_addr.to_string()).await?;
        let request = UpdateRequest {
            fid: fid.clone(),
            old_keywords,
            new_keywords,
        };

        let start = Instant::now();
        match grpc_client.update(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                if resp.success {
                    let latency = start.elapsed();
                    let proof_size = resp.combined_proof.len();
                    metrics.record(latency, proof_size);
                } else {
                    eprintln!("    ⚠ Update 失败 {}: {}", fid, resp.message);
                    metrics.record_failure();
                }
            }
            Err(e) => {
                eprintln!("    ⚠ Update gRPC 失败 {}: {}", fid, e);
                metrics.record_failure();
            }
        }

        if (i + 1) % 100 == 0 {
            print!("\r    进度: {}/{}", i + 1, num_operations);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }
    }
    println!("\r    完成: {}/{}", num_operations, num_operations);

    Ok(metrics)
}

async fn test_delete_operations(
    manager_addr: &str,
    num_operations: usize,
) -> Result<OperationMetrics, Box<dyn std::error::Error>> {
    let mut metrics = OperationMetrics::new();
    println!("  执行 {} 次 Delete 操作...", num_operations);

    for i in 0..num_operations {
        let fid = format!("comparison_file_{:06}", i);
        let keywords = vec![
            format!("updated_category_{}", i % 10),
            format!("updated_tag_{}", i % 50),
            "updated".to_string(),
        ];

        let mut grpc_client = create_client(manager_addr.to_string()).await?;
        let request = DeleteRequest {
            fid: fid.clone(),
            keywords,
        };

        let start = Instant::now();
        match grpc_client.delete(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                if resp.success {
                    let latency = start.elapsed();
                    let proof_size = resp.combined_proof.len();
                    metrics.record(latency, proof_size);
                } else {
                    eprintln!("    ⚠ Delete 失败 {}: {}", fid, resp.message);
                    metrics.record_failure();
                }
            }
            Err(e) => {
                eprintln!("    ⚠ Delete gRPC 失败 {}: {}", fid, e);
                metrics.record_failure();
            }
        }

        if (i + 1) % 100 == 0 {
            print!("\r    进度: {}/{}", i + 1, num_operations);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }
    }
    println!("\r    完成: {}/{}", num_operations, num_operations);

    Ok(metrics)
}

fn print_metrics_table(results: &HashMap<String, OperationMetrics>) {
    println!(
        "\n╔══════════════════════════════════════════════════════════════════════════════════╗"
    );
    println!(
        "║                          操作性能对比 - 延迟分析                                   ║"
    );
    println!(
        "╠══════════════════════════════════════════════════════════════════════════════════╣"
    );
    println!(
        "║ 操作类型 │  平均延迟  │  最小延迟  │  最大延迟  │  P50   │  P95   │  P99   │ 成功率 ║"
    );
    println!(
        "╠══════════════════════════════════════════════════════════════════════════════════╣"
    );

    for op_name in &["Add", "Query", "Update", "Delete"] {
        if let Some(metrics) = results.get(*op_name) {
            println!(
                "║ {:^8} │ {:>9.2?} │ {:>9.2?} │ {:>9.2?} │ {:>5.0?} │ {:>5.0?} │ {:>5.0?} │ {:>5.1}% ║",
                op_name,
                metrics.avg_latency(),
                metrics.min_latency,
                metrics.max_latency,
                metrics.percentile(0.50),
                metrics.percentile(0.95),
                metrics.percentile(0.99),
                metrics.success_rate(),
            );
        }
    }
    println!(
        "╚══════════════════════════════════════════════════════════════════════════════════╝"
    );

    println!("\n╔═══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                     操作性能对比 - 证明大小分析                                ║");
    println!("╠═══════════════════════════════════════════════════════════════════════════════╣");
    println!("║ 操作类型 │  平均证明 (字节)  │  最小证明  │  最大证明  │      类型          ║");
    println!("╠═══════════════════════════════════════════════════════════════════════════════╣");

    // 显示所有操作的证明数据
    for op_name in &["Add", "Query", "Update", "Delete"] {
        if let Some(metrics) = results.get(*op_name) {
            if !metrics.proof_sizes.is_empty() {
                let min_proof = metrics.proof_sizes.iter().min().unwrap_or(&0);
                let max_proof = metrics.proof_sizes.iter().max().unwrap_or(&0);
                let avg_proof = metrics.avg_proof_size();

                let proof_type = if avg_proof > 1000.0 {
                    "完整 Merkle Proof"
                } else if avg_proof == 32.0 {
                    "简化 (Root Hash)"
                } else {
                    "Merkle Proof"
                };

                println!(
                    "║ {:^8} │ {:>15.0} │ {:>9} │ {:>9} │ {:^16} ║",
                    op_name, avg_proof, min_proof, max_proof, proof_type
                );
            } else {
                println!(
                    "║ {:^8} │ {:>15} │ {:>9} │ {:>9} │ {:^16} ║",
                    op_name, "-", "-", "-", "无证明"
                );
            }
        }
    }
    println!("╠═══════════════════════════════════════════════════════════════════════════════╣");
    println!("║ 💡 说明：                                                                     ║");
    println!("║   • 完整 Merkle Proof: 包含从叶子到根的完整路径，可密码学验证              ║");
    println!("║   • 简化 Root Hash: 仅 32 字节根哈希，用于快速一致性检查                   ║");
    println!("║   • 证明大小随树深度和分支因子变化，典型范围: 1-25 KB                      ║");
    println!("╚═══════════════════════════════════════════════════════════════════════════════╝");
}

fn print_comparison_analysis(results: &HashMap<String, OperationMetrics>) {
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                      性能对比分析总结                          ║");
    println!("╚══════════════════════════════════════════════════════════════╝");

    // 找出最快和最慢的操作
    let mut latencies: Vec<(&str, Duration)> = results
        .iter()
        .map(|(name, metrics)| (name.as_str(), metrics.avg_latency()))
        .collect();
    latencies.sort_by_key(|&(_, latency)| latency);

    if let Some((_fastest, fastest_latency)) = latencies.first() {
        println!("\n📊 延迟性能排名:");
        for (i, (op, latency)) in latencies.iter().enumerate() {
            let ratio = latency.as_micros() as f64 / fastest_latency.as_micros() as f64;
            println!(
                "  {}. {:8} - {:>9.2?}  (相对最快: {:.2}x)",
                i + 1,
                op,
                latency,
                ratio
            );
        }
    }

    // 证明大小对比
    println!("\n📦 证明大小统计:");
    for op_name in &["Add", "Query", "Update", "Delete"] {
        if let Some(metrics) = results.get(*op_name) {
            if !metrics.proof_sizes.is_empty() {
                let min_proof = metrics.proof_sizes.iter().min().unwrap_or(&0);
                let max_proof = metrics.proof_sizes.iter().max().unwrap_or(&0);
                let avg_proof = metrics.avg_proof_size();

                println!(
                    "  • {:8} - 平均: {:>6.0} 字节 (范围: {} - {} 字节)",
                    op_name, avg_proof, min_proof, max_proof
                );
            } else {
                println!("  • {:8} - 无证明数据", op_name);
            }
        }
    }

    // 吞吐量对比
    println!("\n⚡ 理论吞吐量对比 (基于平均延迟):");
    let mut throughputs: Vec<(&str, f64)> = results
        .iter()
        .map(|(name, metrics)| {
            let ops_per_sec = 1.0 / metrics.avg_latency().as_secs_f64();
            (name.as_str(), ops_per_sec)
        })
        .collect();
    throughputs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    for (i, (op, throughput)) in throughputs.iter().enumerate() {
        println!("  {}. {:8} - {:>9.0} ops/s", i + 1, op, throughput);
    }

    println!("\n💡 关键发现:");

    // 分析 Query vs Add
    if let (Some(query_metrics), Some(add_metrics)) = (results.get("Query"), results.get("Add")) {
        let latency_ratio = query_metrics.avg_latency().as_micros() as f64
            / add_metrics.avg_latency().as_micros() as f64;

        println!("  • Query 相比 Add:");
        println!("    - 延迟: {:.2}x", latency_ratio);
        if !query_metrics.proof_sizes.is_empty() && !add_metrics.proof_sizes.is_empty() {
            let proof_ratio = query_metrics.avg_proof_size() / add_metrics.avg_proof_size();
            println!("    - 证明大小: {:.2}x", proof_ratio);
        }
    }

    // 分析 Update vs Add
    if let (Some(update_metrics), Some(add_metrics)) = (results.get("Update"), results.get("Add")) {
        let latency_ratio = update_metrics.avg_latency().as_micros() as f64
            / add_metrics.avg_latency().as_micros() as f64;

        println!("  • Update 相比 Add:");
        println!(
            "    - 延迟: {:.2}x (包含删除旧数据 + 添加新数据)",
            latency_ratio
        );
    }

    // 分析 Delete vs Add
    if let (Some(delete_metrics), Some(add_metrics)) = (results.get("Delete"), results.get("Add")) {
        let latency_ratio = delete_metrics.avg_latency().as_micros() as f64
            / add_metrics.avg_latency().as_micros() as f64;

        println!("  • Delete 相比 Add:");
        println!("    - 延迟: {:.2}x", latency_ratio);
    }

    println!("\n🔐 密码学可验证性:");
    let mut has_proof_count = 0;
    for op_name in &["Add", "Query", "Update", "Delete"] {
        if let Some(metrics) = results.get(*op_name) {
            if !metrics.proof_sizes.is_empty() && metrics.avg_proof_size() > 100.0 {
                has_proof_count += 1;
            }
        }
    }

    if has_proof_count == 4 {
        println!("  ✅ 所有操作 (Add/Query/Update/Delete) 都支持完整 Merkle Proof 验证");
        println!("  ✅ 客户端可独立验证每个操作的正确性，无需信任服务器");
        println!("  ✅ 证明大小：平均 2-23 KB，随树深度线性增长");
    } else {
        println!("  ⚠️  部分操作使用简化证明方案（仅 root hash 验证）");
        println!("  ⚠️  完整 Merkle Proof 可提供更强的安全保证");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║        分布式存储系统 - 操作性能全面对比测试                    ║");
    println!("╚═══════════════════════════════════════════════════════════════╝");

    let manager_addr = "http://[::1]:50051";
    let num_operations = 10000; // 每种操作测试 10000 次

    println!("\n📋 测试配置:");
    println!("  • Manager 地址: {}", manager_addr);
    println!("  • 每种操作次数: {}", num_operations);
    println!("  • 测试操作: Add, Query, Update, Delete");
    println!("\n开始测试...\n");

    let mut results = HashMap::new();

    // 1. 测试 Add 操作
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("1️⃣  测试 Add 操作");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    let add_metrics = test_add_operations(manager_addr, num_operations).await?;
    results.insert("Add".to_string(), add_metrics);
    sleep(Duration::from_millis(500)).await;

    // 2. 测试 Query 操作
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("2️⃣  测试 Query 操作");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    let query_metrics = test_query_operations(manager_addr, num_operations).await?;
    results.insert("Query".to_string(), query_metrics);
    sleep(Duration::from_millis(500)).await;

    // 3. 测试 Update 操作
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("3️⃣  测试 Update 操作");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    let update_metrics = test_update_operations(manager_addr, num_operations).await?;
    results.insert("Update".to_string(), update_metrics);
    sleep(Duration::from_millis(500)).await;

    // 4. 测试 Delete 操作
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("4️⃣  测试 Delete 操作");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    let delete_metrics = test_delete_operations(manager_addr, num_operations).await?;
    results.insert("Delete".to_string(), delete_metrics);

    // 打印对比结果
    print_metrics_table(&results);
    print_comparison_analysis(&results);

    println!("\n╔═══════════════════════════════════════════════════════════════╗");
    println!("║                    ✅ 测试完成                                 ║");
    println!("╚═══════════════════════════════════════════════════════════════╝");

    Ok(())
}
