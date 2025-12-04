//! 系统级性能指标收集

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 系统级性能指标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    /// 操作类型统计
    pub operation_stats: OperationStats,
    
    /// 端到端延迟 (包含网络通信)
    pub end_to_end_latency: LatencyStats,
    
    /// 总吞吐量
    pub total_throughput: f64,
    
    /// 测试持续时间
    pub total_duration: Duration,
    
    /// 成功/失败统计
    pub success_count: usize,
    pub failure_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationStats {
    pub add_count: usize,
    pub query_count: usize,
    pub update_count: usize,
    pub delete_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    pub min_ms: f64,
    pub max_ms: f64,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
}

impl SystemMetrics {
    pub fn new() -> Self {
        Self {
            operation_stats: OperationStats {
                add_count: 0,
                query_count: 0,
                update_count: 0,
                delete_count: 0,
            },
            end_to_end_latency: LatencyStats {
                min_ms: f64::MAX,
                max_ms: 0.0,
                avg_ms: 0.0,
                p50_ms: 0.0,
                p95_ms: 0.0,
                p99_ms: 0.0,
            },
            total_throughput: 0.0,
            total_duration: Duration::from_secs(0),
            success_count: 0,
            failure_count: 0,
        }
    }

    /// 计算延迟百分位数
    pub fn calculate_percentiles(&mut self, latencies: &mut Vec<f64>) {
        if latencies.is_empty() {
            return;
        }

        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());

        self.end_to_end_latency.min_ms = latencies[0];
        self.end_to_end_latency.max_ms = latencies[latencies.len() - 1];
        self.end_to_end_latency.avg_ms = latencies.iter().sum::<f64>() / latencies.len() as f64;

        let p50_idx = (latencies.len() as f64 * 0.50) as usize;
        let p95_idx = (latencies.len() as f64 * 0.95) as usize;
        let p99_idx = (latencies.len() as f64 * 0.99) as usize;

        self.end_to_end_latency.p50_ms = latencies[p50_idx.min(latencies.len() - 1)];
        self.end_to_end_latency.p95_ms = latencies[p95_idx.min(latencies.len() - 1)];
        self.end_to_end_latency.p99_ms = latencies[p99_idx.min(latencies.len() - 1)];
    }

    /// 计算总吞吐量
    pub fn calculate_throughput(&mut self) {
        let total_ops = self.success_count + self.failure_count;
        if self.total_duration.as_secs_f64() > 0.0 {
            self.total_throughput = total_ops as f64 / self.total_duration.as_secs_f64();
        }
    }

    /// 成功率
    pub fn success_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total > 0 {
            (self.success_count as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self::new()
    }
}
