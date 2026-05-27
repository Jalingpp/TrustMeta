//! 进程管理器 - 启动和管理 Manager 和 Storager 进程

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;

/// 系统进程管理器
pub struct ProcessManager {
    manager_process: Option<Child>,
    storager_processes: Vec<Child>,
    manager_bind_addr: SocketAddr,
    storager_ports: Vec<u16>,
}

impl ProcessManager {
    /// 创建新的进程管理器
    pub fn new(storager_ports: Vec<u16>) -> Self {
        Self {
            manager_process: None,
            storager_processes: Vec::new(),
            manager_bind_addr: common::config::load_manager_bind_addr_from_file()
                .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 50051))),
            storager_ports,
        }
    }

    /// 启动完整系统
    pub async fn start_system(&mut self, ads_mode: &str) -> Result<()> {
        println!(
            "\n{}",
            "🚀 Starting distributed storage system..."
                .bright_cyan()
                .bold()
        );

        // 确保日志目录存在（位于 experiments/logs），避免在创建日志文件时发生 "No such file or directory"
        fs::create_dir_all("experiments/logs")
            .context("Failed to create experiments/logs directory")?;

        // 1. 启动所有 Storager 节点
        println!("\n📦 Starting Storager nodes...");
        let ports = self.storager_ports.clone();
        for (idx, port) in ports.iter().enumerate() {
            self.start_storager(*port, ads_mode)
                .with_context(|| format!("Failed to start Storager on port {}", port))?;
            println!("  ✓ Storager {} started on port {}", idx + 1, port);
            sleep(Duration::from_millis(500)).await; // 等待启动
        }

        // 2. 等待 Storager 完全启动
        println!("\n⏳ Waiting for Storagers to initialize...");
        sleep(Duration::from_secs(2)).await;

        // 3. 启动 Manager
        println!("\n🎯 Starting Manager node...");
        self.start_manager(ads_mode)
            .context("Failed to start Manager")?;
        println!("  ✓ Manager started on {}", self.manager_bind_addr);

        // 4. 等待 Manager 完全启动
        println!("\n⏳ Waiting for Manager to initialize...");
        sleep(Duration::from_secs(2)).await;

        println!("\n{}", "✨ System startup completed!".bright_green().bold());
        self.print_system_info();

        Ok(())
    }

    /// 启动 Manager 进程
    fn start_manager(&mut self, ads_mode: &str) -> Result<()> {
        let storager_addrs = self
            .storager_ports
            .iter()
            .map(|p| format!("http://127.0.0.1:{}", p))
            .collect::<Vec<_>>()
            .join(",");

        // 首先尝试使用 `cargo run` 启动（开发环境），若 `cargo` 不可用则回退到直接运行已编译的二进制
        let child = match Command::new("cargo")
            .args([
                "run",
                "--release",
                "--bin",
                "manager",
                "--",
                "--bind-addr",
                &self.manager_bind_addr.to_string(),
                "--ads-mode",
                ads_mode,
                "--storagers",
                &storager_addrs,
            ])
            .stdout(Stdio::from(File::create("experiments/logs/manager.log")?))
            .stderr(Stdio::from(File::create("experiments/logs/manager.err")?))
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                if e.kind() == ErrorKind::NotFound {
                    // 回退到 target/release/manager 可执行文件
                    Command::new("./target/release/manager")
                        .args([
                            "--bind-addr",
                            &self.manager_bind_addr.to_string(),
                            "--ads-mode",
                            ads_mode,
                            "--storagers",
                            &storager_addrs,
                        ])
                        .stdout(Stdio::from(File::create("experiments/logs/manager.log")?))
                        .stderr(Stdio::from(File::create("experiments/logs/manager.err")?))
                        .spawn()
                        .context("Failed to spawn Manager fallback binary")?
                } else {
                    return Err(e).context("Failed to spawn Manager process")?;
                }
            }
        };

        self.manager_process = Some(child);
        Ok(())
    }

    /// 启动 Storager 进程
    fn start_storager(&mut self, port: u16, ads_mode: &str) -> Result<()> {
        // 同样先尝试用 cargo 启动，失败时回退到已编译的二进制
        let child = match Command::new("cargo")
            .args([
                "run",
                "--release",
                "--bin",
                "storager",
                "--",
                &port.to_string(),
                ads_mode,
            ])
            .stdout(Stdio::from(File::create(format!(
                "experiments/logs/storager_{}.log",
                port
            ))?))
            .stderr(Stdio::from(File::create(format!(
                "experiments/logs/storager_{}.err",
                port
            ))?))
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                if e.kind() == ErrorKind::NotFound {
                    Command::new(format!("./target/release/storager"))
                        .args([&port.to_string(), ads_mode])
                        .stdout(Stdio::from(File::create(format!(
                            "experiments/logs/storager_{}.log",
                            port
                        ))?))
                        .stderr(Stdio::from(File::create(format!(
                            "experiments/logs/storager_{}.err",
                            port
                        ))?))
                        .spawn()
                        .with_context(|| {
                            format!("Failed to spawn Storager fallback binary on port {}", port)
                        })?
                } else {
                    return Err(e)
                        .with_context(|| format!("Failed to spawn Storager on port {}", port))?;
                }
            }
        };

        self.storager_processes.push(child);
        Ok(())
    }

    /// 打印系统信息
    fn print_system_info(&self) {
        println!("\n{}", "═".repeat(60).bright_blue());
        println!("{}", "  SYSTEM INFORMATION".bright_blue().bold());
        println!("{}", "═".repeat(60).bright_blue());
        println!("  Manager:   http://{}", self.manager_bind_addr);
        println!("  Storagers: {} nodes", self.storager_processes.len());
        for (idx, port) in self.storager_ports.iter().enumerate() {
            println!("    - Storager {}: http://127.0.0.1:{}", idx + 1, port);
        }
        println!("{}", "═".repeat(60).bright_blue());
    }

    /// 停止所有进程
    pub fn shutdown(&mut self) -> Result<()> {
        println!("\n{}", "🛑 Shutting down system...".bright_yellow());

        // 停止 Manager
        if let Some(mut process) = self.manager_process.take() {
            let _ = process.kill();
            let _ = process.wait();
            println!("  ✓ Manager stopped");
        }

        // 停止所有 Storager
        for (idx, mut process) in self.storager_processes.drain(..).enumerate() {
            let _ = process.kill();
            let _ = process.wait();
            println!("  ✓ Storager {} stopped", idx + 1);
        }

        println!("{}", "✨ System shutdown completed".bright_green());
        Ok(())
    }

    /// 获取 Manager 地址
    pub fn manager_addr(&self) -> String {
        format!("http://{}", self.manager_bind_addr)
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
