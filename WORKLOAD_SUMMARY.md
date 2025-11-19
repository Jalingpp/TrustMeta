# Workload 测试数据集 - 快速参考

## ✅ 已完成

### 数据生成工具
- ✅ `scripts/generate_workload_data.py` - Python 数据生成器
  - 支持 small (1K), medium (10K), large (100K) 三种预设规模
  - 支持自定义规模和输出路径
  - 自动生成统计信息 JSON 文件

### 生成的数据集
- ✅ `data/workload_small_1000.csv` - 1,000 条记录
- ✅ `data/workload_small_1000_stats.json` - 统计信息
- ✅ `data/workload_medium_10000.csv` - 10,000 条记录
- ✅ `data/workload_medium_10000_stats.json` - 统计信息

### 测试程序
- ✅ `crates/client/examples/workload_realistic.rs` - 真实数据测试 (7 个 workload)
- ✅ `crates/client/examples/workload_test.rs` - 随机数据测试 (7 个 workload)
- ✅ `crates/client/examples/workload_hotspot.rs` - 热点访问测试

### 自动化脚本
- ✅ `scripts/run_workload_test.sh` - 一键测试脚本
  - 自动检查/生成数据集
  - 自动编译测试程序
  - 自动检查系统状态
  - 支持三种规模选择

### 文档
- ✅ `docs/WORKLOAD_TESTING.md` - 完整测试指南
- ✅ `data/README.md` - 数据集使用说明

## 🚀 使用方法

### 快速开始
```bash
# 1. 生成数据集
python3 scripts/generate_workload_data.py --size small
python3 scripts/generate_workload_data.py --size medium

# 2. 启动系统
./scripts/start.sh

# 3. 运行测试
./scripts/run_workload_test.sh         # 小型数据集
./scripts/run_workload_test.sh medium  # 中型数据集
```

### 手动运行
```bash
# 真实数据测试
cargo run --release --example workload_realistic data/workload_small_1000.csv

# 随机数据测试
cargo run --release --example workload_test

# 热点访问测试
cargo run --release --example workload_hotspot
```

## 📊 数据集特征

### 10 个类别
- Technology (15%)
- Business (12%)
- Education (12%)
- Health (10%)
- Science (10%)
- News (10%)
- Entertainment (8%)
- Sports (8%)
- Lifestyle (8%)
- Environment (7%)

### 关键词分布
- ~134 个唯一关键词
- 每条记录 2-7 个关键词 (Zipf 分布)
- 20% 概率包含热门关键词

### 数据格式
```csv
fid,keyword1,keyword2,keyword3,...
```

## 🧪 测试内容

### workload_realistic.rs (7 个测试)
1. **批量插入** - 全量数据插入
2. **随机查询** - 500 次关键词查询
3. **类别扫描** - 扫描所有类别
4. **热点访问** - 80/20 规则,300 次查询
5. **混合负载** - 70% 读 + 30% 写,500 次操作
6. **复杂查询** - 100 次布尔查询 (AND/OR/NOT/嵌套)
7. **更新负载** - 200 次更新操作

### workload_test.rs (7 个测试)
1. 写密集型 (90/10) - 1000 操作
2. 读密集型 (10/90) - 1000 操作
3. 均衡负载 (50/50) - 1000 操作
4. 突发写入 - 5×100 条记录
5. 扫描负载 - 10 次类别扫描
6. 更新密集型 - 500 次更新
7. 复杂查询 - 200 个布尔查询

## 📈 性能基准 (10K 数据集)

| Workload | 吞吐量        |
| -------- | ------------- |
| 批量插入 | 300-400 ops/s |
| 随机读取 | 500-2000 QPS  |
| 类别扫描 | 400-1000 QPS  |
| 热点访问 | 600-1500 QPS  |
| 混合负载 | 300-500 ops/s |
| 复杂查询 | 400-800 QPS   |
| 更新负载 | 300-600 ops/s |

## 📝 文件清单

```
distributed-storage-system/
├── data/
│   ├── README.md                        # 数据集说明文档 ✅
│   ├── workload_small_1000.csv          # 小型数据集 ✅
│   ├── workload_small_1000_stats.json   # 小型统计 ✅
│   ├── workload_medium_10000.csv        # 中型数据集 ✅
│   └── workload_medium_10000_stats.json # 中型统计 ✅
├── docs/
│   └── WORKLOAD_TESTING.md              # 测试指南 ✅
├── scripts/
│   ├── generate_workload_data.py        # 数据生成器 ✅
│   └── run_workload_test.sh             # 测试启动脚本 ✅
└── crates/client/examples/
    ├── workload_realistic.rs            # 真实数据测试 ✅
    ├── workload_test.rs                 # 随机数据测试 ✅
    └── workload_hotspot.rs              # 热点访问测试 ✅
```

## 🎯 下一步

现在你可以:

1. **验证测试**: 运行 `./scripts/run_workload_test.sh` 确保一切正常
2. **性能分析**: 使用中型数据集进行完整性能测试
3. **压力测试**: 生成大型数据集 (100K) 测试系统极限
4. **自定义测试**: 修改 workload 参数或创建新的测试模式

## 📚 参考文档

- [完整测试指南](docs/WORKLOAD_TESTING.md)
- [数据集说明](data/README.md)
- [项目主页](README.md)

---

**生成日期**: 2025-11-19
**数据集版本**: v1.0
**测试套件版本**: v1.0
