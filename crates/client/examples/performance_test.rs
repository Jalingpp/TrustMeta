use common::rpc::{
    manager_service_client::ManagerServiceClient, AddRequest, DeleteRequest, QueryRequest,
    UpdateRequest,
};
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 分布式存储系统性能测试");
    println!("============================================================\n");

    let manager_addr = "http://[::1]:50051";

    // 测试配置
    let batch_sizes = vec![100, 500, 1000, 1500];
    let query_iterations = 1000; // 恢复到1000次迭代

    println!("📊 测试配置:");
    println!("  - Manager 地址: {}", manager_addr);
    println!("  - 批量测试规模: {:?}", batch_sizes);
    println!("  - 查询迭代次数: {}", query_iterations);
    println!();

    // ========================================
    // 测试 1: 批量添加性能
    // ========================================
    println!("测试 1: 批量添加性能");
    println!("------------------------------------------------------------");

    for batch_size in &batch_sizes {
        let start = Instant::now();
        let mut success_count = 0;

        for i in 0..*batch_size {
            let fid = format!("perf_file_{:04}", i);
            let keywords = vec![
                format!("batch_{}", batch_size),
                format!("index_{}", i),
                "performance".to_string(),
                "test".to_string(),
            ];

            let mut grpc_client = create_client(manager_addr.to_string()).await?;
            let request = AddRequest {
                fid: fid.clone(),
                keywords,
            };

            match grpc_client.add(request).await {
                Ok(_) => success_count += 1,
                Err(e) => eprintln!("  ⚠ 添加 {} 失败: {}", fid, e),
            }

            // 添加小延迟避免连接数过多
            if i % 50 == 0 && i > 0 {
                sleep(Duration::from_millis(10)).await;
            }
        }

        let duration = start.elapsed();
        let ops_per_sec = (*batch_size as f64 / duration.as_secs_f64()).round();

        println!(
            "  批量大小: {:4} | 成功: {:4}/{:4} | 耗时: {:8.2?} | 速度: {:6.0} ops/s",
            batch_size, success_count, batch_size, duration, ops_per_sec
        );
    }
    println!();

    // ========================================
    // 测试 2: 单关键词查询性能
    // ========================================
    println!("测试 2: 单关键词查询性能 ({}次查询)", query_iterations);
    println!("------------------------------------------------------------");

    let test_keywords = vec!["performance", "batch_100", "batch_500", "test"];

    for keyword in &test_keywords {
        // 在每个关键词测试之间稍作休息
        sleep(Duration::from_millis(500)).await;

        let mut total_duration = std::time::Duration::ZERO;
        let mut success_count = 0;
        let mut result_count = 0;

        // 复用同一个客户端连接
        let mut grpc_client = create_client(manager_addr.to_string()).await?;

        for i in 0..query_iterations {
            let start = Instant::now();
            let request = QueryRequest {
                query_type: Some(common::rpc::query_request::QueryType::Keyword(
                    keyword.to_string(),
                )),
            };

            match grpc_client.query(request).await {
                Ok(response) => {
                    let resp = response.into_inner();
                    total_duration += start.elapsed();
                    success_count += 1;
                    result_count = resp.fids.len();
                }
                Err(e) => eprintln!("  ⚠ 查询 '{}' 失败: {}", keyword, e),
            }

            // 添加小延迟避免连接数过多
            if i % 50 == 0 && i > 0 {
                sleep(Duration::from_millis(10)).await;
            }
        }

        let avg_latency = total_duration / query_iterations as u32;
        let qps = (query_iterations as f64 / total_duration.as_secs_f64()).round();

        println!(
            "  关键词: {:15} | 成功: {:3}/{:3} | 结果数: {:4} | 平均延迟: {:8.2?} | QPS: {:6.0}",
            keyword, success_count, query_iterations, result_count, avg_latency, qps
        );
    }
    println!();

    // ========================================
    // 测试 3: 布尔查询性能
    // ========================================
    println!("测试 3: 布尔查询性能 ({}次查询)", query_iterations);
    println!("------------------------------------------------------------");

    let boolean_queries = vec![
        ("performance AND test", "简单 AND"),
        ("batch_100 OR batch_500", "简单 OR"),
        ("(performance OR test) AND batch_100", "复杂嵌套"),
    ];

    for (query, description) in &boolean_queries {
        // 在每个查询测试之间稍作休息
        sleep(Duration::from_millis(500)).await;

        let mut total_duration = std::time::Duration::ZERO;
        let mut success_count = 0;
        let mut result_count = 0;

        // 复用同一个客户端连接
        let mut grpc_client = create_client(manager_addr.to_string()).await?;

        for i in 0..query_iterations {
            let start = Instant::now();
            let request = QueryRequest {
                query_type: Some(common::rpc::query_request::QueryType::BooleanFunction(
                    query.to_string(),
                )),
            };

            match grpc_client.query(request).await {
                Ok(response) => {
                    let resp = response.into_inner();
                    total_duration += start.elapsed();
                    success_count += 1;
                    result_count = resp.fids.len();
                }
                Err(e) => eprintln!("  ⚠ 查询 '{}' 失败: {}", query, e),
            }

            // 添加小延迟避免连接数过多
            if i % 50 == 0 && i > 0 {
                sleep(Duration::from_millis(10)).await;
            }
        }

        let avg_latency = total_duration / query_iterations as u32;
        let qps = (query_iterations as f64 / total_duration.as_secs_f64()).round();

        println!(
            "  {:<15} | 成功: {:3}/{:3} | 结果数: {:4} | 平均延迟: {:8.2?} | QPS: {:6.0}",
            description, success_count, query_iterations, result_count, avg_latency, qps
        );
    }
    println!();

    // ========================================
    // 测试 4: 更新操作性能
    // ========================================
    println!("测试 4: 更新操作性能 (500次更新)");
    println!("------------------------------------------------------------");

    let update_count = 500;
    let start = Instant::now();
    let mut success_count = 0;

    for i in 0..update_count {
        let fid = format!("perf_file_{:04}", i);
        let old_keywords = vec![format!("batch_100"), format!("index_{}", i)];
        let new_keywords = vec!["updated".to_string(), format!("modified_{}", i)];

        let mut grpc_client = create_client(manager_addr.to_string()).await?;
        let request = UpdateRequest {
            fid: fid.clone(),
            old_keywords,
            new_keywords,
        };

        match grpc_client.update(request).await {
            Ok(_) => success_count += 1,
            Err(e) => eprintln!("  ⚠ 更新 {} 失败: {}", fid, e),
        }
    }

    let duration = start.elapsed();
    let ops_per_sec = (update_count as f64 / duration.as_secs_f64()).round();

    println!(
        "  成功: {}/{} | 耗时: {:8.2?} | 速度: {:6.0} ops/s",
        success_count, update_count, duration, ops_per_sec
    );
    println!();

    // ========================================
    // 测试 5: 删除操作性能
    // ========================================
    println!("测试 5: 删除操作性能 (500次删除)");
    println!("------------------------------------------------------------");

    let delete_count = 500;
    let start = Instant::now();
    let mut success_count = 0;

    for i in 0..delete_count {
        let fid = format!("perf_file_{:04}", i);
        let keywords = vec!["updated".to_string(), format!("modified_{}", i)];

        let mut grpc_client = create_client(manager_addr.to_string()).await?;
        let request = DeleteRequest {
            fid: fid.clone(),
            keywords,
        };

        match grpc_client.delete(request).await {
            Ok(_) => success_count += 1,
            Err(e) => eprintln!("  ⚠ 删除 {} 失败: {}", fid, e),
        }
    }

    let duration = start.elapsed();
    let ops_per_sec = (delete_count as f64 / duration.as_secs_f64()).round();

    println!(
        "  成功: {}/{} | 耗时: {:8.2?} | 速度: {:6.0} ops/s",
        success_count, delete_count, duration, ops_per_sec
    );
    println!();

    // ========================================
    // 测试 6: 混合负载性能
    // ========================================
    println!("测试 6: 混合负载性能 (Add:Query:Update:Delete = 4:4:1:1)");
    println!("------------------------------------------------------------");

    let mixed_operations = 1000;
    let start = Instant::now();
    let mut add_count = 0;
    let mut query_count = 0;
    let mut update_count = 0;
    let mut delete_count = 0;

    for i in 0..mixed_operations {
        let op_type = i % 10;

        match op_type {
            0..=3 => {
                // Add (40%)
                let fid = format!("mixed_file_{:04}", i);
                let keywords = vec!["mixed".to_string(), "load".to_string()];

                let mut grpc_client = create_client(manager_addr.to_string()).await?;
                let request = AddRequest { fid, keywords };
                if let Ok(_) = grpc_client.add(request).await {
                    add_count += 1;
                }
            }
            4..=7 => {
                // Query (40%)
                let mut grpc_client = create_client(manager_addr.to_string()).await?;
                let request = QueryRequest {
                    query_type: Some(common::rpc::query_request::QueryType::Keyword(
                        "mixed".to_string(),
                    )),
                };
                if let Ok(_) = grpc_client.query(request).await {
                    query_count += 1;
                }
            }
            8 => {
                // Update (10%)
                if i > 0 {
                    let fid = format!("mixed_file_{:04}", i - 1);
                    let old_kw = vec!["mixed".to_string()];
                    let new_kw = vec!["updated_mixed".to_string()];

                    let mut grpc_client = create_client(manager_addr.to_string()).await?;
                    let request = UpdateRequest {
                        fid,
                        old_keywords: old_kw,
                        new_keywords: new_kw,
                    };
                    if let Ok(_) = grpc_client.update(request).await {
                        update_count += 1;
                    }
                }
            }
            9 => {
                // Delete (10%)
                if i > 0 {
                    let fid = format!("mixed_file_{:04}", i - 1);
                    let keywords = vec!["updated_mixed".to_string()];

                    let mut grpc_client = create_client(manager_addr.to_string()).await?;
                    let request = DeleteRequest { fid, keywords };
                    if let Ok(_) = grpc_client.delete(request).await {
                        delete_count += 1;
                    }
                }
            }
            _ => {}
        }
    }

    let duration = start.elapsed();
    let total_ops = add_count + query_count + update_count + delete_count;
    let ops_per_sec = (total_ops as f64 / duration.as_secs_f64()).round();

    println!(
        "  总操作数: {} | 耗时: {:8.2?} | 吞吐量: {:6.0} ops/s",
        total_ops, duration, ops_per_sec
    );
    println!("  详细统计:");
    println!("    - Add:    {:3} 次", add_count);
    println!("    - Query:  {:3} 次", query_count);
    println!("    - Update: {:3} 次", update_count);
    println!("    - Delete: {:3} 次", delete_count);
    println!();

    println!("============================================================");
    println!("✅ 性能测试完成!");
    println!("============================================================");

    Ok(())
}
