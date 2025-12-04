#!/bin/bash
# 系统级性能测试脚本
#
# 测试完整的分布式存储系统架构
#
# 用法:
#   ./scripts/run_system_benchmark.sh              # 默认: small workload, MPT, 3 storagers
#   ./scripts/run_system_benchmark.sh mpt          # 指定 ADS 模式
#   ./scripts/run_system_benchmark.sh mest small   # 指定 ADS 和 workload
#   ./scripts/run_system_benchmark.sh acctrie medium 5  # 完整参数

set -e

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  System-Level Benchmark - Testing Complete Architecture"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 参数解析 (更直观的顺序)
# $1: ADS模式 (mpt/mest/acctrie)
# $2: workload大小 (small/medium/large) 或完整路径
# $3: storager数量

ADS_MODE=${1:-"mpt"}
WORKLOAD_SIZE=${2:-"small"}
NUM_STORAGERS=${3:-3}

# 解析 workload 路径
if [[ "$WORKLOAD_SIZE" == *.csv ]]; then
    # 如果提供了完整路径
    WORKLOAD="$WORKLOAD_SIZE"
else
    # 根据大小选择预设文件
    case "$WORKLOAD_SIZE" in
        small)
            WORKLOAD="data/workload_small_1000.csv"
            ;;
        medium)
            WORKLOAD="data/workload_medium_10000.csv"
            ;;
        large)
            WORKLOAD="data/workload_large_100000.csv"
            ;;
        *)
            WORKLOAD="data/workload_small_1000.csv"
            ;;
    esac
fi

echo ""
echo "📋 Configuration:"
echo "  Workload:       $WORKLOAD"
echo "  ADS Mode:       $ADS_MODE"
echo "  Storager Nodes: $NUM_STORAGERS"
echo ""

# 编译项目
echo "🔨 Building system_benchmark..."
cargo build --release --bin system_benchmark

echo ""
echo "🚀 Starting system benchmark..."
echo ""

# 运行测试
./target/release/system_benchmark "$WORKLOAD" "$ADS_MODE" "$NUM_STORAGERS"

echo ""
echo "✨ Benchmark completed!"
echo ""
echo "📊 Check the logs/ directory for detailed reports"
