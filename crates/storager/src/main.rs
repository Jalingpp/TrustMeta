use common::config::RuntimeConfig;
use common::init_accumulator_public_parameters;
use common::rpc::storager_service_server::StoragerServiceServer;
use serde::Deserialize;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use storager::Storager;
use tonic::transport::Server;

const STORAGER_DATA_ROOT: &str = "scripts/data/storager";

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

#[derive(Deserialize)]
struct Config {
    ads_mode: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut port = 50052u16;
    let mut bind_addr: Option<SocketAddr> = None;
    let mut ads_type_arg: Option<String> = None;
    let mut storager_id_arg: Option<String> = None;
    let mut acctrie_persistence_arg: Option<String> = None;
    let mut positional_args: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse::<u16>().unwrap_or(50052);
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
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--ads-mode" | "-a" => {
                if i + 1 < args.len() {
                    ads_type_arg = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--storager-id" | "-s" => {
                if i + 1 < args.len() {
                    storager_id_arg = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--acctrie-persistence" => {
                if i + 1 < args.len() {
                    acctrie_persistence_arg = Some(args[i + 1].clone());
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
                positional_args.push(args[i].clone());
                i += 1;
            }
        }
    }

    if let Some(value) = positional_args.first() {
        if let Ok(parsed_port) = value.parse::<u16>() {
            port = parsed_port;
            if ads_type_arg.is_none() {
                ads_type_arg = positional_args.get(1).cloned();
            }
        } else if ads_type_arg.is_none() {
            ads_type_arg = Some(value.clone());
        }
    }

    init_accumulator_public_parameters()?;

    let ads_type = if let Some(value) = ads_type_arg {
        value
    } else {
        match RuntimeConfig::load_default() {
            Ok(cfg) => cfg
                .ads_mode
                .unwrap_or_else(|| match fs::read_to_string("config.json") {
                    Ok(content) => serde_json::from_str::<Config>(&content)
                        .map(|c| c.ads_mode)
                        .unwrap_or_else(|_| "accumulator".to_string()),
                    Err(_) => "accumulator".to_string(),
                }),
            Err(_) => match fs::read_to_string("config.json") {
                Ok(content) => match serde_json::from_str::<Config>(&content) {
                    Ok(config) => config.ads_mode,
                    Err(_) => "accumulator".to_string(),
                },
                Err(_) => "accumulator".to_string(),
            },
        }
    };

    if acctrie_persistence_arg.is_none()
        && (ads_type.eq_ignore_ascii_case("acctrie")
            || ads_type.eq_ignore_ascii_case("accumulator"))
    {
        acctrie_persistence_arg = if positional_args
            .first()
            .map(|value| value.parse::<u16>().is_ok())
            .unwrap_or(false)
        {
            positional_args.get(2).cloned()
        } else {
            positional_args.get(1).cloned()
        };
    }
    let acctrie_persistence_mode = acctrie_persistence_arg.unwrap_or_else(|| "page".to_string());

    let addr = bind_addr.unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], port)));
    let storager_id = storager_id_arg.unwrap_or_else(|| format!("storager-{}", port));

    let storager = match ads_type.as_str() {
        "mpt" => {
            let data_dir = PathBuf::from(STORAGER_DATA_ROOT).join(format!("storager-{port}-mpt"));
            Storager::with_mpt_persistence(data_dir)
        }
        "mest" => {
            let data_dir = PathBuf::from(STORAGER_DATA_ROOT).join(format!("storager-{port}-mest"));
            Storager::with_mest_persistence(data_dir)
        }
        "acctrie" | "accumulator" => {
            let data_dir = PathBuf::from(STORAGER_DATA_ROOT).join(format!(
                "storager-{port}-acctrie-{}",
                acctrie_persistence_mode.to_lowercase()
            ));
            Storager::with_acctrie_persistence_mode(data_dir, acctrie_persistence_mode)
        }
        "acctree" => {
            let data_dir =
                PathBuf::from(STORAGER_DATA_ROOT).join(format!("storager-{port}-acctree"));
            Storager::with_acctree_persistence(data_dir)
        }
        _ => {
            eprintln!("Unknown ADS type '{}', using default (MEST)", ads_type);
            let data_dir = PathBuf::from(STORAGER_DATA_ROOT).join(format!("storager-{port}-mest"));
            Storager::with_mest_persistence(data_dir)
        }
    };
    storager.set_storager_id(storager_id.clone());
    storager.set_metrics_tag(format!("{port}-{ads_type}"));
    storager.set_ads_mode(ads_type.clone());

    println!("Storager server listening on {} (ADS: {})", addr, ads_type);

    let is_heavy = ads_type.eq_ignore_ascii_case("acctree") || ads_type.eq_ignore_ascii_case("mpt");
    let tcp_keepalive = if is_heavy {
        env_duration_secs("STORAGER_HEAVY_SERVER_TCP_KEEPALIVE_SECS", 300)
    } else {
        env_duration_secs("STORAGER_SERVER_TCP_KEEPALIVE_SECS", 120)
    };
    let http2_keepalive_interval = if is_heavy {
        env_optional_duration_secs(
            "STORAGER_HEAVY_SERVER_HTTP2_KEEPALIVE_INTERVAL_SECS",
            Some(120),
        )
    } else {
        env_optional_duration_secs("STORAGER_SERVER_HTTP2_KEEPALIVE_INTERVAL_SECS", Some(60))
    };
    let http2_keepalive_timeout = if is_heavy {
        env_optional_duration_secs(
            "STORAGER_HEAVY_SERVER_HTTP2_KEEPALIVE_TIMEOUT_SECS",
            Some(3600),
        )
    } else {
        env_optional_duration_secs("STORAGER_SERVER_HTTP2_KEEPALIVE_TIMEOUT_SECS", Some(120))
    };

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
        .add_service(StoragerServiceServer::new(storager))
        .serve(addr)
        .await?;

    Ok(())
}

fn print_help() {
    println!("Storager Server - Distributed Storage System");
    println!();
    println!("USAGE:");
    println!("    storager [OPTIONS]");
    println!("    storager <PORT> <ADS_MODE>");
    println!();
    println!("OPTIONS:");
    println!("    -p, --port <PORT>              Set the server port (default: 50052)");
    println!(
        "        --bind-addr <ADDR>         Set listen address, e.g. 0.0.0.0:50052 (default: 127.0.0.1:<port>)"
    );
    println!(
        "    -a, --ads-mode <MODE>          Set ADS mode: mpt|mest|acctrie|acctree (default: runtime config)"
    );
    println!("    -s, --storager-id <ID>        Set storager id used in output file names (default: storager-<port>)");
    println!("        --acctrie-persistence <MODE>  Set AccTrie persistence mode: page|kvdb (default: page)");
    println!("    -h, --help                     Print this help message");
    println!();
    println!("EXAMPLES:");
    println!("    storager 50052 mpt");
    println!("    storager --bind-addr 0.0.0.0:50052 --ads-mode acctrie --acctrie-persistence kvdb --storager-id sn1");
}
