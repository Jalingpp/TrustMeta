#!/usr/bin/env zsh

# 分布式存储系统完整测试脚本
# 测试所有 ADS 模式和不同数据规模

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

# 默认参数
TEST_ADS_MODES=("mpt" "mest")  # 默认测试 MPT 和 MEST
TEST_DATA_SIZES=("small")      # 默认只测小数据集
QUICK_TEST=false

# 显示帮助
show_help() {
    echo -e "${CYAN}分布式存储系统 - 完整测试脚本${NC}"
    echo ""
    echo "用法: $0 [选项]"
    echo ""
    echo "选项:"
    echo "  -a, --ads <MODES>      测试的 ADS 模式,逗号分隔 (默认: mpt,mest)"
    echo "                         可选: mpt,mest 或 all"
    echo "  -s, --size <SIZES>     测试数据规模,逗号分隔 (默认: small)"
    echo "                         可选: small,medium,large 或 all"
    echo "  -q, --quick            快速测试模式 (仅 MEST + 小数据集)"
    echo "  -h, --help             显示此帮助信息"
    echo ""
    echo "示例:"
    echo "  $0                           # 默认测试 (MPT+MEST, 小数据集)"
    echo "  $0 --quick                   # 快速测试 (MEST, 小数据集)"
    echo "  $0 -a all -s all             # 完整测试 (所有模式, 所有规模)"
    echo "  $0 -a mpt -s small,medium    # MPT 模式, 小+中数据集"
    echo ""
    exit 0
}

# 解析参数
while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--ads)
            if [[ "$2" == "all" ]]; then
                TEST_ADS_MODES=("mpt" "mest")
            else
                IFS=',' read -ra TEST_ADS_MODES <<< "$2"
            fi
            shift 2
            ;;
        -s|--size)
            if [[ "$2" == "all" ]]; then
                TEST_DATA_SIZES=("small" "medium" "large")
            else
                IFS=',' read -ra TEST_DATA_SIZES <<< "$2"
            fi
            shift 2
            ;;
        -q|--quick)
            QUICK_TEST=true
            TEST_ADS_MODES=("mest")
            TEST_DATA_SIZES=("small")
            shift
            ;;
        -h|--help)
            show_help
            ;;
        *)
            echo -e "${RED}未知选项: $1${NC}"
            show_help
            ;;
    esac
done

# 测试结果记录 (使用 zsh 关联数组)
typeset -A TEST_RESULTS
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# 记录测试结果
record_result() {
    local test_name="$1"
    local test_status="$2"
    TEST_RESULTS[$test_name]="$test_status"
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    if [[ "$test_status" == "PASS" ]]; then
        PASSED_TESTS=$((PASSED_TESTS + 1))
    else
        FAILED_TESTS=$((FAILED_TESTS + 1))
    fi
}

# 测试单个配置
test_configuration() {
    local ads_mode="$1"
    local data_size="$2"
    
    echo ""
    echo -e "${CYAN}=======================================${NC}"
    echo -e "${CYAN}测试配置: ADS=${(U)ads_mode}, 数据规模=${data_size}${NC}"
    echo -e "${CYAN}=======================================${NC}"
    
    # 停止现有系统
    echo -e "${YELLOW}[1/4] 停止现有系统...${NC}"
    ./scripts/stop.sh > /dev/null 2>&1 || true
    sleep 2
    
    # 启动系统
    echo -e "${YELLOW}[2/4] 启动系统 (ADS: ${ads_mode})...${NC}"
    if ! ./scripts/start_system.sh -a "$ads_mode" > /tmp/start_${ads_mode}.log 2>&1; then
        echo -e "${RED}启动失败!${NC}"
        cat /tmp/start_${ads_mode}.log
        record_result "${ads_mode}_${data_size}" "FAIL"
        return 1
    fi
    echo -e "${GREEN}系统启动成功${NC}"
    sleep 3
    
    # 运行测试
    echo -e "${YELLOW}[3/4] 运行 workload 测试...${NC}"
    echo ""
    local test_output="/tmp/test_${ads_mode}_${data_size}.log"
    
    # 实时显示测试输出并保存到文件
    ./scripts/run_workload_test.sh "$data_size" 2>&1 | tee "$test_output"
    local test_result=${PIPESTATUS[0]}
    
    echo ""
    if [[ $test_result -eq 0 ]]; then
        echo -e "${GREEN}✓ 测试通过!${NC}"
        
        # 提取关键指标
        local insert_ops=$(grep "Workload 1: 批量插入" -A 5 "$test_output" | grep "吞吐量:" | grep -oE '[0-9]+ ops/s' | head -1)
        local query_qps=$(grep "Workload 2: 随机关键词查询" -A 5 "$test_output" | grep "QPS:" | grep -oE '[0-9]+' | head -1)
        
        echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
        echo -e "${CYAN}关键性能指标:${NC}"
        echo -e "  ${GREEN}●${NC} 批量插入吞吐量: ${YELLOW}${insert_ops:-N/A}${NC}"
        echo -e "  ${GREEN}●${NC} 随机查询 QPS: ${YELLOW}${query_qps:-N/A}${NC}"
        echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
        
        record_result "${ads_mode}_${data_size}" "PASS"
    else
        echo -e "${RED}✗ 测试失败!${NC}"
        echo -e "${YELLOW}最后 20 行输出:${NC}"
        tail -20 "$test_output"
        record_result "${ads_mode}_${data_size}" "FAIL"
        return 1
    fi
    
    # 停止系统
    echo -e "${YELLOW}[4/4] 停止系统...${NC}"
    ./scripts/stop.sh > /dev/null 2>&1 || true
    sleep 2
    
    return 0
}

