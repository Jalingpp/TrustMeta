use common::rpc::{
    manager_service_client::ManagerServiceClient, 
    AddRequest, QueryRequest, UpdateRequest,
};
use std::time::Instant;
use std::collections::HashMap;
use tokio;
use tokio::time::{sleep, Duration};
use rand::Rng;
use rand::seq::SliceRandom;
use std::fs::File;
use std::io::{BufRead, BufReader};

#[derive(Clone, Debug)]
struct Record {
    fid: String,
    keywords: Vec<String>,
}

async fn create_client(manager_addr: String) -> Result<ManagerServiceClient<tonic::transport::Channel>, Box<dyn std::error::Error>> {
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

fn load_dataset(filepath: &str) -> Result<Vec<Record>, Box<dyn std::error::Error>> {
    println!("加载数据集: {}", filepath);
    let file = File::open(filepath)?;
    let reader = BufReader::new(file);
    
    let mut records = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        let parts: Vec<&str> = line.split(',').collect();
        
        if parts.len() < 2 {
            eprintln!("警告: 第 {} 行格式错误,跳过", idx + 1);
            continue;
        }
        
        let fid = parts[0].to_string();
        let keywords: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
        
        records.push(Record { fid, keywords });
    }
    
    println!("加载完成: {} 条记录\n", records.len());
    Ok(records)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("分布式存储系统 - 真实数据 Workload 测试");
    println!("============================================================\n");

    let manager_addr = "http://[::1]:50051";
    
    // 从命令行参数获取数据文件路径
    let args: Vec<String> = std::env::args().collect();
    let data_file = if args.len() > 1 {
        &args[1]
    } else {
        "data/workload_small_1000.csv"
    };

    // 加载数据集
    let dataset = load_dataset(data_file)?;
    
    if dataset.is_empty() {
        eprintln!("错误: 数据集为空");
        return Ok(());
    }

    // Workload 1: 批量插入 (Bulk Insert)
    println!("\nWorkload 1: 批量插入");
    println!("{}", "-".repeat(60));
    run_bulk_insert(manager_addr, &dataset).await?;
    sleep(Duration::from_secs(1)).await;

    // Workload 2: 随机读取 (Random Read)
    println!("\nWorkload 2: 随机关键词查询");
    println!("{}", "-".repeat(60));
    run_random_read(manager_addr, &dataset).await?;
    sleep(Duration::from_secs(1)).await;

    // Workload 3: 类别扫描 (Category Scan)
    println!("\nWorkload 3: 类别扫描");
    println!("{}", "-".repeat(60));
    run_category_scan(manager_addr, &dataset).await?;
    sleep(Duration::from_secs(1)).await;

    // Workload 4: 热点访问 (Hotspot Access - 80/20)
    println!("\nWorkload 4: 热点访问 (80/20 规则)");
    println!("{}", "-".repeat(60));
    run_hotspot_access(manager_addr, &dataset).await?;
    sleep(Duration::from_secs(1)).await;

    // Workload 5: 混合负载 (Mixed Workload)
    println!("\nWorkload 5: 混合负载 (70% 读, 30% 写)");
    println!("{}", "-".repeat(60));
    run_mixed_workload(manager_addr, &dataset).await?;
    sleep(Duration::from_secs(1)).await;

    // Workload 6: 复杂布尔查询 (Complex Boolean Queries)
    println!("\nWorkload 6: 复杂布尔查询");
    println!("{}", "-".repeat(60));
    run_complex_queries(manager_addr, &dataset).await?;
    sleep(Duration::from_secs(1)).await;

    // Workload 7: 更新负载 (Update Workload)
    println!("\nWorkload 7: 更新负载");
    println!("{}", "-".repeat(60));
    run_update_workload(manager_addr, &dataset).await?;

    println!("\n============================================================");
    println!("所有 Workload 测试完成");
    println!("============================================================");

    Ok(())
}

