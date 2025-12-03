//! 三种 ADS 系统完整性能对比测试
//!
//! 测试目标：
//! - AccTrie: 基于密码学累加器的认证数据结构
//! - MEST: Merkle-based Extendible Segmented Hash Tree
//! - MPT: Merkle Patricia Trie
//!
//! 测试指标：
//! 1. 插入性能 (吞吐量、延迟)
//! 2. 查询性能 (吞吐量、延迟)
//! 3. 删除性能 (吞吐量、延迟)
//! 4. 证明大小 (内存占用)
//! 5. 证明验证性能

use ads_rust::acctrie::AccTrie;
use ads_rust::mest::{MEHT, verify_key_proof, KVPair as MestKVPair};
use ads_rust::mpt::{KVPair as MptKVPair, MPT, MemoryDatabase};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use std::mem;

// ================================================================================================
// 测试配置和数据结构
// ================================================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestConfig {
    num_operations: usize,
    key_prefix: String,
    value_base: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct OperationMetrics {
    ads_type: String,
    operation: String,
    total_ops: usize,
    total_time_ms: u128,
    avg_time_us: f64,
    throughput_ops_sec: f64,
    proof_size_estimate_bytes: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchmarkReport {
    timestamp: String,
    config: TestConfig,
    metrics: Vec<OperationMetrics>,
}

// ================================================================================================
// 辅助函数
// ================================================================================================

/// 生成测试数据
fn generate_test_data(config: &TestConfig) -> Vec<(Vec<u8>, i64)> {
    (0..config.num_operations)
        .map(|i| {
            let key = format!("{}{:08}", config.key_prefix, i).into_bytes();
            let value = config.value_base + (i as i64);
            (key, value)
        })
        .collect()
}

/// 估算AccTrie InsertionProof的内存大小
fn estimate_acctrie_insertion_proof_size() -> usize {
    // InsertionProof包含:
    // - key: Vec<u8> (动态)
    // - value: i64 (8 bytes)
    // - key_prev, key_next: Option<Vec<u8>> (动态)
    // - 累加器值 (G1Affine): ~96 bytes each
    // - 成员证明 (MembershipProof): ~192 bytes each
    
    let base_size = mem::size_of::<i64>(); // value
    let g1_affine_size = 96; // 椭圆曲线点的近似大小
    let membership_proof_size = 192; // 成员证明的近似大小
    
    base_size 
        + 20 // key 平均大小
        + 40 // key_prev + key_next 平均大小
        + g1_affine_size * 5 // 5个累加器值
        + membership_proof_size * 5 // 5个成员证明
}

/// 估算AccTrie QueryResult的内存大小
fn estimate_acctrie_query_result_size() -> usize {
    // QueryResult::Exists包含ExistenceProof
    // 或QueryResult::NotExists包含NonExistenceProof
    let base_size = 20; // key
    let g1_affine_size = 96;
    let membership_proof_size = 192;
    
    base_size + g1_affine_size * 3 + membership_proof_size * 3
}

/// 估算MEST KeyProof的内存大小
fn estimate_mest_key_proof_size() -> usize {
    // KeyProof包含:
    // - key: String
    // - bucket_key: Vec<i32>
    // - bucket_proof: BucketProofOut
    //   - value: String
    //   - seg_root_hash: [u8; 32]
    //   - proof: MHTProof (Merkle tree proof)
    //   - leaf_segment_roots: Vec<[u8; 32]>
    // - mgt_proof: MGTProof
    
    let key_size = 20;
    let bucket_key_size = 4 * 10; // assume ~10 levels
    let hash_size = 32;
    let merkle_proof_size = hash_size * 10; // ~10 levels in Merkle tree
    let mgt_proof_size = hash_size * 15; // MGT proof siblings
    
    key_size + bucket_key_size + 100 + hash_size + merkle_proof_size + 
        (hash_size * 5) + mgt_proof_size
}

/// 估算MPT Proof的内存大小
fn estimate_mpt_proof_size() -> usize {
    // MPT证明包含路径上的所有节点哈希
    // 平均树深度约为8-12层
    let avg_depth = 10;
    let hash_size = 32;
    let node_size = hash_size + 50; // 哈希 + 节点信息
    
    avg_depth * node_size
}

// ================================================================================================
// AccTrie 性能测试
// ================================================================================================

fn benchmark_acctrie(config: &TestConfig) -> Vec<OperationMetrics> {
    println!("\n╔════════════════════════════════════════╗");
    println!("║      AccTrie 性能基准测试              ║");
    println!("╚════════════════════════════════════════╝");
    
    let mut metrics = Vec::new();
    let test_data = generate_test_data(config);
    let mut trie = AccTrie::new();
    
    // 1. 插入性能测试
    println!("\n[1/4] 插入性能测试");
    println!("----------------------------------------");
    let start = Instant::now();
    
    for (key, value) in &test_data {
        trie.insert(key.clone(), *value).unwrap();
    }
    
    let duration = start.elapsed();
    let proof_size = estimate_acctrie_insertion_proof_size();
    
    println!("  ✓ 插入 {} 条记录", test_data.len());
    println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
    println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / test_data.len() as f64);
    println!("  🚀 吞吐量: {:.2} ops/sec", test_data.len() as f64 / duration.as_secs_f64());
    println!("  📦 证明大小: ~{} bytes", proof_size);
    
    metrics.push(OperationMetrics {
        ads_type: "AccTrie".to_string(),
        operation: "Insert".to_string(),
        total_ops: test_data.len(),
        total_time_ms: duration.as_millis(),
        avg_time_us: duration.as_micros() as f64 / test_data.len() as f64,
        throughput_ops_sec: test_data.len() as f64 / duration.as_secs_f64(),
        proof_size_estimate_bytes: Some(proof_size),
    });
    
    // 2. 查询性能测试
    println!("\n[2/4] 查询性能测试");
    println!("----------------------------------------");
    let start = Instant::now();
    let mut success_count = 0;
    
    for (key, value) in &test_data {
        if let Ok(_result) = trie.query(key, *value) {
            success_count += 1;
        }
    }
    
    let duration = start.elapsed();
    let proof_size = estimate_acctrie_query_result_size();
    
    println!("  ✓ 查询 {} 条记录 (成功: {})", test_data.len(), success_count);
    println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
    println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / test_data.len() as f64);
    println!("  🚀 吞吐量: {:.2} ops/sec", test_data.len() as f64 / duration.as_secs_f64());
    println!("  📦 证明大小: ~{} bytes", proof_size);
    
    metrics.push(OperationMetrics {
        ads_type: "AccTrie".to_string(),
        operation: "Query".to_string(),
        total_ops: success_count,
        total_time_ms: duration.as_millis(),
        avg_time_us: duration.as_micros() as f64 / test_data.len() as f64,
        throughput_ops_sec: test_data.len() as f64 / duration.as_secs_f64(),
        proof_size_estimate_bytes: Some(proof_size),
    });
    
    // 3. 删除性能测试
    println!("\n[3/4] 删除性能测试");
    println!("----------------------------------------");
    let delete_count = test_data.len() / 10;
    let start = Instant::now();
    let mut deleted = 0;
    
    for (key, value) in test_data.iter().take(delete_count) {
        if trie.delete(key, Some(*value)).is_ok() {
            deleted += 1;
        }
    }
    
    let duration = start.elapsed();
    
    println!("  ✓ 删除 {}/{} 条记录", deleted, delete_count);
    println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
    println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / deleted.max(1) as f64);
    println!("  🚀 吞吐量: {:.2} ops/sec", deleted as f64 / duration.as_secs_f64().max(0.001));
    
    metrics.push(OperationMetrics {
        ads_type: "AccTrie".to_string(),
        operation: "Delete".to_string(),
        total_ops: deleted,
        total_time_ms: duration.as_millis(),
        avg_time_us: duration.as_micros() as f64 / deleted.max(1) as f64,
        throughput_ops_sec: deleted as f64 / duration.as_secs_f64().max(0.001),
        proof_size_estimate_bytes: None,
    });
    
    // 4. 证明验证性能测试
    println!("\n[4/4] 证明验证性能测试");
    println!("----------------------------------------");
    
    // 选择中间的一个键进行验证测试
    let verify_idx = test_data.len() / 2;
    if verify_idx < test_data.len() {
        let (key, value) = &test_data[verify_idx];
        
        if let Ok(result) = trie.query(key, *value) {
            // 使用audit_query来验证
            let iterations = 100; // 验证100次
            let start = Instant::now();
            
            for _ in 0..iterations {
                let _ = AccTrie::audit_query(&result);
            }
            
            let duration = start.elapsed();
            
            println!("  ✓ 验证 {} 次查询证明", iterations);
            println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
            println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / iterations as f64);
            println!("  🚀 吞吐量: {:.2} verifications/sec", iterations as f64 / duration.as_secs_f64());
            
            metrics.push(OperationMetrics {
                ads_type: "AccTrie".to_string(),
                operation: "Verify".to_string(),
                total_ops: iterations,
                total_time_ms: duration.as_millis(),
                avg_time_us: duration.as_micros() as f64 / iterations as f64,
                throughput_ops_sec: iterations as f64 / duration.as_secs_f64(),
                proof_size_estimate_bytes: Some(proof_size),
            });
        }
    }
    
    metrics
}

