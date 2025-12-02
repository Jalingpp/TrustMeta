# 运维脚本

本目录包含系统运维和测试相关的脚本。

## 📜 脚本列表

### 系统管理

- `start_system.sh` - 启动分布式系统（Manager + Storager 节点）
- `stop.sh` - 停止所有系统进程
- `test_system.sh` - 系统集成测试

### 性能测试

- `run_workload_test.sh` - 运行工作负载测试
- `compare_performance.sh` - 对比不同 ADS 的性能
- `compare_operations.sh` - 对比单个操作性能

### 数据生成

- `generate_workload_data.py` - 生成测试工作负载数据

## 🚀 使用方法

### 启动系统

```bash
# 使用 MPT
./scripts/start_system.sh

# 使用 MEST
./scripts/start_system.sh mest

# 使用 AccTrie
./scripts/start_system.sh acctrie
```

### 性能测试

```bash
# 运行完整性能测试
./scripts/compare_performance.sh

# 测试特定工作负载
./scripts/run_workload_test.sh data/workload_medium_10000.csv
```

### 生成测试数据

```bash
cd scripts
python generate_workload_data.py --size 10000 --output ../data/custom_workload.csv
```

### 停止系统

```bash
./scripts/stop.sh
```

## ⚙️ 配置

脚本使用 `config.json` 中的配置，可根据需要修改：

```json
{
  "manager_addr": "127.0.0.1:50051",
  "storagers": [...],
  "ads_mode": "mpt"
}
```

## 📊 输出

- 日志文件：`logs/` 目录
- 性能报告：`docs/reports/` 目录
- 测试数据：`data/` 目录

## ⚠️ 注意

- 运行脚本前确保已构建项目：`cargo build --release`
- 确保端口未被占用（50051-50054）
- Python 脚本需要 Python 3.6+
