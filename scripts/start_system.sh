#!/usr/bin/env zsh

# Distributed Storage System - Universal Start Script
# 通用启动脚本 - 支持选择 ADS 类型

set -e  # 遇到错误立即退出

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# 项目根目录
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# 默认参数
ADS_MODE="mest"
BUILD_MODE="release"
NUM_STORAGERS=3

# 显示帮助信息
show_help() {
    echo -e "${CYAN}=== 分布式存储系统启动脚本 ===${NC}"
    echo ""
    echo "用法: $0 [选项]"
    echo ""
    echo "选项:"
    echo "  -a, --ads <MODE>       设置 ADS 模式: mpt|mest (默认: mest)"
    echo "  -m, --mode <MODE>      设置构建模式: debug|release (默认: release)"
    echo "  -n, --num <NUM>        Storager 节点数量 (默认: 3)"
    echo "  -h, --help             显示此帮助信息"
    echo ""
    echo "示例:"
    echo "  $0                          # 使用默认设置 (MEST, Release, 3个节点)"
    echo "  $0 -a mpt                   # 使用 MPT 模式"
    echo "  $0 -a mest -m debug         # 使用 MEST 模式,调试构建"
    echo "  $0 -n 5                     # 启动 5 个 Storager 节点"
    exit 0
}

# 解析命令行参数
while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--ads)
            ADS_MODE="$2"
            shift 2
            ;;
        -m|--mode)
            BUILD_MODE="$2"
            shift 2
            ;;
        -n|--num)
            NUM_STORAGERS="$2"
            shift 2
            ;;
        -h|--help)
            show_help
            ;;
        *)
            echo -e "${RED}错误: 未知选项 $1${NC}"
            show_help
            ;;
    esac
done

# 验证 ADS 模式
case $ADS_MODE in
    mpt|mest)
        ;;
    *)
        echo -e "${RED}错误: 无效的 ADS 模式 '$ADS_MODE'${NC}"
        echo -e "支持的模式: mpt, mest"
        exit 1
        ;;
esac

# 验证构建模式
case $BUILD_MODE in
    debug|release)
        ;;
    *)
        echo -e "${RED}错误: 无效的构建模式 '$BUILD_MODE'${NC}"
        echo -e "支持的模式: debug, release"
        exit 1
        ;;
esac

# 显示配置信息
echo -e "${CYAN}=== 分布式存储系统启动脚本 ===${NC}"
echo ""
echo -e "${BLUE}配置信息:${NC}"
echo -e "  ADS 模式:     ${GREEN}${(U)ADS_MODE}${NC}"
echo -e "  构建模式:     ${GREEN}${(U)BUILD_MODE}${NC}"
echo -e "  Storager 数:  ${GREEN}${NUM_STORAGERS}${NC}"
echo ""

# 创建日志目录
mkdir -p logs

# 检查并清理旧进程
echo -e "${YELLOW}[1/4] 清理旧进程...${NC}"
pkill -f "target/${BUILD_MODE}/manager" 2>/dev/null && echo "  - 已停止旧的 Manager 进程" || true
pkill -f "target/${BUILD_MODE}/storager" 2>/dev/null && echo "  - 已停止旧的 Storager 进程" || true
sleep 1

# 编译项目
echo -e "${YELLOW}[2/4] 编译项目 (${BUILD_MODE} 模式)...${NC}"
if [ "$BUILD_MODE" = "release" ]; then
    cargo build --release --quiet
else
    cargo build --quiet
fi

if [ $? -eq 0 ]; then
    echo -e "  ${GREEN}✓ 编译成功${NC}"
else
    echo -e "  ${RED}✗ 编译失败${NC}"
    exit 1
fi

# 启动 Storager 节点
echo -e "${YELLOW}[3/4] 启动 Storager 节点 (使用 ${ADS_MODE})...${NC}"
BASE_PORT=50052
STORAGER_PIDS=()

for i in $(seq 1 $NUM_STORAGERS); do
    PORT=$((BASE_PORT + i - 1))
    echo "  - 正在启动 Storager $i (Port: $PORT, ADS: ${(U)ADS_MODE})..."
    ./target/${BUILD_MODE}/storager $PORT $ADS_MODE > logs/storager${i}.log 2>&1 &
    PID=$!
    STORAGER_PIDS+=($PID)
    
    # 等待启动
    sleep 0.5
    if kill -0 $PID 2>/dev/null; then
        echo -e "    ${GREEN}✓ Storager $i 启动成功 (PID: $PID)${NC}"
    else
        echo -e "    ${RED}✗ Storager $i 启动失败${NC}"
        echo -e "    ${YELLOW}查看日志: tail logs/storager${i}.log${NC}"
    fi
