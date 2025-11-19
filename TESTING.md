# Testing Guide

## Quick Start

### 1. Start the System

```bash
# Start with MEST ADS (recommended)
./scripts/start_system.sh -a mest

# Or start with MPT
./scripts/start_system.sh -a mpt

# Or start with MPT
./scripts/start_system.sh -a mpt
```

### 2. Run Tests

```bash
# Full test suite with 100-record dataset
cargo run --release --example testdata_test

# Basic integration test
cargo run --release --example integration_test

# MEST standalone test
cargo run --release --example test_mest
```

### 3. Stop the System

```bash
./scripts/stop.sh
```

## Available Tests

### 1. Testdata Test (`testdata_test`)

**Purpose**: Comprehensive testing with 100-record dataset  
**Data**: `data/testdata` (100 records across 13 categories)

**Test Coverage**:
- ✅ Batch add operations (100 records)
- ✅ Single keyword queries
- ✅ Boolean queries (AND/OR operations)
- ✅ Update operations
- ✅ Delete operations
- ✅ Data consistency verification

**Run**:
```bash
cargo run --release --example testdata_test
```

**Expected Output**:
```
测试 1: 批量添加数据
  添加完成: 100 成功, 0 失败
  耗时: ~70ms
  平均速度: ~1400 条/秒

测试 2: 单关键词查询
  'animal': 找到 8 个文件
  'red': 找到 7 个文件
  ...

测试 3: 布尔查询
  [animal AND red]: 找到 0 个文件
  [flower AND pink]: 找到 3 个文件
  ...

测试 4: 更新操作
  更新完成: 5 条记录

测试 5: 删除部分数据
  删除完成: 10 条记录
```

### 2. Integration Test (`integration_test`)

**Purpose**: Verify complete data flow  
**Test Coverage**:
- ✅ Client → Manager communication
- ✅ Manager consistent hashing
- ✅ Manager → Storager communication
- ✅ ADS data structure operations
- ✅ Cryptographic proof generation
- ✅ Proof verification

**Run**:
```bash
cargo run --release --example integration_test
```

**Test Cases**:
1. Add files with keywords
2. Single keyword queries
3. Boolean function queries
4. Update file keywords
5. Delete files

### 3. MEST Standalone Test (`test_mest`)

**Purpose**: Test MEST implementation directly  
**Test Coverage**:
- ✅ MEST add operation
- ✅ MEST query operation
- ✅ MEST delete operation
- ✅ Root hash updates

**Run**:
```bash
cargo run --release --example test_mest
```

## Performance Comparison

### MPT vs MEST

Run benchmark across both ADS types:

```bash
# Test MPT
./scripts/stop.sh
./scripts/start_system.sh -a mpt
sleep 3
cargo run --release --example testdata_test

# Test MEST
./scripts/stop.sh
./scripts/start_system.sh -a mest
sleep 3
cargo run --release --example testdata_test
```

**Typical Results**:

| Metric          | MPT         | MEST        | Winner          |
| --------------- | ----------- | ----------- | --------------- |
| Batch Add (100) | ~1614 ops/s | ~1451 ops/s | MPT (+11%)      |
| Query Latency   | ~350µs      | ~220µs      | **MEST (+37%)** |
| Update          | ✅           | ✅           | Tie             |
| Delete          | ✅           | ✅           | Tie             |
| Boolean Query   | ✅           | ✅           | Tie             |

## Manual Testing

### Using the Client

```bash
# Start client (interactive mode)
./target/release/client

# Or use direct commands
echo "help" | ./target/release/client
```

### Viewing Logs

```bash
# Manager logs
tail -f logs/manager.log

# Storager logs
tail -f logs/storager1.log
tail -f logs/storager2.log
tail -f logs/storager3.log
```

## Test Data Format

The test dataset (`data/testdata`) contains 100 records:

```
fid,keyword1,keyword2,keyword3,...
001,animal,bird,art
002,animal,cat,art
003,animal,dog,art
...
```

**Categories** (13 types):
- animal, clothes, flower, food, vehicle
- electronics, furniture, plant, beverage
- stationery, toy, game, book, music, sport

## Troubleshooting

### Services Not Starting

```bash
# Check if ports are already in use
lsof -iTCP:50051-50054 -sTCP:LISTEN

# Force kill if needed
./scripts/stop.sh
pkill -9 manager
pkill -9 storager
```

### Connection Errors

```bash
# Check services are running
ps aux | grep -E "(manager|storager)" | grep -v grep

# Check logs for errors
cat logs/manager.log
cat logs/storager*.log
```

### Compilation Errors

```bash
# Clean build
cargo clean
cargo build --release

# Rebuild specific component
cargo build --release -p manager
cargo build --release -p storager
```

## Development Testing

### Run All Tests

```bash
# Unit tests
cargo test --workspace

# Integration tests
cargo test --workspace --test '*'
```

### Check Code

```bash
# Format check
cargo fmt --check

# Lint
cargo clippy --all-targets --all-features

# Build
cargo build --release
```

## CI/CD Testing

Recommended test sequence for automated testing:

```bash
#!/bin/bash

# 1. Clean environment
./scripts/stop.sh
cargo clean

# 2. Build
cargo build --release

# 3. Test each ADS type
for ads in mpt mest; do
    echo "Testing $ads..."
    ./scripts/start_system.sh -a $ads
    sleep 3
    cargo run --release --example testdata_test || exit 1
    ./scripts/stop.sh
    sleep 2
done

# 4. Integration test
./scripts/start_system.sh -a mest
sleep 3
cargo run --release --example integration_test || exit 1
./scripts/stop.sh

echo "All tests passed!"
```

## Performance Profiling

### Memory Usage

```bash
# Monitor during test
ps aux | grep -E "(manager|storager)" | awk '{print $6}'
```

### CPU Usage

```bash
# Monitor CPU
top -pid $(pgrep manager) -pid $(pgrep storager)
```

### Query Performance

```bash
# Enable timing in logs (modify code to add timestamps)
# Or use the built-in timing in testdata_test
```

## Test Coverage Goals

- ✅ All ADS types (MPT, MEST)
- ✅ All operations (Add, Query, Update, Delete)
- ✅ Boolean query operators (AND, OR, parentheses)
- ✅ Multi-storager routing
- ✅ Proof generation and verification
- ✅ Concurrent operations
- ✅ Error handling
- ✅ Data consistency

## Next Steps

1. Add stress tests (1000+ records)
2. Add concurrency tests (parallel clients)
3. Add failure recovery tests
4. Add network partition tests
5. Add performance benchmarks