// ================================================================================================
// MEST 性能测试
// ================================================================================================

fn benchmark_mest(config: &TestConfig) -> Vec<OperationMetrics> {
    println!("\n╔════════════════════════════════════════╗");
    println!("║       MEST 性能基准测试                ║");
    println!("╚════════════════════════════════════════╝");
    
    let mut metrics = Vec::new();
    let test_data = generate_test_data(config);
    let meht = MEHT::new_simple(16, 100, 8);
    
    // 1. 插入性能测试
    println!("\n[1/4] 插入性能测试");
    println!("----------------------------------------");
    let start = Instant::now();
    
    for (key, value) in &test_data {
        let key_str = String::from_utf8_lossy(key).to_string();
        let value_str = value.to_string();
        let kv = MestKVPair {
            key: key_str,
            value: value_str,
        };
        meht.read().unwrap().insert(kv);
    }
    
    let duration = start.elapsed();
    let proof_size = estimate_mest_key_proof_size();
    
    println!("  ✓ 插入 {} 条记录", test_data.len());
    println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
    println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / test_data.len() as f64);
    println!("  🚀 吞吐量: {:.2} ops/sec", test_data.len() as f64 / duration.as_secs_f64());
    println!("  📦 证明大小: ~{} bytes", proof_size);
    
    metrics.push(OperationMetrics {
        ads_type: "MEST".to_string(),
        operation: "Insert".to_string(),
        total_ops: test_data.len(),
        total_time_ms: duration.as_millis(),
        avg_time_us: duration.as_micros() as f64 / test_data.len() as f64,
        throughput_ops_sec: test_data.len() as f64 / duration.as_secs_f64(),
        proof_size_estimate_bytes: Some(proof_size),
    });
    
    // 2. 查询性能测试
    println!("\n[2/4] 查询性能测试");
    println!("----------------------------------------");
    let start = Instant::now();
    let mut success_count = 0;
    
    for (key, _) in &test_data {
        let key_str = String::from_utf8_lossy(key).to_string();
        if meht.read().unwrap().query(&key_str).is_some() {
            success_count += 1;
        }
    }
    
    let duration = start.elapsed();
    
    println!("  ✓ 查询 {} 条记录 (成功: {})", test_data.len(), success_count);
    println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
    println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / test_data.len() as f64);
    println!("  🚀 吞吐量: {:.2} ops/sec", test_data.len() as f64 / duration.as_secs_f64());
    println!("  📦 证明大小: ~{} bytes", proof_size);
    
    metrics.push(OperationMetrics {
        ads_type: "MEST".to_string(),
        operation: "Query".to_string(),
        total_ops: success_count,
        total_time_ms: duration.as_millis(),
        avg_time_us: duration.as_micros() as f64 / test_data.len() as f64,
        throughput_ops_sec: test_data.len() as f64 / duration.as_secs_f64(),
        proof_size_estimate_bytes: Some(proof_size),
    });
    
    // 3. 删除性能测试
    println!("\n[3/4] 删除性能测试");
    println!("----------------------------------------");
    let delete_count = test_data.len() / 10;
    let start = Instant::now();
    
    for (key, value) in test_data.iter().take(delete_count) {
        let key_str = String::from_utf8_lossy(key).to_string();
        let value_str = value.to_string();
        meht.read().unwrap().delete(&key_str, &value_str);
    }
    
    let duration = start.elapsed();
    
    println!("  ✓ 删除 {} 条记录", delete_count);
    println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
    println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / delete_count as f64);
    println!("  🚀 吞吐量: {:.2} ops/sec", delete_count as f64 / duration.as_secs_f64());
    
    metrics.push(OperationMetrics {
        ads_type: "MEST".to_string(),
        operation: "Delete".to_string(),
        total_ops: delete_count,
        total_time_ms: duration.as_millis(),
        avg_time_us: duration.as_micros() as f64 / delete_count as f64,
        throughput_ops_sec: delete_count as f64 / duration.as_secs_f64(),
        proof_size_estimate_bytes: None,
    });
    
    // 4. 证明验证性能测试
    println!("\n[4/4] 证明验证性能测试");
    println!("----------------------------------------");
    
    let verify_idx = test_data.len() / 2;
    if verify_idx < test_data.len() {
        let (key, _) = &test_data[verify_idx];
        let key_str = String::from_utf8_lossy(key).to_string();
        
        if let Some(proof) = meht.read().unwrap().query(&key_str) {
            let iterations = 1000;
            let start = Instant::now();
            
            for _ in 0..iterations {
                verify_key_proof(&proof);
            }
            
            let duration = start.elapsed();
            
            println!("  ✓ 验证 {} 次查询证明", iterations);
            println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
            println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / iterations as f64);
            println!("  🚀 吞吐量: {:.2} verifications/sec", iterations as f64 / duration.as_secs_f64());
            
            metrics.push(OperationMetrics {
                ads_type: "MEST".to_string(),
                operation: "Verify".to_string(),
                total_ops: iterations,
                total_time_ms: duration.as_millis(),
                avg_time_us: duration.as_micros() as f64 / iterations as f64,
                throughput_ops_sec: iterations as f64 / duration.as_secs_f64(),
                proof_size_estimate_bytes: Some(proof_size),
            });
        }
    }
    
    metrics
}

