# 系统日志

本目录存放系统运行时产生的日志文件。

## 📝 日志文件

- `manager.log` - Manager 节点日志
- `storager*.log` - Storager 节点日志
- `operation_comparison_*.log` - 操作对比测试日志
- `start_system_*.log` - 系统启动测试日志

## 🔍 查看日志

```bash
# 实时查看 manager 日志
tail -f logs/manager.log

# 查看最近 100 行
tail -n 100 logs/storager1.log

# 搜索错误
grep "ERROR" logs/*.log

# 搜索特定操作
grep "add operation" logs/manager.log
```

## 🧹 清理日志

```bash
# 清理所有日志
rm -f logs/*.log

# 清理旧日志（保留最新的）
find logs/ -name "*.log" -mtime +7 -delete
```

## ⚠️ 注意

- 日志文件不会被提交到 Git（已在 .gitignore 中排除）
- 建议定期清理旧日志以节省磁盘空间
- 生产环境建议配置日志轮转
