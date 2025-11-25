#!/usr/bin/env zsh

# 比较增删改查操作性能的脚本

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# 项目根目录
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# 支持参数选择模式
MODES=("mest" "mpt")

while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--ads)
            MODES=("$2")
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done

for mode in "${MODES[@]}"; do
    MODE_UPPER=$(echo "$mode" | tr '[:lower:]' '[:upper:]')
    
    echo -e "\n${CYAN}══════════════════════════════════════════════════════════${NC}"
    echo -e "${CYAN}   开始测试 ${MODE_UPPER} 模式性能${NC}"
    echo -e "${CYAN}══════════════════════════════════════════════════════════${NC}\n"

    # 停止现有系统
    echo -e "${YELLOW}停止现有系统...${NC}"
    ./scripts/stop.sh > /dev/null 2>&1 || true
    sleep 2

    # 启动系统
    echo -e "${BLUE}启动系统 (${MODE_UPPER} 模式)...${NC}"
    ./scripts/start_system.sh -a "$mode" -m release > "logs/start_system_ops_test_${mode}.log" 2>&1

    # 检查启动是否成功
    if [ $? -ne 0 ]; then
        echo -e "${RED}系统启动失败，请查看 logs/start_system_ops_test_${mode}.log${NC}"
        exit 1
    fi

    # 等待系统完全启动
    echo -e "${YELLOW}等待系统初始化 (5s)...${NC}"
    sleep 5

    # 运行操作对比测试
    echo -e "${CYAN}运行 ${MODE_UPPER} 模式下的操作性能对比测试...${NC}"
    cargo run --release --example operation_comparison | tee "logs/operation_comparison_${mode}.log"

    # 停止系统
    echo -e "${YELLOW}测试完成，停止系统...${NC}"
    ./scripts/stop.sh > /dev/null 2>&1
    
    echo -e "${GREEN}${MODE_UPPER} 模式测试结束${NC}"
    sleep 2

done

echo -e "\n${GREEN}所有模式测试完成！${NC}"
