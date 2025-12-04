# 系统级性能测试 (System Benchmark)

## ✨ 新功能

现在支持**完整系统架构的端到端性能测试**！

与之前的 ADS 层测试不同，系统级测试会：
- ✅ 自动启动 Manager 和多个 Storager 进程
- ✅ 通过 Client 发送真实的 gRPC 请求
- ✅ 测量包含网络通信、路由、证明验证的完整延迟
- ✅ 评估真实场景下的系统性能

---

## 🚀 快速运行

```bash
# 方法 1: 使用脚本（推荐）
./scripts/run_system_benchmark.sh

# 方法 2: 直接运行
cargo run --release --bin system_benchmark

# 方法 3: 对比 ADS层 vs 系统级性能
./scripts/compare_ads_vs_system.sh
```

---

## 📊 测试架构对比

### ADS 层测试 (`benchmark`)
```
Benchmark → ADS (MPT/MEST/AccTrie)
```
- **测试范围**: 仅数据结构操作
- **延迟组成**: 纯算法时间
- **用途**: 评估 ADS 算法性能

### 系统级测试 (`system_benchmark`)  
```
Client → gRPC → Manager → gRPC → Storager(s)
                   ↓
           一致性哈希路由
           证明验证
```
- **测试范围**: 完整分布式系统
- **延迟组成**: 网络 + 序列化 + 路由 + 验证 + ADS操作
- **用途**: 评估真实部署性能

---

## 📈 示例输出

```
🚀 Starting distributed storage system...
  ✓ Storager 1 started on port 50052
  ✓ Storager 2 started on port 50053
  ✓ Storager 3 started on port 50054
  ✓ Manager started on port 50051

📊 Running system-level benchmark...
  [100%] Processed 1000/1000 files

═══════════════════════════════════════════════════════════
  SYSTEM BENCHMARK SUMMARY
═══════════════════════════════════════════════════════════

⏱️  End-to-End Latency:
  Avg:    5.678 ms
  P50:    4.567 ms
  P95:    10.234 ms

🚀 Throughput: 148.05 ops/sec

📊 Reports saved to: logs/system_test_mpt_<timestamp>/
```

---

## 📚 详细文档

查看完整使用指南: [docs/SYSTEM_BENCHMARK_GUIDE.md](./SYSTEM_BENCHMARK_GUIDE.md)

---

## 🎯 性能分析场景

### 1. 网络开销分析
对比 ADS 层测试和系统级测试的延迟差异，计算网络和协议开销。

### 2. 扩展性测试
测试不同 Storager 节点数量对性能的影响。

### 3. ADS 类型选型
在真实分布式环境下对比三种 ADS 的性能表现。

---

**立即尝试**: `./scripts/run_system_benchmark.sh` 🚀
