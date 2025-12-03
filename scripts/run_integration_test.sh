#!/bin/bash

# 分布式存储系统集成测试脚本
# 自动启动系统并运行测试

set -e  # 遇到错误立即退出

echo "╔════════════════════════════════════════════════════════════╗"
echo "║          分布式存储系统集成测试                               ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# 项目根目录
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

echo "📁 项目目录: $PROJECT_ROOT"
echo ""

# 配置
MANAGER_PORT=50051
STORAGER1_PORT=50052
STORAGER2_PORT=50053
STORAGER3_PORT=50054

# 数据集选择
DATASET_SIZE=${1:-small}  # small, medium, large
case $DATASET_SIZE in
    small)
        DATASET="data/workload_small_1000.csv"
        ;;
    medium)
        DATASET="data/workload_medium_10000.csv"
        ;;
    large)
        DATASET="data/workload_large_100000.csv"
        ;;
    *)
        echo -e "${RED}❌ 无效的数据集大小: $DATASET_SIZE${NC}"
        echo "用法: $0 [small|medium|large]"
        exit 1
        ;;
esac

# ADS类型选择
ADS_TYPE=${2:-mest}  # mest, mpt, acctrie
echo "⚙️  配置:"
echo "  - 数据集: $DATASET"
echo "  - ADS类型: $ADS_TYPE"
echo "  - Manager端口: $MANAGER_PORT"
echo "  - Storager端口: $STORAGER1_PORT, $STORAGER2_PORT, $STORAGER3_PORT"
echo ""

# 清理函数
cleanup() {
    echo -e "\n${YELLOW}🧹 清理进程...${NC}"
    pkill -f "target/release/manager" || true
    pkill -f "target/release/storager" || true
    sleep 1
    echo -e "${GREEN}✅ 清理完成${NC}"
}

# 设置退出时清理
trap cleanup EXIT INT TERM

# 检查数据集文件
if [ ! -f "$DATASET" ]; then
    echo -e "${RED}❌ 数据集文件不存在: $DATASET${NC}"
    exit 1
fi

# 编译项目
echo -e "${YELLOW}🔨 编译项目...${NC}"
cargo build --release --bin manager --bin storager 2>&1 | grep -E "Compiling|Finished" || true
cargo build --release --example system_integration_test 2>&1 | grep -E "Compiling|Finished" || true
echo -e "${GREEN}✅ 编译完成${NC}"
echo ""

# 清理旧进程
echo -e "${YELLOW}🧹 清理旧进程...${NC}"
pkill -f "target/release/manager" || true
pkill -f "target/release/storager" || true
sleep 1
echo ""

# 启动 Manager
echo -e "${GREEN}🚀 启动 Manager (端口 $MANAGER_PORT)...${NC}"
RUST_LOG=info ./target/release/manager \
    --port $MANAGER_PORT \
    --ads-mode $ADS_TYPE \
    --storagers "http://[::1]:$STORAGER1_PORT,http://[::1]:$STORAGER2_PORT,http://[::1]:$STORAGER3_PORT" \
    > logs/manager.log 2>&1 &
MANAGER_PID=$!
echo "  PID: $MANAGER_PID"

# 等待 Manager 启动
sleep 2

# 启动 Storager 节点
echo -e "${GREEN}🚀 启动 Storager 节点...${NC}"

# Storager 1
echo "  启动 Storager-1 (端口 $STORAGER1_PORT, ADS: $ADS_TYPE)..."
RUST_LOG=info ./target/release/storager $STORAGER1_PORT $ADS_TYPE \
    > logs/storager1.log 2>&1 &
STORAGER1_PID=$!
echo "    PID: $STORAGER1_PID"

# Storager 2
echo "  启动 Storager-2 (端口 $STORAGER2_PORT, ADS: $ADS_TYPE)..."
RUST_LOG=info ./target/release/storager $STORAGER2_PORT $ADS_TYPE \
    > logs/storager2.log 2>&1 &
STORAGER2_PID=$!
echo "    PID: $STORAGER2_PID"

# Storager 3
echo "  启动 Storager-3 (端口 $STORAGER3_PORT, ADS: $ADS_TYPE)..."
RUST_LOG=info ./target/release/storager $STORAGER3_PORT $ADS_TYPE \
    > logs/storager3.log 2>&1 &
STORAGER3_PID=$!
echo "    PID: $STORAGER3_PID"

# 等待所有服务启动
echo ""
echo -e "${YELLOW}⏳ 等待服务启动...${NC}"
sleep 3

# 检查进程是否运行
check_process() {
    if ! ps -p $1 > /dev/null 2>&1; then
        echo -e "${RED}❌ 进程 $2 (PID: $1) 未运行${NC}"
        return 1
    fi
    return 0
}

if ! check_process $MANAGER_PID "Manager"; then
    echo "查看日志: logs/manager.log"
    exit 1
fi

if ! check_process $STORAGER1_PID "Storager-1"; then
    echo "查看日志: logs/storager1.log"
    exit 1
fi

if ! check_process $STORAGER2_PID "Storager-2"; then
    echo "查看日志: logs/storager2.log"
    exit 1
fi

if ! check_process $STORAGER3_PID "Storager-3"; then
    echo "查看日志: logs/storager3.log"
    exit 1
fi

echo -e "${GREEN}✅ 所有服务已启动${NC}"
echo ""

# 运行集成测试
echo -e "${GREEN}🧪 运行集成测试...${NC}"
echo ""

MANAGER_ADDR="http://[::1]:$MANAGER_PORT" \
DATASET_PATH="$DATASET" \
./target/release/examples/system_integration_test

TEST_EXIT_CODE=$?

echo ""

# 显示进程状态
echo "📊 进程状态:"
echo "  Manager (PID $MANAGER_PID): $(ps -p $MANAGER_PID > /dev/null 2>&1 && echo '运行中' || echo '已停止')"
echo "  Storager-1 (PID $STORAGER1_PID): $(ps -p $STORAGER1_PID > /dev/null 2>&1 && echo '运行中' || echo '已停止')"
echo "  Storager-2 (PID $STORAGER2_PID): $(ps -p $STORAGER2_PID > /dev/null 2>&1 && echo '运行中' || echo '已停止')"
echo "  Storager-3 (PID $STORAGER3_PID): $(ps -p $STORAGER3_PID > /dev/null 2>&1 && echo '运行中' || echo '已停止')"

# 日志位置
echo ""
echo "📝 日志文件:"
echo "  Manager:    logs/manager.log"
echo "  Storager-1: logs/storager1.log"
echo "  Storager-2: logs/storager2.log"
echo "  Storager-3: logs/storager3.log"

# 测试结果
echo ""
if [ $TEST_EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║  ✅ 测试成功完成!                                           ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
else
    echo -e "${RED}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║  ❌ 测试失败                                                ║${NC}"
    echo -e "${RED}╚════════════════════════════════════════════════════════════╝${NC}"
fi

# 退出代码
exit $TEST_EXIT_CODE