// Workload 1: 批量插入
async fn run_bulk_insert(manager_addr: &str, dataset: &[Record]) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let mut success = 0;
    let mut failed = 0;
    let batch_size = 30; // 适中的批次大小
    let batch_delay_ms = 50; // 减少批次间延迟
    let total = dataset.len();
    
    // 创建一个客户端连接复用
    let mut client = create_client(manager_addr.to_string()).await?;
    
    for (idx, record) in dataset.iter().enumerate() {
        let request = AddRequest {
            fid: record.fid.clone(),
            keywords: record.keywords.clone(),
        };
        
        match client.add(request).await {
            Ok(_) => success += 1,
            Err(e) => {
                failed += 1;
                // 只打印前几个错误
                if failed <= 3 {
                    eprintln!("    Insert error: {}", e);
                }
                // 连接出错时重试2次
                if e.code() == tonic::Code::Unavailable || e.code() == tonic::Code::Cancelled {
                    let mut retries = 0;
                    while retries < 2 {
                        sleep(Duration::from_millis(100 * (retries + 1))).await;
                        client = create_client(manager_addr.to_string()).await?;
                        
                        let retry_request = AddRequest {
                            fid: record.fid.clone(),
                            keywords: record.keywords.clone(),
                        };
                        
                        match client.add(retry_request).await {
                            Ok(_) => {
                                success += 1;
                                failed -= 1;
                                break;
                            }
                            Err(_) => {
                                retries += 1;
                            }
                        }
                    }
                }
            }
        }
        
        // 每批次增加延迟
        if (idx + 1) % batch_size == 0 {
            sleep(Duration::from_millis(batch_delay_ms)).await;
            if (idx + 1) % 500 == 0 {
                println!("    进度: {}/{} ({:.1}%)", idx + 1, total, (idx + 1) as f64 / total as f64 * 100.0);
            }
        }
    }
    
    let duration = start.elapsed();
    let throughput = (success as f64 / duration.as_secs_f64()).round();
    
    println!("  总记录: {} | 成功: {} | 失败: {} | 耗时: {:?} | 吞吐量: {} ops/s",
        dataset.len(), success, failed, duration, throughput);
    
    Ok(())
}

// Workload 2: 随机关键词查询
async fn run_random_read(manager_addr: &str, dataset: &[Record]) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = rand::thread_rng();
    
    // 收集所有关键词
    let mut all_keywords = Vec::new();
    for record in dataset {
        all_keywords.extend(record.keywords.clone());
    }
    all_keywords.sort();
    all_keywords.dedup();
    
    println!("  唯一关键词数: {}", all_keywords.len());
    
    let num_queries = 500.min(all_keywords.len());
    let start = Instant::now();
    let mut total_results = 0;
    let mut client = create_client(manager_addr.to_string()).await?;
    
    for i in 0..num_queries {
        let keyword = all_keywords.choose(&mut rng).unwrap();
        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(keyword.clone())),
        };
        
        match client.query(request).await {
            Ok(response) => {
                total_results += response.into_inner().fids.len();
            }
            Err(e) => {
                if e.code() == tonic::Code::Unavailable {
                    sleep(Duration::from_millis(50)).await;
                    client = create_client(manager_addr.to_string()).await?;
                }
            }
        }
        
        if i % 50 == 0 && i > 0 {
            sleep(Duration::from_millis(20)).await;
        }
    }
    
    let duration = start.elapsed();
    let qps = (num_queries as f64 / duration.as_secs_f64()).round();
    let avg_results = total_results / num_queries;
    
    println!("  查询数: {} | 平均结果数: {} | 耗时: {:?} | QPS: {}",
        num_queries, avg_results, duration, qps);
    
    Ok(())
}

// Workload 3: 类别扫描
async fn run_category_scan(manager_addr: &str, dataset: &[Record]) -> Result<(), Box<dyn std::error::Error>> {
    // 找出所有类别(通常是第一个关键词)
    let mut categories: Vec<String> = dataset.iter()
        .filter_map(|r| r.keywords.first().cloned())
        .collect();
    categories.sort();
    categories.dedup();
    
    println!("  类别数: {}", categories.len());
    
    let start = Instant::now();
    let mut total_results = 0;
    
    for category in &categories {
        let mut client = create_client(manager_addr.to_string()).await?;
        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(category.clone())),
        };
        
        match client.query(request).await {
            Ok(response) => {
                let count = response.into_inner().fids.len();
                total_results += count;
                println!("    {} → {} 条结果", category, count);
            }
            Err(_) => {}
        }
        
        sleep(Duration::from_millis(20)).await;
    }
    
    let duration = start.elapsed();
    let avg_results = if categories.len() > 0 { total_results / categories.len() } else { 0 };
    
    println!("  总扫描: {} 个类别 | 平均每类: {} 条 | 耗时: {:?}",
        categories.len(), avg_results, duration);
    
    Ok(())
}

