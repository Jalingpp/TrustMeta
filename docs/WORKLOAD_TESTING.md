# Workload 测试指南

本文档介绍分布式存储系统的 Workload 测试数据集和测试方法。

## 📊 数据集

### 数据生成工具

使用 `scripts/generate_workload_data.py` 生成真实场景的测试数据集:

```bash
# 生成小型数据集 (1,000 条记录)
python3 scripts/generate_workload_data.py --size small

# 生成中型数据集 (10,000 条记录)
python3 scripts/generate_workload_data.py --size medium

# 生成大型数据集 (100,000 条记录)
python3 scripts/generate_workload_data.py --size large

# 自定义数据集大小
python3 scripts/generate_workload_data.py --size custom --custom-size 5000 --output data/my_dataset.csv
```

### 数据特征

生成的数据集模拟真实场景,包含:

- **10 个主要类别**:
  - Technology (15%)
  - Business (12%)
  - Science (10%)
  - Education (12%)
  - Health (10%)
  - Entertainment (8%)
  - Sports (8%)
  - News (10%)
  - Lifestyle (8%)
  - Environment (7%)

- **关键词分布**:
  - 每条记录 2-7 个关键词(Zipf 分布)
  - 类别关键词 + 特定领域关键词
  - 20% 概率包含热门关键词(模拟热点数据)

- **数据格式** (CSV):
  ```
  fid,keyword1,keyword2,keyword3,...
  ```

### 数据统计

每次生成数据集都会同时生成统计文件 `*_stats.json`:

```json
{
  "total_records": 10000,
  "categories": 10,
  "unique_keywords": 134,
  "category_distribution": { ... },
  "top_keywords": [ ... ]
}
```

## 🧪 测试方法

### 方法 1: 使用真实数据集测试

推荐使用 `workload_realistic.rs`,从真实数据文件加载数据:

```bash
# 使用默认数据集 (data/workload_small_1000.csv)
cargo run --release --example workload_realistic

# 指定数据集
cargo run --release --example workload_realistic data/workload_medium_10000.csv
```

**测试内容**:

1. **Workload 1: 批量插入** - 测试写入吞吐量
2. **Workload 2: 随机关键词查询** - 测试点查询性能
3. **Workload 3: 类别扫描** - 测试范围查询性能
4. **Workload 4: 热点访问** - 测试 80/20 访问模式
5. **Workload 5: 混合负载** - 测试读写混合场景 (70% 读, 30% 写)
6. **Workload 6: 复杂布尔查询** - 测试 AND/OR/NOT/嵌套查询
7. **Workload 7: 更新负载** - 测试更新性能

### 方法 2: 使用随机生成数据测试

使用 `workload_test.rs`,运行时动态生成测试数据:

```bash
cargo run --release --example workload_test
```

**测试内容**:

1. **写密集型** (90% 写, 10% 读) - 1000 操作
2. **读密集型** (10% 写, 90% 读) - 1000 操作
3. **均衡负载** (50% 写, 50% 读) - 1000 操作
4. **突发写入** (5 个突发,每个 100 条) - 500 操作
5. **扫描负载** (10 次类别扫描)
6. **更新密集型** (500 次更新)
7. **复杂查询** (200 个布尔查询)

### 方法 3: 热点数据单独测试

```bash
cargo run --release --example workload_hotspot
```

测试热点数据访问模式 (200 条记录 + 500 次查询,80/20 规则)。

## 📈 性能基准

基于中型数据集 (10,000 条记录) 的典型性能:

| Workload | 吞吐量 | 延迟 | 备注 |
|----------|--------|------|------|
| 批量插入 | 300-400 ops/s | - | 受网络和磁盘 I/O 限制 |
| 随机读取 | 500-2000 QPS | - | 缓存命中率影响大 |
| 类别扫描 | 400-1000 QPS | - | 依赖索引效率 |
| 热点访问 | 600-1500 QPS | - | 缓存效果显著 |
| 混合负载 | 300-500 ops/s | - | 读写竞争 |
| 复杂查询 | 400-800 QPS | - | 查询复杂度影响 |
| 更新负载 | 300-600 ops/s | - | 需要读-改-写 |

## 🔧 调优建议

### 连接池配置

系统已实现连接池,但在长时间运行下仍可能遇到连接耗尽:

- 单个 workload 测试:无问题
- 连续多个 workload:可能在 3000-4000 操作后耗尽
- 建议:测试间增加延迟或分批运行

### 数据集选择

| 场景 | 推荐数据集 | 理由 |
|------|-----------|------|
| 快速验证 | small (1K) | 快速完成,适合开发调试 |
| 性能测试 | medium (10K) | 平衡性能和时间 |
| 压力测试 | large (100K) | 全面测试系统极限 |
| CI/CD | small (1K) | 快速反馈 |

### 测试参数调整

在 `workload_realistic.rs` 中可调整:

```rust
// 查询次数
let num_queries = 500;  // 可增减

// 延迟控制
if i % 50 == 0 && i > 0 {
    sleep(Duration::from_millis(10)).await;  // 可调整
}

// 混合负载比例
let read_ratio = 0.7;  // 可改为 0.5, 0.8 等
```

## 📝 自定义 Workload

### 创建新的测试模式

1. 复制 `workload_realistic.rs` 并重命名
2. 修改 `main()` 函数,添加新的测试函数
3. 实现测试逻辑,参考现有 workload

示例:

```rust
async fn run_my_workload(manager_addr: &str, dataset: &[Record]) 
    -> Result<(), Box<dyn std::error::Error>> 
{
    // 你的测试逻辑
    let start = Instant::now();
    
    // ... 执行操作 ...
    
    let duration = start.elapsed();
    println!("测试完成: {:?}", duration);
    Ok(())
}
```

### 生成特定领域数据

修改 `generate_workload_data.py` 的 `CATEGORIES` 字典:

```python
CATEGORIES = {
    "your_domain": {
        "keywords": ["keyword1", "keyword2", ...],
        "weight": 0.5  # 50% 的记录属于这个类别
    },
    # ...
}
```

## 🐛 故障排查

### 问题: 连接超时

**症状**: `Resource temporarily unavailable` (errno 35)

**原因**: 连接池耗尽或系统文件描述符限制

**解决**:
1. 增加操作间延迟
2. 减少并发连接数
3. 检查系统限制: `ulimit -n`
4. 分批运行测试

### 问题: 数据加载失败

**症状**: `No such file or directory`

**解决**:
1. 确认数据文件路径正确
2. 先运行数据生成脚本
3. 使用绝对路径

### 问题: 性能低于预期

**原因排查**:
1. 检查网络延迟
2. 查看服务器负载
3. 检查磁盘 I/O
4. 确认是否启用 `--release` 编译

## 📚 相关文档

- [系统架构](../README.md)
- [性能测试](performance_test.rs)
- [布尔查询测试](boolean_query_test.rs)
