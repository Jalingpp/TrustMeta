/// 系统集成测试
/// 
/// 根据需求文档测试四个核心功能:
/// A. 文件上传 (Add)
/// B. 文件查询 (Query) 
/// C. 文件删除 (Delete)
/// D. 文件更新 (Update)
/// 
/// 使用 data 目录中的真实工作负载数据

use common::rpc::{
    manager_service_client::ManagerServiceClient,
    AddRequest, QueryRequest, DeleteRequest, UpdateRequest,
};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::time::Instant;
use tokio::time::{sleep, Duration};

#[derive(Clone, Debug)]
struct Record {
    fid: String,
    category: String,
    keywords: Vec<String>,
}

/// 加载数据集
fn load_dataset(filepath: &str) -> Result<Vec<Record>, Box<dyn std::error::Error>> {
    println!("📂 加载数据集: {}", filepath);
    let file = File::open(filepath)?;
    let reader = BufReader::new(file);

    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let parts: Vec<&str> = line.split(',').collect();

        if parts.len() < 2 {
            continue;
        }

        let fid = parts[0].to_string();
        let category = parts[1].to_string();
        let keywords: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();

        records.push(Record {
            fid,
            category,
            keywords,
        });
    }

    println!("✅ 成功加载 {} 条记录", records.len());
    Ok(records)
}

/// 创建客户端连接
async fn create_client(
    manager_addr: String,
) -> Result<ManagerServiceClient<tonic::transport::Channel>, Box<dyn std::error::Error>> {
    println!("🔌 连接到 Manager: {}", manager_addr);
    let mut retries = 5;
    loop {
        match ManagerServiceClient::connect(manager_addr.clone()).await {
            Ok(client) => {
                println!("✅ 成功连接到 Manager");
                return Ok(client);
            }
            Err(e) if retries > 0 => {
                retries -= 1;
                println!("⚠️  连接失败,剩余重试次数: {}", retries);
                sleep(Duration::from_millis(500)).await;
                if retries == 0 {
                    return Err(Box::new(e));
                }
            }
            Err(e) => return Err(Box::new(e)),
        }
    }
}

/// A. 测试文件上传 (Add)
async fn test_add(
    client: &mut ManagerServiceClient<tonic::transport::Channel>,
    records: &[Record],
    batch_size: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║  测试 A: 文件上传 (Add)                                     ║");
    println!("╚════════════════════════════════════════════════════════════╝");

    let test_records = &records[..batch_size.min(records.len())];
    println!("📤 上传 {} 条记录...", test_records.len());

    let start = Instant::now();
    let mut success_count = 0;
    let mut fail_count = 0;

    for (idx, record) in test_records.iter().enumerate() {
        let request = tonic::Request::new(AddRequest {
            fid: record.fid.clone(),
            keywords: record.keywords.clone(),
        });

        match client.add(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                if resp.success {
                    success_count += 1;
                    if (idx + 1) % 100 == 0 {
                        println!("  ✓ 已上传 {}/{} 条记录", idx + 1, test_records.len());
                    }
                } else {
                    fail_count += 1;
                    eprintln!("  ✗ 上传失败 (fid={}): {}", record.fid, resp.message);
                }
            }
            Err(e) => {
                fail_count += 1;
                eprintln!("  ✗ RPC 错误 (fid={}): {:?}", record.fid, e);
            }
        }
    }

    let duration = start.elapsed();
    println!("\n📊 上传统计:");
    println!("  ✅ 成功: {} 条", success_count);
    println!("  ❌ 失败: {} 条", fail_count);
    println!("  ⏱️  总耗时: {:.2}s", duration.as_secs_f64());
    println!("  🚀 吞吐量: {:.2} ops/sec", success_count as f64 / duration.as_secs_f64());

    Ok(())
}

