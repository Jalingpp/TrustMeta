# 系统级性能测试 - 使用指南

## 📋 概述

系统级性能测试框架 (`system_benchmark`) 测试完整的分布式存储系统架构：

```
Client (客户端)
    ↓ gRPC
Manager (管理节点 - 一致性哈希路由)
    ↓ gRPC  
Storager 1/2/3... (存储节点 - ADS)
```

**与 ADS 层测试的区别**：
- ✅ **ADS层测试** (`benchmark`): 直接测试数据结构性能，无网络开销
- ✅ **系统级测试** (`system_benchmark`): 测试端到端性能，包含网络、路由、证明验证等真实开销

---

## 🚀 快速开始

### 方法 1: 使用脚本（推荐）

```bash
# 运行系统级测试（小规模数据，MPT模式，3个Storager）
./scripts/run_system_benchmark.sh

# 指定 workload 和 ADS 模式
./scripts/run_system_benchmark.sh data/workload_medium_10000.csv mest

# 完整参数
./scripts/run_system_benchmark.sh <workload_path> <ads_mode> <num_storagers>
```

### 方法 2: 直接运行

```bash
# 编译
cargo build --release --bin system_benchmark

# 运行（默认：小规模测试，MPT，3个Storager）
./target/release/system_benchmark

# 指定参数
./target/release/system_benchmark data/workload_small_1000.csv mpt 3
./target/release/system_benchmark data/workload_medium_10000.csv mest 5
```

---

## 📊 测试配置

### 可用的 Workload

- `data/workload_small_1000.csv` - 1,000条记录（推荐用于快速测试）
- `data/workload_medium_10000.csv` - 10,000条记录
- `data/workload_large_100000.csv` - 100,000条记录（需要较长时间）

### ADS 模式

- `mpt` - Merkle Patricia Trie
- `mest` - Merkle-based Extendible Segmented Hash Tree
- `acctrie` - Accumulator-based Trie

### Storager 节点数

- 推荐 3-5 个节点
- 更多节点可以测试负载均衡，但会增加启动时间

---

## 📈 测试流程

系统测试会自动执行以下步骤：

1. **启动系统组件**
   - 启动 N 个 Storager 进程（端口 50052-5005N）
   - 启动 1 个 Manager 进程（端口 50051）
   - 等待系统稳定（3秒预热）

2. **执行工作负载**
   - 通过 Client 发送请求到 Manager
   - Manager 使用一致性哈希路由到对应 Storager
   - 测量每个操作的端到端延迟：
     * Add (添加文件和关键词)
     * Query (查询关键词)
     * Update (更新文件关键词)
     * Delete (删除文件)

3. **收集性能指标**
   - 操作计数
   - 端到端延迟（Min/Avg/Max/P50/P95/P99）
   - 总吞吐量
   - 成功率

4. **生成报告**
   - 文本报告: `logs/system_test_<ads>_<timestamp>/metrics.txt`
   - JSON数据: `logs/system_test_<ads>_<timestamp>/metrics.json`

5. **关闭系统**
   - 自动停止所有 Manager 和 Storager 进程

---

## 📊 性能对比测试

运行 ADS层 vs 系统级 对比测试：

```bash
# 对比脚本（自动运行两种测试）
./scripts/compare_ads_vs_system.sh

# 指定参数
./scripts/compare_ads_vs_system.sh data/workload_small_1000.csv mpt
```

**对比分析**：
- **ADS层延迟**: 纯数据结构操作时间
- **系统级延迟**: 完整端到端时间
- **网络开销**: 系统延迟 - ADS延迟

---

## 📝 输出示例

### 终端输出

```
╔═══════════════════════════════════════════════════════════════════╗
║                                                                   ║
║           System-Level Performance Benchmark                      ║
║                                                                   ║
╚═══════════════════════════════════════════════════════════════════╝

📋 Test Configuration:
  Workload: data/workload_small_1000.csv
  ADS Mode: MPT
  Storager Nodes: 3

🚀 Starting distributed storage system...

📦 Starting Storager nodes...
  ✓ Storager 1 started on port 50052
  ✓ Storager 2 started on port 50053
  ✓ Storager 3 started on port 50054

🎯 Starting Manager node...
  ✓ Manager started on port 50051

═══════════════════════════════════════════════════════════════
  SYSTEM INFORMATION
═══════════════════════════════════════════════════════════════
  Manager:   http://[::1]:50051
  Storagers: 3 nodes
    - Storager 1: http://[::1]:50052
    - Storager 2: http://[::1]:50053
    - Storager 3: http://[::1]:50054
═══════════════════════════════════════════════════════════════

📊 Running system-level benchmark...

▶️  Executing 1000 file operations...

  [20%] Processed 200/1000 files
  [40%] Processed 400/1000 files
  [60%] Processed 600/1000 files
  [80%] Processed 800/1000 files
  [100%] Processed 1000/1000 files

✅ System test completed!

═══════════════════════════════════════════════════════════════
  SYSTEM BENCHMARK SUMMARY
═══════════════════════════════════════════════════════════════

📈 Operation Statistics:
  Add:    1000 operations
  Query:  1000 operations
  Update: 800 operations
  Delete: 1000 operations

✅ Success/Failure:
  Total:   3800 operations
  Success: 3800
  Failure: 0
  Success Rate: 100.00%

⏱️  End-to-End Latency:
  Min:    2.345 ms
  Avg:    5.678 ms
  Max:    15.234 ms
  P50:    4.567 ms
  P95:    10.234 ms
  P99:    12.456 ms

🚀 Throughput:
  Total Duration: 25.67s
  Throughput: 148.05 ops/sec

═══════════════════════════════════════════════════════════════
```

