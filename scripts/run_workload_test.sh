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

echo -e "${BLUE}[1/4] 检查数据集...${NC}"
if [[ ! -f "$DATA_SMALL" ]]; then
    echo -e "${YELLOW}小型数据集不存在,正在生成...${NC}"
    python3 "$PROJECT_ROOT/scripts/generate_workload_data.py" --size small
fi

if [[ ! -f "$DATA_MEDIUM" ]]; then
    echo -e "${YELLOW}中型数据集不存在,正在生成...${NC}"
    python3 "$PROJECT_ROOT/scripts/generate_workload_data.py" --size medium
fi

echo -e "${GREEN}✓ 数据集准备完成${NC}"
echo ""

# 编译测试程序
echo -e "${BLUE}[2/4] 编译测试程序...${NC}"
cargo build --release --example workload_realistic 2>&1 | grep -E "(Compiling|Finished)" || true
echo -e "${GREEN}✓ 编译完成${NC}"
echo ""

# 检查系统是否运行
echo -e "${BLUE}[3/4] 检查系统状态...${NC}"
if ! pgrep -f "manager" > /dev/null || ! pgrep -f "storager" > /dev/null; then
    echo -e "${RED}✗ 系统未运行!${NC}"
    echo -e "${YELLOW}请先启动系统:${NC}"
    echo -e "  ./scripts/start_system.sh"
    echo ""
    echo "是否现在启动系统? (y/n)"
    read -r response
    if [[ "$response" =~ ^([yY][eE][sS]|[yY])$ ]]; then
        echo -e "${YELLOW}启动系统...${NC}"
        "$PROJECT_ROOT/scripts/start_system.sh"
        sleep 3
        echo -e "${GREEN}✓ 系统已启动${NC}"
    else
        echo -e "${RED}已取消测试${NC}"
        exit 1
    fi
else
    echo -e "${GREEN}✓ 系统正在运行${NC}"
fi
echo ""

# 运行测试
echo -e "${BLUE}[4/4] 运行 Workload 测试...${NC}"
echo ""
echo "========================================="
echo "测试数据集: $DATA_SMALL"
echo "========================================="
echo ""

# 根据参数选择数据集
DATASET="$DATA_SMALL"
if [[ "$1" == "medium" ]]; then
    DATASET="$DATA_MEDIUM"
    echo "使用中型数据集: $DATASET"
elif [[ "$1" == "large" ]]; then
    DATASET="$DATA_LARGE"
    if [[ ! -f "$DATASET" ]]; then
        echo -e "${YELLOW}大型数据集不存在,正在生成(需要较长时间)...${NC}"
        python3 "$PROJECT_ROOT/scripts/generate_workload_data.py" --size large
    fi
    echo "使用大型数据集: $DATASET"
elif [[ -n "$1" ]] && [[ -f "$1" ]]; then
    DATASET="$1"
    echo "使用自定义数据集: $DATASET"
fi

# 运行测试
cargo run --release --example workload_realistic "$DATASET"

echo ""
echo -e "${GREEN}=========================================${NC}"
echo -e "${GREEN}测试完成!${NC}"
echo -e "${GREEN}=========================================${NC}"
echo ""
echo -e "${YELLOW}提示:${NC}"
echo "  - 查看统计数据: cat ${DATASET%.csv}_stats.json | jq"
echo "  - 运行热点测试: cargo run --release --example workload_hotspot"
echo "  - 测试中型数据: $0 medium"
echo "  - 测试大型数据: $0 large"
