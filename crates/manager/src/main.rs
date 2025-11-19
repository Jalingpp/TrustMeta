//! Manager 服务入口
//!
//! Manager 负责：
//! - 接收客户端请求
//! - 使用一致性哈希路由到对应的 storager
//! - 验证 storager 返回的证明
//! - 处理布尔查询
//!
//! # 使用方法
//! ```bash
//! # 使用默认配置（端口 50051，MEST）
//! cargo run --bin manager
//!
//! # 指定 ADS 模式
//! cargo run --bin manager -- --ads-mode mpt
//! cargo run --bin manager -- --ads-mode mest
//!
//! # 指定端口
//! cargo run --bin manager -- --port 50051
//!
//! # 指定 storager 地址（逗号分隔）
//! cargo run --bin manager -- --storagers "http://[::1]:50052,http://[::1]:50053"
//! ```

use common::rpc::manager_service_server::ManagerServiceServer;
use common::AdsMode;
use manager::Manager;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 解析命令行参数
    let args: Vec<String> = std::env::args().collect();

    let mut port = 50051u16;
    let mut ads_mode = AdsMode::Mest;
    let mut storager_addrs = vec![
        "http://[::1]:50052".to_string(),
        "http://[::1]:50053".to_string(),
    ];

    // 简单的命令行参数解析
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse().unwrap_or(50051);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--ads-mode" | "-a" => {
                if i + 1 < args.len() {
                    ads_mode = match args[i + 1].to_lowercase().as_str() {
                        "mpt" => AdsMode::Mpt,
                        "mest" => AdsMode::Mest,
                        _ => {
                            eprintln!("Unknown ADS mode: {}, using default (MEST)", args[i + 1]);
                            AdsMode::Mest
                        }
                    };
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--storagers" | "-s" => {
                if i + 1 < args.len() {
                    storager_addrs = args[i + 1]
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            _ => {
                i += 1;
            }
        }
    }

    let addr = format!("[::1]:{}", port).parse()?;

    let manager = Manager::new(storager_addrs.clone(), ads_mode);

    println!("🚀 Manager server starting...");
    println!("   Listening on: {}", addr);
    println!("   ADS Mode: {:?}", ads_mode);
    println!("   Storagers: {:?}", storager_addrs);

    // 配置服务器以提高并发性能
    Server::builder()
        .tcp_keepalive(Some(std::time::Duration::from_secs(60)))  // TCP keepalive
        .tcp_nodelay(true)  // 禁用 Nagle 算法,减少延迟
        .http2_keepalive_interval(Some(std::time::Duration::from_secs(30)))  // HTTP/2 keepalive
        .http2_keepalive_timeout(Some(std::time::Duration::from_secs(10)))
        .http2_adaptive_window(Some(true))  // 自适应流控窗口
        .concurrency_limit_per_connection(256)  // 每个连接的并发请求数
        .add_service(ManagerServiceServer::new(manager))
        .serve(addr)
        .await?;

    Ok(())
}

fn print_help() {
    println!("Manager Server - Distributed Storage System");
    println!();
    println!("USAGE:");
    println!("    manager [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -p, --port <PORT>              Set the server port (default: 50051)");
    println!(
        "    -a, --ads-mode <MODE>          Set ADS mode: mpt|mest (default: mest)"
    );
    println!("    -s, --storagers <ADDRS>        Comma-separated storager addresses");
    println!("    -h, --help                     Print this help message");
    println!();
    println!("EXAMPLES:");
    println!("    manager --port 50051");
    println!("    manager --ads-mode mpt");
    println!("    manager --storagers \"http://[::1]:50052,http://[::1]:50053\"");
}