### 报告文件

**文本报告** (`logs/system_test_mpt_20241204_123456/metrics.txt`):
```markdown
# System Benchmark Report - 2024-12-04 12:34:56 (ADS: MPT)

## Overview

- Total Operations: 3800
- Success Count: 3800
- Failure Count: 0
- Success Rate: 100.00%
- Total Duration: 25.67s
- Throughput: 148.05 ops/sec

## Operation Statistics

| Operation | Count |
| --------- | ----- |
| Add       | 1000  |
| Query     | 1000  |
| Update    | 800   |
| Delete    | 1000  |

...
```

**JSON数据** (`logs/system_test_mpt_20241204_123456/metrics.json`):
```json
{
  "operation_stats": {
    "add_count": 1000,
    "query_count": 1000,
    "update_count": 800,
    "delete_count": 1000
  },
  "end_to_end_latency": {
    "min_ms": 2.345,
    "max_ms": 15.234,
    "avg_ms": 5.678,
    "p50_ms": 4.567,
    "p95_ms": 10.234,
    "p99_ms": 12.456
  },
  ...
}
```

---

## 🔧 故障排除

### 端口冲突

如果遇到 "Address already in use" 错误：

```bash
# 检查占用端口的进程
lsof -i :50051
lsof -i :50052

# 杀死进程
kill -9 <PID>
```

### 进程未正常关闭

测试框架会自动清理，但如果进程残留：

```bash
# 查找并杀死所有 manager 和 storager 进程
pkill -f "manager"
pkill -f "storager"
```

### 测试超时

大规模数据集可能需要较长时间：
- `workload_small_1000.csv`: ~30秒
- `workload_medium_10000.csv`: ~5分钟
- `workload_large_100000.csv`: ~30-60分钟

---

## 🎯 性能分析建议

### 1. 对比 ADS 层 vs 系统级性能

```bash
# 运行对比测试
./scripts/compare_ads_vs_system.sh data/workload_small_1000.csv mpt

# 分析网络开销
网络开销 = 系统级延迟 - ADS层延迟
```

**预期结果**：
- gRPC序列化/反序列化: ~0.5-1ms
- 网络传输（本地）: ~0.1-0.5ms
- Manager路由查找: ~0.1-0.3ms
- 证明验证: ~1-5ms（取决于ADS类型）

**总开销**: 约 2-7ms

### 2. 扩展性测试

测试不同 Storager 数量的影响：

```bash
# 3 个 Storager
./target/release/system_benchmark data/workload_small_1000.csv mpt 3

# 5 个 Storager
./target/release/system_benchmark data/workload_small_1000.csv mpt 5

# 10 个 Storager
./target/release/system_benchmark data/workload_small_1000.csv mpt 10
```

**分析点**：
- 吞吐量是否随节点数线性增长？
- 一致性哈希负载是否均衡？
- Manager是否成为瓶颈？

### 3. ADS 类型对比

```bash
# 测试三种 ADS
./target/release/system_benchmark data/workload_small_1000.csv mpt 3
./target/release/system_benchmark data/workload_small_1000.csv mest 3
./target/release/system_benchmark data/workload_small_1000.csv acctrie 3
```

**对比维度**：
- 端到端延迟
- 证明验证开销
- 网络传输大小（证明大小）
- 系统吞吐量

---

## 📚 相关文档

- [ADS层测试文档](../docs/BENCHMARK_ANALYSIS.md)
- [系统架构文档](../README.md)
- [性能优化建议](../docs/BENCHMARK_ANALYSIS.md#五性能优化建议)

---

**注意**: 系统级测试会启动多个进程，确保系统有足够资源（至少 2GB 可用内存）。