// Workload 4: 热点访问 (80/20 规则)
async fn run_hotspot_access(manager_addr: &str, dataset: &[Record]) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = rand::thread_rng();
    
    // 收集所有关键词并按频率排序
    let mut keyword_freq: HashMap<String, usize> = HashMap::new();
    for record in dataset {
        for kw in &record.keywords {
            *keyword_freq.entry(kw.clone()).or_insert(0) += 1;
        }
    }
    
    let mut sorted_keywords: Vec<_> = keyword_freq.into_iter().collect();
    sorted_keywords.sort_by(|a, b| b.1.cmp(&a.1));
    
    // 前 20% 是热点关键词
    let hotspot_size = (sorted_keywords.len() as f64 * 0.2).ceil() as usize;
    let hot_keywords: Vec<_> = sorted_keywords[..hotspot_size].iter()
        .map(|(k, _)| k.clone())
        .collect();
    let cold_keywords: Vec<_> = sorted_keywords[hotspot_size..].iter()
        .map(|(k, _)| k.clone())
        .collect();
    
    println!("  热点关键词: {} | 冷关键词: {}", hot_keywords.len(), cold_keywords.len());
    
    let num_queries = 300;
    let start = Instant::now();
    let mut hot_access = 0;
    let mut cold_access = 0;
    let mut client = create_client(manager_addr.to_string()).await?;
    
    for i in 0..num_queries {
        let is_hot = rng.gen::<f64>() < 0.8;
        let keyword = if is_hot && !hot_keywords.is_empty() {
            hot_access += 1;
            hot_keywords.choose(&mut rng).unwrap()
        } else if !cold_keywords.is_empty() {
            cold_access += 1;
            cold_keywords.choose(&mut rng).unwrap()
        } else {
            continue;
        };
        
        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(keyword.clone())),
        };
        
        if let Err(e) = client.query(request).await {
            if e.code() == tonic::Code::Unavailable {
                client = create_client(manager_addr.to_string()).await?;
            }
        }
        
        if i % 50 == 0 && i > 0 {
            sleep(Duration::from_millis(20)).await;
        }
    }
    
    let duration = start.elapsed();
    let qps = (num_queries as f64 / duration.as_secs_f64()).round();
    
    println!("  总查询: {} | 热点访问: {} ({}%) | 冷数据访问: {} ({}%)",
        num_queries, hot_access, (hot_access * 100 / num_queries),
        cold_access, (cold_access * 100 / num_queries));
    println!("  耗时: {:?} | QPS: {}", duration, qps);
    
    Ok(())
}

// Workload 5: 混合负载
async fn run_mixed_workload(manager_addr: &str, dataset: &[Record]) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = rand::thread_rng();
    let num_ops = 500;
    let read_ratio = 0.7;
    
    let start = Instant::now();
    let mut reads = 0;
    let mut writes = 0;
    let mut client = create_client(manager_addr.to_string()).await?;
    
    for i in 0..num_ops {
        if rng.gen::<f64>() < read_ratio {
            // 读操作
            let record = dataset.choose(&mut rng).unwrap();
            let keyword = record.keywords.choose(&mut rng).unwrap();
            
            let request = QueryRequest {
                query_type: Some(common::rpc::query_request::QueryType::Keyword(keyword.clone())),
            };
            
            match client.query(request).await {
                Ok(_) => reads += 1,
                Err(e) if e.code() == tonic::Code::Unavailable => {
                    client = create_client(manager_addr.to_string()).await?;
                }
                _ => {}
            }
        } else {
            // 写操作(添加新记录)
            let base_record = dataset.choose(&mut rng).unwrap();
            let fid = format!("{}_{}", base_record.fid, rng.gen::<u32>());
            
            let request = AddRequest {
                fid,
                keywords: base_record.keywords.clone(),
            };
            
            match client.add(request).await {
                Ok(_) => writes += 1,
                Err(e) if e.code() == tonic::Code::Unavailable => {
                    client = create_client(manager_addr.to_string()).await?;
                }
                _ => {}
            }
        }
        
        if i % 50 == 0 && i > 0 {
            sleep(Duration::from_millis(20)).await;
        }
    }
    
    let duration = start.elapsed();
    let throughput = (num_ops as f64 / duration.as_secs_f64()).round();
    
    println!("  总操作: {} | 读: {} ({}%) | 写: {} ({}%)",
        num_ops, reads, (reads * 100 / num_ops), writes, (writes * 100 / num_ops));
    println!("  耗时: {:?} | 吞吐量: {} ops/s", duration, throughput);
    
    Ok(())
}