done

echo "  等待 Storager 节点初始化..."
sleep 2

# 构建 storager 地址列表
STORAGER_ADDRS=""
for i in $(seq 1 $NUM_STORAGERS); do
    PORT=$((BASE_PORT + i - 1))
    if [ -z "$STORAGER_ADDRS" ]; then
        STORAGER_ADDRS="[::1]:$PORT"
    else
        STORAGER_ADDRS="$STORAGER_ADDRS,[::1]:$PORT"
    fi
done

# 启动 Manager
echo -e "${YELLOW}[4/4] 启动 Manager 节点 (使用 ${ADS_MODE})...${NC}"
echo "  - 正在启动 Manager (Port: 50051, ADS: ${(U)ADS_MODE})..."
./target/${BUILD_MODE}/manager --ads-mode $ADS_MODE --storagers "$STORAGER_ADDRS" > logs/manager.log 2>&1 &
MANAGER_PID=$!

# 等待启动
sleep 1
if kill -0 $MANAGER_PID 2>/dev/null; then
    echo -e "    ${GREEN}✓ Manager 启动成功 (PID: $MANAGER_PID)${NC}"
else
    echo -e "    ${RED}✗ Manager 启动失败${NC}"
    echo -e "    ${YELLOW}查看日志: tail logs/manager.log${NC}"
    exit 1
fi

echo "  等待 Manager 初始化..."
sleep 2

# 验证所有进程状态
echo ""
echo -e "${YELLOW}验证服务状态...${NC}"
ALL_OK=true

# 检查 Manager
if kill -0 $MANAGER_PID 2>/dev/null; then
    echo -e "  ${GREEN}✓ Manager 运行正常 (PID: $MANAGER_PID)${NC}"
else
    echo -e "  ${RED}✗ Manager 进程已停止${NC}"
    ALL_OK=false
fi

# 检查所有 Storager
for i in $(seq 1 $NUM_STORAGERS); do
    PID=${STORAGER_PIDS[$i]}
    if kill -0 $PID 2>/dev/null; then
        echo -e "  ${GREEN}✓ Storager $i 运行正常 (PID: $PID)${NC}"
    else
        echo -e "  ${RED}✗ Storager $i 进程已停止${NC}"
        ALL_OK=false
    fi
done

if [ "$ALL_OK" = false ]; then
    echo ""
    echo -e "${RED}警告: 部分服务启动失败，请检查日志文件${NC}"
    echo -e "${YELLOW}查看日志: ls -lh logs/${NC}"
fi

# 验证服务是否正常启动
echo ""
if [ "$ALL_OK" = true ]; then
    echo -e "${GREEN}=== 系统启动成功 ===${NC}"
else
    echo -e "${YELLOW}=== 系统部分启动（有警告）===${NC}"
fi
echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}服务信息${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "  ${CYAN}Manager:${NC}    [::1]:50051 (PID: ${GREEN}$MANAGER_PID${NC}, ADS: ${YELLOW}${(U)ADS_MODE}${NC})"
for i in $(seq 1 $NUM_STORAGERS); do
    PORT=$((BASE_PORT + i - 1))
    echo -e "  ${CYAN}Storager $i:${NC} [::1]:$PORT (PID: ${GREEN}${STORAGER_PIDS[$i]}${NC}, ADS: ${YELLOW}${(U)ADS_MODE}${NC})"
done
echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}使用方法${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "  运行客户端:     ${CYAN}./target/${BUILD_MODE}/client${NC}"
echo -e "  运行性能测试:   ${CYAN}cargo run --${BUILD_MODE} --example performance_test${NC}"
echo -e "  运行负载测试:   ${CYAN}cargo run --${BUILD_MODE} --example workload_realistic${NC}"
echo -e "  查看 Manager:   ${CYAN}tail -f logs/manager.log${NC}"
echo -e "  查看 Storager:  ${CYAN}tail -f logs/storager1.log${NC}"
echo -e "  停止系统:       ${CYAN}./scripts/stop.sh${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo -e "${YELLOW}提示:${NC}"
echo -e "  - 所有日志保存在 ${CYAN}logs/${NC} 目录"
echo -e "  - 当前使用 ${YELLOW}${(U)ADS_MODE}${NC} 认证数据结构"
echo -e "  - 构建模式: ${YELLOW}${(U)BUILD_MODE}${NC}"
echo ""