/// B. 测试文件查询 (Query)
async fn test_query(
    client: &mut ManagerServiceClient<tonic::transport::Channel>,
    records: &[Record],
    batch_size: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║  测试 B: 文件查询 (Query)                                   ║");
    println!("╚════════════════════════════════════════════════════════════╝");

    let test_records = &records[..batch_size.min(records.len())];
    println!("🔍 查询 {} 个关键词...", test_records.len());

    let start = Instant::now();
    let mut success_count = 0;
    let mut fail_count = 0;
    let mut found_count = 0;
    let mut not_found_count = 0;

    for (idx, record) in test_records.iter().enumerate() {
        // 查询第一个关键词
        if record.keywords.is_empty() {
            continue;
        }

        let keyword = &record.keywords[0];
        let request = tonic::Request::new(QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(keyword.clone())),
        });

        match client.query(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                success_count += 1;

                if !resp.fids.is_empty() {
                    found_count += 1;
                    if (idx + 1) % 100 == 0 {
                        println!("  ✓ 已查询 {}/{} 条,找到 {} 个结果", 
                            idx + 1, test_records.len(), resp.fids.len());
                    }
                } else {
                    not_found_count += 1;
                }
            }
            Err(e) => {
                fail_count += 1;
                eprintln!("  ✗ 查询错误 (keyword={}): {:?}", keyword, e);
            }
        }
    }

    let duration = start.elapsed();
    println!("\n📊 查询统计:");
    println!("  ✅ 成功: {} 次", success_count);
    println!("  🎯 找到结果: {} 次", found_count);
    println!("  🔍 未找到: {} 次", not_found_count);
    println!("  ❌ 失败: {} 次", fail_count);
    println!("  ⏱️  总耗时: {:.2}s", duration.as_secs_f64());
    println!("  🚀 吞吐量: {:.2} ops/sec", success_count as f64 / duration.as_secs_f64());

    Ok(())
}

/// C. 测试文件删除 (Delete)
async fn test_delete(
    client: &mut ManagerServiceClient<tonic::transport::Channel>,
    records: &[Record],
    delete_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║  测试 C: 文件删除 (Delete)                                  ║");
    println!("╚════════════════════════════════════════════════════════════╝");

    let delete_records = &records[..delete_count.min(records.len())];
    println!("🗑️  删除 {} 条记录...", delete_records.len());

    let start = Instant::now();
    let mut success_count = 0;
    let mut fail_count = 0;

    for (idx, record) in delete_records.iter().enumerate() {
        let request = tonic::Request::new(DeleteRequest {
            fid: record.fid.clone(),
            keywords: record.keywords.clone(),
        });

        match client.delete(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                if resp.success {
                    success_count += 1;
                    if (idx + 1) % 50 == 0 {
                        println!("  ✓ 已删除 {}/{} 条记录", idx + 1, delete_records.len());
                    }
                } else {
                    fail_count += 1;
                    eprintln!("  ✗ 删除失败 (fid={}): {}", record.fid, resp.message);
                }
            }
            Err(e) => {
                fail_count += 1;
                eprintln!("  ✗ RPC 错误 (fid={}): {:?}", record.fid, e);
            }
        }
    }

    let duration = start.elapsed();
    println!("\n📊 删除统计:");
    println!("  ✅ 成功: {} 条", success_count);
    println!("  ❌ 失败: {} 条", fail_count);
    println!("  ⏱️  总耗时: {:.2}s", duration.as_secs_f64());
    println!("  🚀 吞吐量: {:.2} ops/sec", success_count as f64 / duration.as_secs_f64());

    // 验证删除
    println!("\n🔍 验证删除结果...");
    let mut verified_deleted = 0;
    for record in delete_records.iter().take(10) {
        if record.keywords.is_empty() {
            continue;
        }
        let request = tonic::Request::new(QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(record.keywords[0].clone())),
        });

        if let Ok(response) = client.query(request).await {
            let resp = response.into_inner();
            if !resp.fids.contains(&record.fid) {
                verified_deleted += 1;
            }
        }
    }
    println!("  ✓ 验证删除: {}/10 条确认已删除", verified_deleted);

    Ok(())
}

