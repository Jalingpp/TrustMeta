#!/usr/bin/env zsh

# MPT vs MEST 性能对比测试脚本
# 测试两种ADS实现在相同workload下的性能表现

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m'

# 项目根目录
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# 结果文件
RESULTS_FILE="logs/performance_comparison_$(date +%Y%m%d_%H%M%S).md"
mkdir -p logs

echo -e "${CYAN}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║     MPT vs MEST 性能对比测试                               ║${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════════════════════╝${NC}"
echo ""

# 初始化结果文件
cat > "$RESULTS_FILE" << 'EOF'
# MPT vs MEST 性能对比测试报告

测试时间: 
测试环境: macOS
数据集规模: 1K, 10K

---

EOF

echo "测试时间: $(date '+%Y-%m-%d %H:%M:%S')" >> "$RESULTS_FILE"
echo "" >> "$RESULTS_FILE"

# 测试配置
WORKLOAD_SIZES=("small" "medium")
ADS_MODES=("mpt" "mest")

# 存储测试结果
typeset -A TEST_RESULTS

# 函数: 停止系统
stop_system() {
    echo -e "${YELLOW}停止现有系统...${NC}"
    pkill -f "target/release/manager" 2>/dev/null || true
    pkill -f "target/release/storager" 2>/dev/null || true
    sleep 2
}

# 函数: 启动系统
start_system() {
    local ads_mode=$1
    echo -e "${BLUE}启动系统 (ADS: ${ads_mode})...${NC}"
    
    # 修改配置文件
    if [[ "$ads_mode" == "mpt" ]]; then
        sed -i.bak 's/"ads_mode": "Mest"/"ads_mode": "Mpt"/' config.json
    else
        sed -i.bak 's/"ads_mode": "Mpt"/"ads_mode": "Mest"/' config.json
    fi
    
    # 启动Manager
    ./target/release/manager > logs/manager_${ads_mode}.log 2>&1 &
    sleep 2
    
    # 启动Storager节点
    ./target/release/storager 50052 > logs/storager_${ads_mode}_50052.log 2>&1 &
    ./target/release/storager 50053 > logs/storager_${ads_mode}_50053.log 2>&1 &
    sleep 3
    
    # 验证服务启动
    if pgrep -f "target/release/manager" > /dev/null && pgrep -f "target/release/storager" > /dev/null; then
        echo -e "${GREEN}✓ 系统启动成功${NC}"
        return 0
    else
        echo -e "${RED}✗ 系统启动失败${NC}"
        return 1
    fi
}

# 函数: 运行workload测试
run_workload() {
    local ads_mode=$1
    local size=$2
    local dataset=""
    
    case $size in
        small)
            dataset="data/workload_small_1000.csv"
            ;;
        medium)
            dataset="data/workload_medium_10000.csv"
            ;;
        large)
            dataset="data/workload_large_100000.csv"
            ;;
    esac
    
    echo -e "${CYAN}运行 ${ads_mode} ${size} workload...${NC}"
    
    local output_file="logs/workload_${ads_mode}_${size}.log"
    
    cargo run --release --example workload_realistic "$dataset" > "$output_file" 2>&1
    
    if [[ $? -eq 0 ]]; then
        echo -e "${GREEN}✓ 测试完成${NC}"
        
        # 提取关键性能指标
        local insert_ops=$(grep "批量插入" -A 1 "$output_file" | grep "吞吐量" | grep -oE '[0-9]+' | tail -1)
        local query_qps=$(grep "随机关键词查询" -A 1 "$output_file" | grep "QPS" | grep -oE '[0-9]+' | tail -1)
        local insert_time=$(grep "批量插入" -A 1 "$output_file" | grep "耗时" | grep -oE '[0-9]+\.[0-9]+')
        
        TEST_RESULTS[${ads_mode}_${size}_insert_ops]=$insert_ops
        TEST_RESULTS[${ads_mode}_${size}_query_qps]=$query_qps
        TEST_RESULTS[${ads_mode}_${size}_insert_time]=$insert_time
        
        echo "   插入吞吐量: ${insert_ops} ops/s"
        echo "   查询QPS: ${query_qps}"
        echo "   插入总时间: ${insert_time}s"
        
        return 0
    else
        echo -e "${RED}✗ 测试失败${NC}"
        return 1
    fi
}

# 主测试循环
echo -e "\n${MAGENTA}开始性能对比测试...${NC}\n"