// ================================================================================================
// MPT 性能测试
// ================================================================================================

fn benchmark_mpt(config: &TestConfig) -> Vec<OperationMetrics> {
    println!("\n╔════════════════════════════════════════╗");
    println!("║        MPT 性能基准测试                ║");
    println!("╚════════════════════════════════════════╝");
    
    let mut metrics = Vec::new();
    let test_data = generate_test_data(config);
    
    // 使用内存数据库（统一测试环境）
    let mut db = MemoryDatabase::new();
    let mut mpt = MPT::new(None);
    
    // 1. 插入性能测试
    println!("\n[1/4] 插入性能测试");
    println!("----------------------------------------");
    let start = Instant::now();
    
    for (key, value) in &test_data {
        let key_str = String::from_utf8_lossy(key).to_string();
        let value_str = value.to_string();
        let kv = MptKVPair::new(key_str, value_str);
        let _ = mpt.insert(kv, &mut db, true, false);
    }
    
    let duration = start.elapsed();
    let proof_size = estimate_mpt_proof_size();
    
    println!("  ✓ 插入 {} 条记录", test_data.len());
    println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
    println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / test_data.len() as f64);
    println!("  🚀 吞吐量: {:.2} ops/sec", test_data.len() as f64 / duration.as_secs_f64());
    println!("  📦 证明大小: ~{} bytes", proof_size);
    
    metrics.push(OperationMetrics {
        ads_type: "MPT".to_string(),
        operation: "Insert".to_string(),
        total_ops: test_data.len(),
        total_time_ms: duration.as_millis(),
        avg_time_us: duration.as_micros() as f64 / test_data.len() as f64,
        throughput_ops_sec: test_data.len() as f64 / duration.as_secs_f64(),
        proof_size_estimate_bytes: Some(proof_size),
    });
    
    // 2. 查询性能测试
    println!("\n[2/4] 查询性能测试");
    println!("----------------------------------------");
    let start = Instant::now();
    let mut success_count = 0;
    
    for (key, _) in &test_data {
        let key_str = String::from_utf8_lossy(key).to_string();
        if mpt.query_by_key(&key_str, &mut db).is_ok() {
            success_count += 1;
        }
    }
    
    let duration = start.elapsed();
    
    println!("  ✓ 查询 {} 条记录 (成功: {})", test_data.len(), success_count);
    println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
    println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / test_data.len() as f64);
    println!("  🚀 吞吐量: {:.2} ops/sec", test_data.len() as f64 / duration.as_secs_f64());
    println!("  📦 证明大小: ~{} bytes", proof_size);
    
    metrics.push(OperationMetrics {
        ads_type: "MPT".to_string(),
        operation: "Query".to_string(),
        total_ops: success_count,
        total_time_ms: duration.as_millis(),
        avg_time_us: duration.as_micros() as f64 / test_data.len() as f64,
        throughput_ops_sec: test_data.len() as f64 / duration.as_secs_f64(),
        proof_size_estimate_bytes: Some(proof_size),
    });
    
    // 3. 删除性能测试
    println!("\n[3/4] 删除性能测试");
    println!("----------------------------------------");
    let delete_count = test_data.len() / 10;
    let start = Instant::now();
    
    for (key, _) in test_data.iter().take(delete_count) {
        let key_str = String::from_utf8_lossy(key).to_string();
        let _ = mpt.delete(&key_str, &mut db);
    }
    
    let duration = start.elapsed();
    
    println!("  ✓ 删除 {} 条记录", delete_count);
    println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
    println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / delete_count as f64);
    println!("  🚀 吞吐量: {:.2} ops/sec", delete_count as f64 / duration.as_secs_f64());
    
    metrics.push(OperationMetrics {
        ads_type: "MPT".to_string(),
        operation: "Delete".to_string(),
        total_ops: delete_count,
        total_time_ms: duration.as_millis(),
        avg_time_us: duration.as_micros() as f64 / delete_count as f64,
        throughput_ops_sec: delete_count as f64 / duration.as_secs_f64(),
        proof_size_estimate_bytes: None,
    });
    
    // 4. 证明验证性能测试
    println!("\n[4/4] 证明验证性能测试");
    println!("----------------------------------------");
    
    // 选择一个未被删除的键进行验证测试
    let verify_idx = test_data.len() / 2 + delete_count; // 避开已删除的键
    if verify_idx < test_data.len() {
        let (key, _) = &test_data[verify_idx];
        let key_str = String::from_utf8_lossy(key).to_string();
        
        // 先查询获取证明
        if let Ok((value, proof)) = mpt.query_by_key(&key_str, &mut db) {
            let iterations = 1000;
            let start = Instant::now();
            
            for _ in 0..iterations {
                let _ = mpt.verify_query_result(&value, &proof);
            }
            
            let duration = start.elapsed();
            
            println!("  ✓ 验证 {} 次查询证明", iterations);
            println!("  ⏱  总时间: {:.2} ms", duration.as_millis());
            println!("  📊 平均延迟: {:.2} μs", duration.as_micros() as f64 / iterations as f64);
            println!("  🚀 吞吐量: {:.2} verifications/sec", iterations as f64 / duration.as_secs_f64());
            
            metrics.push(OperationMetrics {
                ads_type: "MPT".to_string(),
                operation: "Verify".to_string(),
                total_ops: iterations,
                total_time_ms: duration.as_millis(),
                avg_time_us: duration.as_micros() as f64 / iterations as f64,
                throughput_ops_sec: iterations as f64 / duration.as_secs_f64(),
                proof_size_estimate_bytes: Some(proof_size),
            });
        }
    }
    
    metrics
}

