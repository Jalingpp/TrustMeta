use common::rpc::{
    manager_service_client::ManagerServiceClient, AddRequest, QueryRequest, query_request::QueryType,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Barrier;
use tokio::time::sleep;

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
    let args: Vec<String> = std::env::args().collect();
    let concurrency_level: usize = if args.len() > 1 {
        args[1].parse().unwrap_or(10)
    } else {
        10
    };
    let total_operations: usize = if args.len() > 2 {
        args[2].parse().unwrap_or(1000)
    } else {
        1000
    };
    let manager_addr = "http://[::1]:50051".to_string();

    println!("并发测试配置:");
    println!("  • 并发数: {}", concurrency_level);
    println!("  • 总操作数: {}", total_operations);
    println!("  • Manager 地址: {}", manager_addr);

    let ops_per_thread = total_operations / concurrency_level;
    let barrier = Arc::new(Barrier::new(concurrency_level));
    
    let mut handles = Vec::new();
    let start_time = Instant::now();

    for i in 0..concurrency_level {
        let manager_addr = manager_addr.clone();
        let barrier = barrier.clone();
        let thread_id = i;
        
        let handle = tokio::spawn(async move {
            let mut client = match create_client(manager_addr).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Thread {} failed to connect: {}", thread_id, e);
                    return 0;
                }
            };

            // Wait for all threads to be ready
            barrier.wait().await;
            
            let mut success_count = 0;
            for j in 0..ops_per_thread {
                let fid = format!("fid_{}_{}", thread_id, j);
                let keyword = format!("kw_{}_{}", thread_id, j);
                
                // Add operation
                let request = tonic::Request::new(AddRequest {
                    fid: fid.clone(),
                    keywords: vec![keyword.clone()],
                });
                
                if let Ok(_) = client.add(request).await {
                    success_count += 1;
                }

                // Query operation (optional, to mix load)
                let query_req = tonic::Request::new(QueryRequest {
                    query_type: Some(QueryType::Keyword(keyword)),
                });
                let _ = client.query(query_req).await;
            }
            success_count
        });
        handles.push(handle);
    }

    let mut total_success = 0;
    for handle in handles {
        total_success += handle.await?;
    }

    let duration = start_time.elapsed();
    let tps = total_operations as f64 / duration.as_secs_f64();

    println!("\n测试结果:");
    println!("  • 总耗时: {:.2?}", duration);
    println!("  • 成功操作数: {}/{}", total_success, total_operations); // Note: this counts Add successes only
    println!("  • TPS (Add+Query mixed): {:.2}", tps); // TPS calculation is rough here, strictly it's (Add ops)/time if we only count Adds in total_operations loop logic, but we did Add + Query per loop.
    // Let's clarify: total_operations passed in arg is "loop count" per thread * threads.
    // Actual RPC calls = total_operations * 2 (1 Add + 1 Query).
    
    let actual_rpc_count = total_operations * 2;
    let actual_tps = actual_rpc_count as f64 / duration.as_secs_f64();
    
    println!("  • 实际 RPC 总数: {}", actual_rpc_count);
    println!("  • 实际 TPS: {:.2}", actual_tps);

    Ok(())
}