// Workload 6: 复杂布尔查询
async fn run_complex_queries(manager_addr: &str, dataset: &[Record]) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = rand::thread_rng();
    
    // 收集常见关键词
    let mut all_keywords: Vec<String> = Vec::new();
    for record in dataset {
        all_keywords.extend(record.keywords.clone());
    }
    all_keywords.sort();
    all_keywords.dedup();
    
    if all_keywords.len() < 4 {
        println!("  关键词不足,跳过");
        return Ok(());
    }
    
    let num_queries = 100;
    let start = Instant::now();
    let mut total_results = 0;
    let mut client = create_client(manager_addr.to_string()).await?;
    
    for i in 0..num_queries {
        let kw1 = all_keywords.choose(&mut rng).unwrap().clone();
        let kw2 = all_keywords.choose(&mut rng).unwrap().clone();
        let kw3 = all_keywords.choose(&mut rng).unwrap().clone();
        
        // 生成不同类型的布尔函数字符串
        let boolean_function = match i % 4 {
            0 => {
                // AND 查询
                format!("({} AND {})", kw1, kw2)
            }
            1 => {
                // OR 查询
                format!("({} OR {})", kw1, kw2)
            }
            2 => {
                // NOT 查询
                format!("(NOT {})", kw1)
            }
            _ => {
                // 嵌套查询: (kw1 AND kw2) OR kw3
                format!("(({} AND {}) OR {})", kw1, kw2, kw3)
            }
        };
        
        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::BooleanFunction(boolean_function)),
        };
        
        match client.query(request).await {
            Ok(response) => {
                total_results += response.into_inner().fids.len();
            }
            Err(e) if e.code() == tonic::Code::Unavailable => {
                client = create_client(manager_addr.to_string()).await?;
            }
            _ => {}
        }
        
        if i % 25 == 0 && i > 0 {
            sleep(Duration::from_millis(20)).await;
        }
    }
    
    let duration = start.elapsed();
    let qps = (num_queries as f64 / duration.as_secs_f64()).round();
    let avg_results = total_results / num_queries;
    
    println!("  查询数: {} | 平均结果数: {} | 耗时: {:?} | QPS: {}",
        num_queries, avg_results, duration, qps);
    
    Ok(())
}

// Workload 7: 更新负载
async fn run_update_workload(manager_addr: &str, dataset: &[Record]) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = rand::thread_rng();
    let num_updates = 200.min(dataset.len());
    
    let start = Instant::now();
    let mut success = 0;
    let mut client = create_client(manager_addr.to_string()).await?;
    
    for i in 0..num_updates {
        let record = dataset.choose(&mut rng).unwrap();
        
        // 随机修改关键词
        let mut new_keywords = record.keywords.clone();
        if new_keywords.len() > 1 {
            let idx = rng.gen_range(1..new_keywords.len());
            new_keywords[idx] = format!("updated_{}", rng.gen::<u32>() % 100);
        }
        
        let request = UpdateRequest {
            fid: record.fid.clone(),
            old_keywords: record.keywords.clone(),
            new_keywords: new_keywords,
        };
        
        match client.update(request).await {
            Ok(_) => success += 1,
            Err(e) if e.code() == tonic::Code::Unavailable => {
                client = create_client(manager_addr.to_string()).await?;
            }
            _ => {}
        }
        
        if i % 50 == 0 && i > 0 {
            sleep(Duration::from_millis(20)).await;
        }
    }
    
    let duration = start.elapsed();
    let throughput = (success as f64 / duration.as_secs_f64()).round();
    
    println!("  更新数: {} | 成功: {} | 耗时: {:?} | 吞吐量: {} ops/s",
        num_updates, success, duration, throughput);
    
    Ok(())
}