// ================================================================================================
// 报告生成
// ================================================================================================

fn generate_comparison_report(all_metrics: &[OperationMetrics], config: &TestConfig) {
    println!("\n\n");
    println!("╔══════════════════════════════════════════════════════════════════════════╗");
    println!("║                     性能对比分析报告                                      ║");
    println!("╚══════════════════════════════════════════════════════════════════════════╝");
    
    println!("\n📋 测试配置:");
    println!("  • 操作数量: {}", config.num_operations);
    println!("  • 键前缀: {}", config.key_prefix);
    println!("  • 值基数: {}", config.value_base);
    
    // 按操作类型分组对比
    let operations = vec!["Insert", "Query", "Delete", "Verify"];
    
    for op in &operations {
        let op_metrics: Vec<_> = all_metrics.iter()
            .filter(|m| m.operation == *op)
            .collect();
        
        if op_metrics.is_empty() {
            continue;
        }
        
        println!("\n");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  {} 操作性能对比", op);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        
        println!("\n{:<12} {:>12} {:>15} {:>18} {:>18}", 
            "ADS类型", "操作数", "总时间(ms)", "平均延迟(μs)", "吞吐量(ops/s)");
        println!("{}", "─".repeat(78));
        
        for m in &op_metrics {
            println!("{:<12} {:>12} {:>15.2} {:>18.2} {:>18.2}", 
                m.ads_type, m.total_ops, m.total_time_ms, m.avg_time_us, m.throughput_ops_sec);
        }
        
        // 证明大小对比
        if op == &"Insert" || op == &"Query" {
            println!("\n{:<12} {:>25}", "ADS类型", "证明大小估算(bytes)");
            println!("{}", "─".repeat(40));
            for m in &op_metrics {
                if let Some(size) = m.proof_size_estimate_bytes {
                    println!("{:<12} {:>25}", m.ads_type, size);
                }
            }
        }
    }
    
    println!("\n");
    println!("═══════════════════════════════════════════════════════════════════════════");
    println!("  报告生成时间: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    println!("═══════════════════════════════════════════════════════════════════════════");
}

// ================================================================================================
// 测试入口
// ================================================================================================

#[test]
fn test_ads_performance_benchmark() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Warn)
        .try_init();
    
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════════════╗");
    println!("║                                                                          ║");
    println!("║         三种 ADS 系统完整性能基准测试                                     ║");
    println!("║         AccTrie vs MEST vs MPT                                           ║");
    println!("║                                                                          ║");
    println!("╚══════════════════════════════════════════════════════════════════════════╝");
    
    let config = TestConfig {
        num_operations: 1000,
        key_prefix: "key_".to_string(),
        value_base: 1000,
    };
    
    println!("\n⚙️  初始化测试环境...");
    println!("  • 测试规模: {} 条记录", config.num_operations);
    
    let mut all_metrics = Vec::new();
    
    // 测试 AccTrie
    all_metrics.extend(benchmark_acctrie(&config));
    
    // 测试 MEST
    all_metrics.extend(benchmark_mest(&config));
    
    // 测试 MPT
    all_metrics.extend(benchmark_mpt(&config));
    
    // 生成对比报告
    generate_comparison_report(&all_metrics, &config);
    
    // 保存详细报告
    let report = BenchmarkReport {
        timestamp: chrono::Local::now().to_rfc3339(),
        config,
        metrics: all_metrics,
    };
    
    let report_json = serde_json::to_string_pretty(&report).unwrap();
    std::fs::write("ads_performance_benchmark_report.json", report_json).unwrap();
    
    println!("\n✅ 详细报告已保存: ads_performance_benchmark_report.json");
    println!("\n✓ 所有性能测试完成!");
}

#[test]
fn test_ads_large_scale_benchmark() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Warn)
        .try_init();
    
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════════════╗");
    println!("║                    大规模性能压力测试 (10,000 条记录)                     ║");
    println!("╚══════════════════════════════════════════════════════════════════════════╝");
    
    let config = TestConfig {
        num_operations: 10000,
        key_prefix: "large_key_".to_string(),
        value_base: 10000,
    };
    
    let mut all_metrics = Vec::new();
    
    all_metrics.extend(benchmark_acctrie(&config));
    all_metrics.extend(benchmark_mest(&config));
    all_metrics.extend(benchmark_mpt(&config));
    
    generate_comparison_report(&all_metrics, &config);
    
    println!("\n✓ 大规模测试完成!");
}
