#!/bin/bash
# 快速测试 MEST ADS 功能

echo "=== 测试 MEST ADS 集成 ==="

cd /Users/kazmiller/Project/distributed-storage-system

# 1. 编译项目
echo "1. 编译项目..."
cargo build --package storager --lib --quiet

if [ $? -eq 0 ]; then
    echo "✅ 编译成功"
else
    echo "❌ 编译失败"
    exit 1
fi

# 2. 运行 MEST ADS 单元测试
echo ""
echo "2. 运行 MEST ADS 单元测试..."
cargo test --package storager --lib mest_ads::tests --quiet -- --nocapture

if [ $? -eq 0 ]; then
    echo "✅ 所有测试通过"
else
    echo "❌ 测试失败"
    exit 1
fi

echo ""
echo "=== MEST ADS 集成测试完成 ==="