/// D. 测试文件更新 (Update)
async fn test_update(
    client: &mut ManagerServiceClient<tonic::transport::Channel>,
    records: &[Record],
    update_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║  测试 D: 文件更新 (Update)                                  ║");
    println!("╚════════════════════════════════════════════════════════════╝");

    let update_records = &records[..update_count.min(records.len())];
    println!("🔄 更新 {} 条记录...", update_records.len());

    let start = Instant::now();
    let mut success_count = 0;
    let mut fail_count = 0;

    for (idx, record) in update_records.iter().enumerate() {
        // 生成新的关键词集合 (添加 "_updated" 后缀)
        let new_keywords: Vec<String> = record
            .keywords
            .iter()
            .map(|k| format!("{}_updated", k))
            .collect();

        let request = tonic::Request::new(UpdateRequest {
            fid: record.fid.clone(),
            old_keywords: record.keywords.clone(),
            new_keywords: new_keywords.clone(),
        });

        match client.update(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                if resp.success {
                    success_count += 1;
                    if (idx + 1) % 50 == 0 {
                        println!("  ✓ 已更新 {}/{} 条记录", idx + 1, update_records.len());
                    }
                } else {
                    fail_count += 1;
                    eprintln!("  ✗ 更新失败 (fid={}): {}", record.fid, resp.message);
                }
            }
            Err(e) => {
                fail_count += 1;
                eprintln!("  ✗ RPC 错误 (fid={}): {:?}", record.fid, e);
            }
        }
    }

    let duration = start.elapsed();
    println!("\n📊 更新统计:");
    println!("  ✅ 成功: {} 条", success_count);
    println!("  ❌ 失败: {} 条", fail_count);
    println!("  ⏱️  总耗时: {:.2}s", duration.as_secs_f64());
    println!("  🚀 吞吐量: {:.2} ops/sec", success_count as f64 / duration.as_secs_f64());

    // 验证更新
    println!("\n🔍 验证更新结果...");
    let mut verified_updated = 0;
    for record in update_records.iter().take(10) {
        if record.keywords.is_empty() {
            continue;
        }
        let new_keyword = format!("{}_updated", record.keywords[0]);
        let request = tonic::Request::new(QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(new_keyword)),
        });

        if let Ok(response) = client.query(request).await {
            let resp = response.into_inner();
            if resp.fids.contains(&record.fid) {
                verified_updated += 1;
            }
        }
    }
    println!("  ✓ 验证更新: {}/10 条确认已更新", verified_updated);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║          分布式存储系统集成测试                               ║");
    println!("║          System Integration Test                          ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    // 配置
    let manager_addr = std::env::var("MANAGER_ADDR")
        .unwrap_or_else(|_| "http://127.0.0.1:50051".to_string());
    let dataset_path = std::env::var("DATASET_PATH")
        .unwrap_or_else(|_| "data/workload_small_1000.csv".to_string());

    println!("⚙️  配置:");
    println!("  Manager 地址: {}", manager_addr);
    println!("  数据集路径: {}", dataset_path);

    // 加载数据集
    let records = load_dataset(&dataset_path)?;
    println!("");

    // 创建客户端
    let mut client = create_client(manager_addr).await?;
    println!("");

    // 测试参数
    let add_count = 100;      // 上传100条
    let query_count = 100;    // 查询100次
    let delete_count = 50;    // 删除50条
    let update_count = 30;    // 更新30条

    println!("📋 测试计划:");
    println!("  - 上传记录数: {}", add_count);
    println!("  - 查询次数: {}", query_count);
    println!("  - 删除记录数: {}", delete_count);
    println!("  - 更新记录数: {}", update_count);

    let total_start = Instant::now();

    // 执行测试
    test_add(&mut client, &records, add_count).await?;
    test_query(&mut client, &records, query_count).await?;
    test_delete(&mut client, &records, delete_count).await?;
    test_update(&mut client, &records, update_count).await?;

    let total_duration = total_start.elapsed();

    // 总结
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║          测试总结                                            ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!("\n✅ 所有测试完成!");
    println!("⏱️  总耗时: {:.2}s", total_duration.as_secs_f64());
    println!("\n四个核心功能测试:");
    println!("  ✓ A. 文件上传 (Add)     - 完成");
    println!("  ✓ B. 文件查询 (Query)   - 完成");
    println!("  ✓ C. 文件删除 (Delete)  - 完成");
    println!("  ✓ D. 文件更新 (Update)  - 完成");

    Ok(())
}
