//! 进程管理器 - 启动和管理 Manager 和 Storager 进程

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs::File;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;

/// 系统进程管理器
pub struct ProcessManager {
    manager_process: Option<Child>,
    storager_processes: Vec<Child>,
    manager_port: u16,
    storager_ports: Vec<u16>,
}

impl ProcessManager {
    /// 创建新的进程管理器
    pub fn new(manager_port: u16, storager_ports: Vec<u16>) -> Self {
        Self {
            manager_process: None,
            storager_processes: Vec::new(),
            manager_port,
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
        println!("  ✓ Manager started on port {}", self.manager_port);

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

        let child = Command::new("cargo")
            .args([
                "run",
                "--release",
                "--bin",
                "manager",
                "--",
                "--port",
                &self.manager_port.to_string(),
                "--ads-mode",
                ads_mode,
                "--storagers",
                &storager_addrs,
            ])
            .stdout(Stdio::from(File::create("logs/manager.log")?))
            .stderr(Stdio::from(File::create("logs/manager.err")?))
            .spawn()
            .context("Failed to spawn Manager process")?;

        self.manager_process = Some(child);
        Ok(())
    }

    /// 启动 Storager 进程
    fn start_storager(&mut self, port: u16, ads_mode: &str) -> Result<()> {
        let child = Command::new("cargo")
            .args([
                "run",
                "--release",
                "--bin",
                "storager",
                "--",
                &port.to_string(),
                ads_mode,
            ])
            .stdout(Stdio::from(File::create(format!("logs/storager_{}.log", port))?))
            .stderr(Stdio::from(File::create(format!("logs/storager_{}.err", port))?))
            .spawn()
            .with_context(|| format!("Failed to spawn Storager on port {}", port))?;

        self.storager_processes.push(child);
        Ok(())
    }

    /// 打印系统信息
    fn print_system_info(&self) {
        println!("\n{}", "═".repeat(60).bright_blue());
        println!("{}", "  SYSTEM INFORMATION".bright_blue().bold());
        println!("{}", "═".repeat(60).bright_blue());
        println!("  Manager:   http://127.0.0.1:{}", self.manager_port);
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
        format!("http://127.0.0.1:{}", self.manager_port)
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