# 主测试流程
main() {
    echo -e "${CYAN}=========================================${NC}"
    echo -e "${CYAN}分布式存储系统 - 完整测试${NC}"
    echo -e "${CYAN}=========================================${NC}"
    echo ""
    echo "测试配置:"
    echo "  ADS 模式: ${TEST_ADS_MODES[*]}"
    echo "  数据规模: ${TEST_DATA_SIZES[*]}"
    echo "  快速模式: $QUICK_TEST"
    echo ""
    
    # 检查并生成数据集
    echo -e "${YELLOW}准备数据集...${NC}"
    for size in "${TEST_DATA_SIZES[@]}"; do
        case $size in
            small)
                if [[ ! -f "$PROJECT_ROOT/data/workload_small_1000.csv" ]]; then
                    echo "生成小型数据集..."
                    python3 "$PROJECT_ROOT/scripts/generate_workload_data.py" --size small
                fi
                ;;
            medium)
                if [[ ! -f "$PROJECT_ROOT/data/workload_medium_10000.csv" ]]; then
                    echo "生成中型数据集..."
                    python3 "$PROJECT_ROOT/scripts/generate_workload_data.py" --size medium
                fi
                ;;
            large)
                if [[ ! -f "$PROJECT_ROOT/data/workload_large_100000.csv" ]]; then
                    echo "生成大型数据集..."
                    python3 "$PROJECT_ROOT/scripts/generate_workload_data.py" --size large
                fi
                ;;
        esac
    done
    echo -e "${GREEN}数据集准备完成${NC}"
    
    # 开始测试
    local start_time=$(date +%s)
    
    for ads_mode in "${TEST_ADS_MODES[@]}"; do
        for data_size in "${TEST_DATA_SIZES[@]}"; do
            test_configuration "$ads_mode" "$data_size"
        done
    done
    
    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    
    # 显示测试结果摘要
    echo ""
    echo -e "${CYAN}=========================================${NC}"
    echo -e "${CYAN}测试结果摘要${NC}"
    echo -e "${CYAN}=========================================${NC}"
    echo ""
    
    for test_name in "${(@k)TEST_RESULTS}"; do
        local test_status="${TEST_RESULTS[$test_name]}"
        if [[ "$test_status" == "PASS" ]]; then
            echo -e "  ${GREEN}✓${NC} $test_name: ${GREEN}PASS${NC}"
        else
            echo -e "  ${RED}✗${NC} $test_name: ${RED}FAIL${NC}"
        fi
    done
    
    echo ""
    echo "总测试数: $TOTAL_TESTS"
    echo -e "通过: ${GREEN}$PASSED_TESTS${NC}"
    echo -e "失败: ${RED}$FAILED_TESTS${NC}"
    echo "总耗时: ${duration}秒"
    echo ""
    
    if [[ $FAILED_TESTS -eq 0 ]]; then
        echo -e "${GREEN}=========================================${NC}"
        echo -e "${GREEN}所有测试通过!${NC}"
        echo -e "${GREEN}=========================================${NC}"
        exit 0
    else
        echo -e "${RED}=========================================${NC}"
        echo -e "${RED}部分测试失败${NC}"
        echo -e "${RED}=========================================${NC}"
        exit 1
    fi
}

# 运行主程序
main
