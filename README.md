# 分布式存储系统 (Distributed Storage System)

一个基于 Rust 实现的高性能分布式存储系统，支持多种认证数据结构（ADS）。

## 🚀 特性

- **多种 ADS 支持**：MPT (Merkle Patricia Trie)、MEST (Merkle-based Extendible Segmented Hash Tree)、AccTrie (Accumulator-based Trie)
- **一致性哈希**：支持动态节点添加/删除，最小化数据迁移
- **高性能**：Rust 实现，线程安全，零成本抽象
- **可验证性**：支持密码学证明和验证
- **分布式架构**：Manager-Storager 架构，支持水平扩展

## 📁 项目结构

```
distributed-storage-system/
├── crates/
│   ├── client/          # 客户端实现
│   ├── manager/         # 管理节点（包含一致性哈希）
│   ├── storager/        # 存储节点
│   │   └── ads_lib/     # ADS 库（MPT、MEST、AccTrie）
│   ├── common/          # 共享类型和工具
│   └── system/          # 系统集成
├── docs/                # 文档
│   ├── requirements/    # 需求文档
│   └── reports/         # 性能报告
├── data/                # 测试数据
├── logs/                # 日志文件
├── proto/               # gRPC 协议定义
└── scripts/             # 运行脚本
```

## 🛠️ 构建

```bash
# 构建整个项目
cargo build --release

# 运行测试
cargo test --workspace

# 构建特定组件
cargo build --package manager --release
cargo build --package storager --release
cargo build --package client --release
```

## 🎯 快速开始

### 1. 启动系统

```bash
# 使用脚本启动
./scripts/start_system.sh

# 或手动启动
cargo run --package manager --release -- --ads-mode mpt &
cargo run --package storager --release &
```

### 2. 运行客户端

```bash
cargo run --package client --release
```

### 3. 运行性能测试

```bash
./scripts/run_workload_test.sh
./scripts/compare_performance.sh
```

## 📊 认证数据结构

### MPT (Merkle Patricia Trie)
- **特点**：类 Ethereum 实现，持久化存储
- **适用场景**：需要历史状态查询的场景
- **性能**：中等，支持 RocksDB 持久化

### MEST (Merkle-based Extendible Segmented Hash Tree)
- **特点**：高吞吐量，动态扩展
- **适用场景**：高并发写入场景
- **性能**：高，内存优化

### AccTrie (Accumulator-based Trie)
- **特点**：基于密码学累加器，强验证保证
- **适用场景**：需要密码学证明的场景
- **性能**：中等，BLS12-381 曲线

## 🔧 配置

编辑 `config.json` 配置系统参数：

```json
{
  "manager_addr": "127.0.0.1:50051",
  "storagers": [
    {"id": "storager1", "addr": "127.0.0.1:50052"},
    {"id": "storager2", "addr": "127.0.0.1:50053"},
    {"id": "storager3", "addr": "127.0.0.1:50054"}
  ],
  "ads_mode": "mpt",
  "virtual_nodes": 150
}
```

## 📈 性能

查看性能报告：`docs/reports/operation_performance_comparison_report.md`

## 🧪 测试

```bash
# 所有测试
cargo test --workspace

# 特定包测试
cargo test --package esa_rust
cargo test --package manager
cargo test --package storager

# 集成测试
cargo test --test acctrie_integration_test
```

## 📝 开发

### 添加新的 ADS

1. 在 `crates/storager/ads_lib/src/` 下创建新模块
2. 实现 `AdsOperations` trait
3. 在 `common/src/types.rs` 中添加枚举值
4. 在 manager 中添加验证逻辑

### 代码规范

```bash
# 格式化代码
cargo fmt --all

# 检查代码
cargo clippy --all -- -D warnings

# 构建文档
cargo doc --no-deps --open
```

## 📄 License

MIT License

## 👥 作者

kazmiller0

---

**注意**：本项目为研究原型，生产环境使用需进一步优化和测试。
