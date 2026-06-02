//! Manager 服务入口
//!
//! Manager 负责：
//! - 接收客户端请求
//! - 按配置使用 EPRing 或一致性哈希路由到对应的 storager
//! - 验证 storager 返回的证明
//! - 处理布尔查询
//!
//! # 使用方法
//! ```bash
//! # 使用默认配置（端口 50051，AccTrie）
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
use common::{
    config::load_manager_bind_addr_from_file, init_accumulator_public_parameters, AdsMode,
    SetProofMode,
};
use manager::core::{Manager, RouteMode};
use std::net::SocketAddr;
use tonic::transport::Server;

fn env_duration_secs(key: &str, default_secs: u64) -> std::time::Duration {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| std::time::Duration::from_secs(default_secs))
}

fn env_optional_duration_secs(key: &str, default_secs: Option<u64>) -> Option<std::time::Duration> {
    match std::env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .and_then(|secs| (secs > 0).then(|| std::time::Duration::from_secs(secs))),
        Err(_) => default_secs.map(std::time::Duration::from_secs),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 解析命令行参数
    let args: Vec<String> = std::env::args().collect();

    let mut bind_addr = load_manager_bind_addr_from_file();
    let mut port = bind_addr.map(|addr| addr.port()).unwrap_or(50051u16);
    let mut bind_addr_explicit = false;
    let mut ads_mode = AdsMode::AccTrie;
    let mut set_proof_mode = SetProofMode::Accumulator;
    let mut split_threshold: usize = 150;
    let mut route_mode = RouteMode::Epring;
    let mut storager_addrs = vec![
        "http://127.0.0.1:50052".to_string(),
        "http://127.0.0.1:50053".to_string(),
    ];

    // 简单的命令行参数解析
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!(
                            "Invalid port number: {}, using default (50051)",
                            args[i + 1]
                        );
                        50051
                    });
                    if !bind_addr_explicit {
                        if let Some(addr) = bind_addr.as_mut() {
                            *addr = SocketAddr::new(addr.ip(), port);
                        }
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--bind-addr" => {
                if i + 1 < args.len() {
                    bind_addr = Some(args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!(
                            "Invalid bind address: {}, using default (127.0.0.1:{})",
                            args[i + 1],
                            port
                        );
                        SocketAddr::from(([127, 0, 0, 1], port))
                    }));
                    bind_addr_explicit = true;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--ads-mode" | "-a" => {
                if i + 1 < args.len() {
                    ads_mode = args[i + 1].parse().unwrap_or_else(|err| {
                        eprintln!("{}; using default ({})", err, AdsMode::AccTrie);
                        AdsMode::AccTrie
                    });
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
            "--set-proof-mode" => {
                if i + 1 < args.len() {
                    set_proof_mode = args[i + 1].parse().unwrap_or_else(|err| {
                        eprintln!("{}; using default ({})", err, SetProofMode::Polynomial);
                        SetProofMode::Polynomial
                    });
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--split-threshold" => {
                if i + 1 < args.len() {
                    split_threshold = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!(
                            "Invalid split threshold: {}, using default (150)",
                            args[i + 1]
                        );
                        150
                    });
                    if split_threshold == 0 {
                        split_threshold = 150;
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--route-mode" => {
                if i + 1 < args.len() {
                    route_mode = args[i + 1].parse().unwrap_or_else(|err| {
                        eprintln!("{}; using default ({})", err, RouteMode::Epring);
                        RouteMode::Epring
                    });
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

    init_accumulator_public_parameters()?;

    let addr = bind_addr.unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], port)));

    let manager = Manager::new_with_route_mode(
        storager_addrs.clone(),
        ads_mode,
        set_proof_mode,
        split_threshold,
        route_mode,
    );
    manager.set_metrics_tag(format!(
        "{}-{}-{}",
        port,
        ads_mode.as_str(),
        route_mode.as_str()
    ));

    println!("🚀 Manager server starting...");
    println!("   Listening on: {}", addr);
    println!("   ADS Mode: {:?}", ads_mode);
    println!("   Set Proof Mode: {}", set_proof_mode);
    println!("   Route Mode: {}", route_mode);
    println!("   Split Threshold: {}", split_threshold);
    println!("   Storagers: {:?}", storager_addrs);

    let manager_metrics_flush_interval =
        env_duration_secs("MANAGER_METRICS_FLUSH_INTERVAL_SECS", 5);
    let manager_prefix_flush_interval =
        env_duration_secs("MANAGER_PREFIX_REPORT_FLUSH_INTERVAL_SECS", 5);
    manager.spawn_background_report_flushers(
        manager_metrics_flush_interval,
        manager_prefix_flush_interval,
    );

    let tcp_keepalive = env_duration_secs("MANAGER_SERVER_TCP_KEEPALIVE_SECS", 60);
    let http2_keepalive_interval =
        env_optional_duration_secs("MANAGER_SERVER_HTTP2_KEEPALIVE_INTERVAL_SECS", Some(30));
    let http2_keepalive_timeout =
        env_optional_duration_secs("MANAGER_SERVER_HTTP2_KEEPALIVE_TIMEOUT_SECS", Some(10));

    let mut server = Server::builder()
        .tcp_keepalive(Some(tcp_keepalive))
        .tcp_nodelay(true)
        .http2_adaptive_window(Some(true))
        .concurrency_limit_per_connection(256);

    if let Some(interval) = http2_keepalive_interval {
        server = server.http2_keepalive_interval(Some(interval));
        if let Some(timeout) = http2_keepalive_timeout {
            server = server.http2_keepalive_timeout(Some(timeout));
        }
    }

    server
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
        "        --bind-addr <ADDR>         Set listen address, e.g. 0.0.0.0:50051 (default: scripts/data/manageraddrs)"
    );
    println!(
        "    -a, --ads-mode <MODE>          Set ADS mode: mpt|mest|acctrie|acctree (default: mest)"
    );
    println!("        --set-proof-mode <MODE>    Set set proof mode: polynomial|accumulator (default: polynomial)");
    println!("    -s, --storagers <ADDRS>        Comma-separated storager addresses");
    println!("        --split-threshold <N>      Set EPRing split threshold (default: 150)");
    println!(
        "        --route-mode <MODE>        Set routing backend: epring|chring (default: epring)"
    );
    println!("    -h, --help                     Print this help message");
    println!();
    println!("EXAMPLES:");
    println!("    manager --port 50051");
    println!("    manager --bind-addr 0.0.0.0:50051");
    println!("    manager --ads-mode mpt");
    println!("    manager --set-proof-mode accumulator");
    println!("    manager --split-threshold 300");
    println!("    manager --route-mode chring");
    println!("    manager --storagers \"http://127.0.0.1:50052,http://127.0.0.1:50053\"");
}
