use serde::Deserialize;
use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use toml;

#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub struct RuntimeConfig {
    pub num_clients: Option<usize>,
    pub num_storagers: Option<usize>,
    pub ads_mode: Option<String>,
    pub manager_addr: Option<String>,
    pub storager_addrs: Option<Vec<String>>,
    pub client_addrs: Option<Vec<String>>,
}

const MANAGER_ADDR_CANDIDATES: [&str; 4] = [
    "scripts/data/manageraddrs",
    "../scripts/data/manageraddrs",
    "../../scripts/data/manageraddrs",
    "../../../scripts/data/manageraddrs",
];

fn normalize_manager_addr(line: &str) -> String {
    line.strip_prefix("http://")
        .or_else(|| line.strip_prefix("https://"))
        .unwrap_or(line)
        .to_string()
}

fn read_first_manager_addr_line() -> Option<String> {
    for candidate in MANAGER_ADDR_CANDIDATES {
        if let Ok(raw) = fs::read_to_string(candidate) {
            for line in raw.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                return Some(normalize_manager_addr(trimmed));
            }
        }
    }
    None
}

pub fn load_manager_bind_addr_from_file() -> Option<SocketAddr> {
    read_first_manager_addr_line()?.parse().ok()
}

pub fn load_manager_http_addr_from_file() -> Option<String> {
    read_first_manager_addr_line().map(|addr| format!("http://{}", addr))
}

impl RuntimeConfig {
    /// Load config from given file path (defaults to `config.json`).
    /// Environment variables override values when present. Examples:
    /// - SYS_NUM_STORAGERS
    /// - SYS_MANAGER_ADDR
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let raw = fs::read_to_string(&path).map_err(|e| format!("read config failed: {}", e))?;
        // Try to parse as TOML first if file extension indicates TOML, otherwise fall back to JSON.
        let path_ref = path.as_ref();
        let mut cfg: RuntimeConfig =
            if let Some(ext) = path_ref.extension().and_then(|s| s.to_str()) {
                if ext.eq_ignore_ascii_case("toml") {
                    toml::from_str(&raw).map_err(|e| format!("parse TOML config failed: {}", e))?
                } else {
                    serde_json::from_str(&raw)
                        .map_err(|e| format!("parse JSON config failed: {}", e))?
                }
            } else {
                // no extension — try TOML first, then JSON
                match toml::from_str(&raw) {
                    Ok(c) => c,
                    Err(_) => serde_json::from_str(&raw)
                        .map_err(|e| format!("parse JSON config failed: {}", e))?,
                }
            };

        // env overrides
        if let Ok(v) = env::var("SYS_NUM_CLIENTS") {
            if let Ok(n) = v.parse::<usize>() {
                cfg.num_clients = Some(n);
            }
        }
        if let Ok(v) = env::var("SYS_NUM_STORAGERS") {
            if let Ok(n) = v.parse::<usize>() {
                cfg.num_storagers = Some(n);
            }
        }
        if let Ok(v) = env::var("SYS_ADS_MODE") {
            cfg.ads_mode = Some(v);
        }
        if let Ok(v) = env::var("SYS_MANAGER_ADDR") {
            cfg.manager_addr = Some(v);
        }
        if let Ok(v) = env::var("SYS_STORAGER_ADDRS") {
            // comma separated
            let addrs: Vec<String> = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !addrs.is_empty() {
                cfg.storager_addrs = Some(addrs);
            }
        }
        // client_addrs via SYS_CLIENT_ADDRS
        if let Ok(v) = env::var("SYS_CLIENT_ADDRS") {
            let addrs: Vec<String> = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !addrs.is_empty() {
                cfg.client_addrs = Some(addrs);
            }
        }

        Ok(cfg)
    }

    /// Load from default `config.json` in repo root
    pub fn load_default() -> Result<Self, String> {
        // Prefer configs/prod.toml if present, then configs/dev.toml, then configs/bench.toml
        let candidates = [
            "configs/prod.toml",
            "configs/dev.toml",
            "configs/bench.toml",
        ];
        for c in &candidates {
            if Path::new(c).exists() {
                return Self::load_from_file(c);
            }
        }
        Err(
            "no config file found (tried configs/system.toml, configs/dev.toml, config.json)"
                .into(),
        )
    }

    /// Basic validation to ensure required fields exist
    pub fn validate(&self) -> Result<(), String> {
        if self
            .manager_addr
            .as_ref()
            .map(|s| s.is_empty())
            .unwrap_or(true)
        {
            return Err("manager_addr missing or empty".into());
        }
        if self
            .storager_addrs
            .as_ref()
            .map(|v| v.is_empty())
            .unwrap_or(true)
        {
            return Err("storager_addrs missing or empty".into());
        }
        Ok(())
    }
}