for size in "${WORKLOAD_SIZES[@]}"; do
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}数据集: ${size}${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}\n"
    
    for ads_mode in "${ADS_MODES[@]}"; do
        ADS_UPPER=$(echo "$ads_mode" | tr '[:lower:]' '[:upper:]')
        echo -e "${BLUE}[${ADS_UPPER}] 测试${NC}"
        
        # 停止旧系统
        stop_system
        
        # 启动新系统
        if ! start_system "$ads_mode"; then
            echo -e "${RED}跳过 ${ads_mode} ${size} 测试${NC}"
            continue
        fi
        
        # 运行workload
        run_workload "$ads_mode" "$size"
        
        echo ""
    done
done

# 停止系统
stop_system

# 生成对比报告
echo -e "\n${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${CYAN}性能对比结果${NC}"
echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}\n"

cat >> "$RESULTS_FILE" << 'EOF'
## 测试结果

### 小型数据集 (1K 记录)

| 指标 | MPT | MEST | MEST vs MPT |
|------|-----|------|-------------|
EOF

# 小型数据集对比
mpt_small_insert=${TEST_RESULTS[mpt_small_insert_ops]:-0}
mest_small_insert=${TEST_RESULTS[mest_small_insert_ops]:-0}
mpt_small_query=${TEST_RESULTS[mpt_small_query_qps]:-0}
mest_small_query=${TEST_RESULTS[mest_small_query_qps]:-0}

if [[ $mpt_small_insert -gt 0 && $mest_small_insert -gt 0 ]]; then
    insert_ratio=$(echo "scale=2; $mest_small_insert * 100 / $mpt_small_insert" | bc)
    echo "| 插入吞吐量 (ops/s) | $mpt_small_insert | $mest_small_insert | ${insert_ratio}% |" >> "$RESULTS_FILE"
fi

if [[ $mpt_small_query -gt 0 && $mest_small_query -gt 0 ]]; then
    query_ratio=$(echo "scale=2; $mest_small_query * 100 / $mpt_small_query" | bc)
    echo "| 查询QPS | $mpt_small_query | $mest_small_query | ${query_ratio}% |" >> "$RESULTS_FILE"
fi

cat >> "$RESULTS_FILE" << 'EOF'

### 中型数据集 (10K 记录)

| 指标 | MPT | MEST | MEST vs MPT |
|------|-----|------|-------------|
EOF

# 中型数据集对比
mpt_medium_insert=${TEST_RESULTS[mpt_medium_insert_ops]:-0}
mest_medium_insert=${TEST_RESULTS[mest_medium_insert_ops]:-0}
mpt_medium_query=${TEST_RESULTS[mpt_medium_query_qps]:-0}
mest_medium_query=${TEST_RESULTS[mest_medium_query_qps]:-0}

if [[ $mpt_medium_insert -gt 0 && $mest_medium_insert -gt 0 ]]; then
    insert_ratio=$(echo "scale=2; $mest_medium_insert * 100 / $mpt_medium_insert" | bc)
    echo "| 插入吞吐量 (ops/s) | $mpt_medium_insert | $mest_medium_insert | ${insert_ratio}% |" >> "$RESULTS_FILE"
fi

if [[ $mpt_medium_query -gt 0 && $mest_medium_query -gt 0 ]]; then
    query_ratio=$(echo "scale=2; $mest_medium_query * 100 / $mpt_medium_query" | bc)
    echo "| 查询QPS | $mpt_medium_query | $mest_medium_query | ${query_ratio}% |" >> "$RESULTS_FILE"
fi

# 打印到终端
echo -e "${GREEN}小型数据集 (1K):${NC}"
echo "  MPT  插入: ${mpt_small_insert} ops/s, 查询: ${mpt_small_query} QPS"
echo "  MEST 插入: ${mest_small_insert} ops/s, 查询: ${mest_small_query} QPS"
echo ""
echo -e "${GREEN}中型数据集 (10K):${NC}"
echo "  MPT  插入: ${mpt_medium_insert} ops/s, 查询: ${mpt_medium_query} QPS"
echo "  MEST 插入: ${mest_medium_insert} ops/s, 查询: ${mest_medium_query} QPS"
echo ""

cat >> "$RESULTS_FILE" << 'EOF'

## 分析总结

### Proof大小对比
- **MPT**: 完整Merkle proof,大小取决于树深度
- **MEST**: 两层proof (桶级Merkle + MGT),~1400字节/关键字

### 性能特点
- **MPT**: 
  - 优势: 成熟的数据结构,查询性能稳定
  - 劣势: 树深度影响proof大小
  
- **MEST**: 
  - 优势: 扩展性强,支持动态分桶
  - 劣势: Proof略大(包含两层结构)

EOF

echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}性能对比测试完成!${NC}"
echo -e "详细报告已保存到: ${MAGENTA}${RESULTS_FILE}${NC}"
echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}\n"

# 打开报告
cat "$RESULTS_FILE"
