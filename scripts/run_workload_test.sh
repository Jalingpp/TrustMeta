#!/usr/bin/env zsh
# Workload 测试启动脚本

set -e

# 项目根目录
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

echo "========================================="
echo "分布式存储系统 Workload 测试套件"
echo "========================================="
echo ""

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 检查数据集
DATA_SMALL="$PROJECT_ROOT/data/workload_small_1000.csv"
DATA_MEDIUM="$PROJECT_ROOT/data/workload_medium_10000.csv"
DATA_LARGE="$PROJECT_ROOT/data/workload_large_100000.csv"

echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}[1/4] 检查数据集${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# 检查小型数据集
if [[ -f "$DATA_SMALL" ]]; then
    echo -e "  ${GREEN}✓${NC} 小型数据集存在: ${CYAN}$(basename $DATA_SMALL)${NC}"
else
    echo -e "  ${YELLOW}⚠${NC}  小型数据集不存在，正在生成..."
    python3 "$PROJECT_ROOT/scripts/generate_workload_data.py" --size small
    echo -e "  ${GREEN}✓${NC} 小型数据集生成完成"
fi

# 检查中型数据集
if [[ -f "$DATA_MEDIUM" ]]; then
    echo -e "  ${GREEN}✓${NC} 中型数据集存在: ${CYAN}$(basename $DATA_MEDIUM)${NC}"
else
    echo -e "  ${YELLOW}⚠${NC}  中型数据集不存在，正在生成..."
    python3 "$PROJECT_ROOT/scripts/generate_workload_data.py" --size medium
    echo -e "  ${GREEN}✓${NC} 中型数据集生成完成"
fi

echo -e "\n${GREEN}✓ 数据集准备完成${NC}"
echo ""

# 编译测试程序
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}[2/4] 编译测试程序${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "  正在编译 ${CYAN}workload_realistic${NC} (release 模式)..."
if cargo build --release --example workload_realistic 2>&1 | grep -E "(Compiling|Finished)"; then
    echo -e "  ${GREEN}✓ 编译成功${NC}"
else
    echo -e "  ${RED}✗ 编译失败${NC}"
    exit 1
fi
echo ""

# 检查系统是否运行
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}[3/4] 检查系统状态${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# 检查 Manager (更精确的匹配)
if pgrep -f "target/.*/manager" > /dev/null 2>&1; then
    local manager_pid=$(pgrep -f "target/.*/manager" | head -1)
    echo -e "  ${GREEN}✓${NC} Manager 运行中 (PID: ${CYAN}${manager_pid}${NC})"
else
    echo -e "  ${RED}✗${NC} Manager 未运行"
fi

# 检查 Storager (更精确的匹配)
local storager_count=$(pgrep -f "target/.*/storager" 2>/dev/null | wc -l | tr -d ' ')
if [ "$storager_count" -gt 0 ]; then
    echo -e "  ${GREEN}✓${NC} Storager 运行中 (${CYAN}${storager_count}${NC} 个节点)"
else
    echo -e "  ${RED}✗${NC} Storager 未运行"
fi

# 如果系统未运行，询问是否启动
if ! pgrep -f "target/.*/manager" > /dev/null 2>&1 || ! pgrep -f "target/.*/storager" > /dev/null 2>&1; then
    echo ""
    echo -e "${RED}✗ 系统未完全运行!${NC}"
    echo -e "${YELLOW}请先启动系统:${NC}"
    echo -e "  ${CYAN}./scripts/start_system.sh${NC}"
    echo ""
    echo "是否现在启动系统? (y/n)"
    read -r response
    if [[ "$response" =~ ^([yY][eE][sS]|[yY])$ ]]; then
        echo -e "${YELLOW}正在启动系统...${NC}"
        "$PROJECT_ROOT/scripts/start_system.sh"
        sleep 3
        echo -e "${GREEN}✓ 系统已启动${NC}"
    else
        echo -e "${RED}已取消测试${NC}"
        exit 1
    fi
else
    echo -e "\n${GREEN}✓ 系统运行正常${NC}"
fi
echo ""

# 运行测试
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}[4/4] 运行 Workload 测试${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# 根据参数选择数据集
DATASET="$DATA_SMALL"
DATASET_NAME="small (1,000 条)"

if [[ "$1" == "medium" ]]; then
    DATASET="$DATA_MEDIUM"
    DATASET_NAME="medium (10,000 条)"
    echo -e "${CYAN}使用中型数据集:${NC} ${YELLOW}$(basename $DATASET)${NC}"
elif [[ "$1" == "large" ]]; then
    DATASET="$DATA_LARGE"
    DATASET_NAME="large (100,000 条)"
    if [[ ! -f "$DATASET" ]]; then
        echo -e "${YELLOW}大型数据集不存在，正在生成 (需要较长时间)...${NC}"
        python3 "$PROJECT_ROOT/scripts/generate_workload_data.py" --size large
        echo -e "${GREEN}✓ 数据集生成完成${NC}"
    fi
    echo -e "${CYAN}使用大型数据集:${NC} ${YELLOW}$(basename $DATASET)${NC}"
elif [[ -n "$1" ]] && [[ -f "$1" ]]; then
    DATASET="$1"
    DATASET_NAME="custom"
    echo -e "${CYAN}使用自定义数据集:${NC} ${YELLOW}$(basename $DATASET)${NC}"
else
    echo -e "${CYAN}使用小型数据集:${NC} ${YELLOW}$(basename $DATASET)${NC}"
fi

echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# 运行测试
cargo run --release --example workload_realistic "$DATASET"

echo ""
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}测试完成${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo -e "${CYAN}后续操作:${NC}"
echo -e "  查看统计数据:   ${CYAN}cat ${DATASET%.csv}_stats.json | jq${NC}"
echo -e "  测试中型数据:   ${CYAN}$0 medium${NC}"
echo -e "  测试大型数据:   ${CYAN}$0 large${NC}"
echo -e "  查看系统日志:   ${CYAN}tail -f logs/manager.log${NC}"
echo ""
