# 运维脚本文档

本目录包含分布式存储系统的运维、测试和性能对比相关脚本。

## 📜 脚本列表

### �� 系统管理

| 脚本 | 功能说明 | 使用场景 |
|------|---------|----------|
| `start_system.sh` | 启动分布式系统 (Manager + Storagers) | 开发、测试环境启动 |
| `stop.sh` | 停止所有系统进程 | 系统维护、重启前清理 |
| `test_system.sh` | 完整的集成测试套件 | CI/CD、版本验证 |

### 📊 性能测试

| 脚本 | 功能说明 | 使用场景 |
|------|---------|----------|
| `run_workload_test.sh` | 运行真实workload测试 | 性能基准测试 |
| `compare_performance.sh` | MPT vs MEST性能对比 | ADS选型、性能分析 |
| `compare_operations.sh` | 单个操作性能对比 | 细粒度性能调优 |

### 🗂️ 数据生成

| 脚本 | 功能说明 |
|------|---------|
| `generate_workload_data.py` | 生成测试数据集 |

---

## 🚀 快速开始

### 启动系统
```bash
# 默认配置 (MEST, 3节点)
./scripts/start_system.sh

# 指定ADS模式
./scripts/start_system.sh -a mpt
./scripts/start_system.sh -a acctrie
```

### 运行测试
```bash
# 快速测试
./scripts/test_system.sh --quick

# 完整测试
./scripts/test_system.sh -a all -s all
```

### 性能对比
```bash
./scripts/compare_performance.sh
./scripts/run_workload_test.sh small
```

### 停止系统
```bash
./scripts/stop.sh
```

---

## 📋 详细说明

### start_system.sh
- `-a, --ads <MODE>`: ADS模式 (mpt|mest|acctrie)
- `-m, --mode <MODE>`: 构建模式 (debug|release)
- `-n, --num <NUM>`: Storager节点数

### test_system.sh
- `-a, --ads <MODES>`: 测试的ADS模式
- `-s, --size <SIZES>`: 数据规模
- `-q, --quick`: 快速测试模式

### run_workload_test.sh
执行7种workload测试：批量插入、随机查询、类别扫描、热点访问、混合负载、布尔查询、更新操作

---

## 📊 性能基准 (1K记录)

| 指标 | MEST | MPT | AccTrie |
|------|------|-----|----------|
| 插入吞吐量 | 408 ops/s | 398 ops/s | 352 ops/s |
| 查询QPS | 1344 | 1304 | 1104 |
| Update成功率 | 100% | 32.5% | 100% |

**推荐**: 🥇 生产环境: MEST | 🥈 读多写少: MPT | 🥉 高安全: AccTrie

---

## 🔍 故障排查

```bash
# 检查端口占用
lsof -i :50051-50054

# 查看日志
tail -f logs/manager.log

# 重新编译
cargo build --release
```

---

## ⚠️ 注意事项

1. 运行前确保已编译: `cargo build --release`
2. 确保端口未被占用 (50051-50054)
3. Python脚本需要 Python 3.7+
4. 大型数据集测试需要8GB+内存
