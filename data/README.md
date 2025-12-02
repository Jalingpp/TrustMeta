# 测试数据

本目录存放系统性能测试所需的工作负载数据。

## 📊 数据文件

- `workload_small_1000.csv` - 小规模测试（1000 条记录）
- `workload_small_1000_stats.json` - 小规模测试统计
- `workload_medium_10000.csv` - 中等规模测试（10000 条记录）
- `workload_medium_10000_stats.json` - 中等规模测试统计
- `workload_large_100000.csv` - 大规模测试（100000 条记录）
- `workload_large_100000_stats.json` - 大规模测试统计

## 🔧 生成数据

使用脚本生成新的测试数据：

```bash
cd scripts
python generate_workload_data.py --size 1000 --output ../data/workload_custom.csv
```

## 📈 数据格式

CSV 格式：
```csv
operation,fid,key,value
add,file001,user123,data_content
query,file001,user123,
delete,file001,user123,
```

统计信息 JSON 格式：
```json
{
  "total_operations": 1000,
  "add_count": 400,
  "query_count": 400,
  "delete_count": 200,
  "unique_files": 100,
  "unique_keys": 500
}
```

## ⚠️ 注意

- 数据文件不会被提交到 Git（已在 .gitignore 中排除）
- 如需测试数据，请运行生成脚本
