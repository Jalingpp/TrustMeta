//! 统一ADS接口的集成测试
//!
//! 展示如何使用统一接口操作不同的ADS实现

use ads_rust::unified_ads::{AuthenticatedDataStructure, UnifiedKey, UnifiedValue, AdsType};
use ads_rust::mpt::{MptAdapter, MemoryDatabase};

#[test]
fn test_unified_mpt_operations() {
    println!("\n╔════════════════════════════════════════╗");
    println!("║   统一ADS接口测试 - MPT实现           ║");
    println!("╚════════════════════════════════════════╝\n");
    
    // 创建MPT适配器
    let mut mpt = MptAdapter::new();
    let mut db = MemoryDatabase::new();
    
    println!("✓ ADS类型: {}", mpt.ads_type());
    println!("✓ 描述: {}\n", AdsType::MPT.description());
    
    // 测试插入
    println!("[1/3] 插入操作测试");
    let key1 = UnifiedKey::from_string("user:1001".to_string());
    let value1 = UnifiedValue::String("Alice".to_string());
    
    let insert_proof = mpt.insert(key1.clone(), value1.clone(), Some(&mut db)).unwrap();
    println!("  ✓ 插入成功: {} = {}", key1.to_string(), value1.as_string());
    println!("  📦 证明大小: {} bytes\n", MptAdapter::estimate_proof_size(&insert_proof));
    
    // 测试查询
    println!("[2/3] 查询操作测试");
    let query_result = mpt.query(&key1, Some(&mut db)).unwrap();
    
    match query_result {
        Some((value, proof)) => {
            println!("  ✓ 查询成功: {}", value.as_string());
            println!("  📦 证明大小: {} bytes", MptAdapter::estimate_proof_size(&proof));
            println!("  ✅ 验证结果: {}\n", mpt.verify(&proof));
        }
        None => {
            println!("  ✗ 键不存在\n");
        }
    }
    
    // 测试删除
    println!("[3/3] 删除操作测试");
    let delete_proof = mpt.delete(&key1, Some(&mut db)).unwrap();
    
    match delete_proof {
        Some(proof) => {
            println!("  ✓ 删除成功");
            println!("  📦 证明大小: {} bytes\n", MptAdapter::estimate_proof_size(&proof));
        }
        None => {
            println!("  ✗ 键不存在\n");
        }
    }
    
    // 验证删除后查询
    let query_after_delete = mpt.query(&key1, Some(&mut db)).unwrap();
    assert!(query_after_delete.is_none(), "删除后应该查询不到");
    println!("  ✓ 验证: 删除后键不存在");
    
    println!("\n╔════════════════════════════════════════╗");
    println!("║   所有测试通过! ✓                      ║");
    println!("╚════════════════════════════════════════╝\n");
}

#[test]
fn test_ads_type_selection() {
    println!("\n╔════════════════════════════════════════╗");
    println!("║   ADS类型智能选择测试                  ║");
    println!("╚════════════════════════════════════════╝\n");
    
    // 场景1: 写密集型
    let ads1 = AdsType::recommend(true, false, false);
    println!("📊 场景1 - 写密集型工作负载");
    println!("  推荐: {:?}", ads1);
    println!("  理由: {}\n", ads1.description());
    assert_eq!(ads1, AdsType::MPT);
    
    // 场景2: 读密集型
    let ads2 = AdsType::recommend(false, true, false);
    println!("📊 场景2 - 读密集型工作负载");
    println!("  推荐: {:?}", ads2);
    println!("  理由: {}\n", ads2.description());
    assert_eq!(ads2, AdsType::AccTrie);
    
    // 场景3: 证明大小敏感
    let ads3 = AdsType::recommend(false, false, true);
    println!("📊 场景3 - 证明大小敏感场景");
    println!("  推荐: {:?}", ads3);
    println!("  理由: {}\n", ads3.description());
    assert_eq!(ads3, AdsType::MPT);
    
    // 场景4: 平衡场景
    let ads4 = AdsType::recommend(false, false, false);
    println!("📊 场景4 - 平衡读写工作负载");
    println!("  推荐: {:?}", ads4);
    println!("  理由: {}\n", ads4.description());
    assert_eq!(ads4, AdsType::MEST);
}

#[test]
fn test_unified_value_conversion() {
    println!("\n╔════════════════════════════════════════╗");
    println!("║   UnifiedValue类型转换测试             ║");
    println!("╚════════════════════════════════════════╝\n");
    
    // 整数类型
    let val1 = UnifiedValue::Integer(42);
    println!("整数值: {:?}", val1);
    println!("  as_i64(): {:?}", val1.as_i64());
    println!("  as_string(): {}", val1.as_string());
    assert_eq!(val1.as_i64(), Some(42));
    assert_eq!(val1.as_string(), "42");
    
    // 字符串类型
    let val2 = UnifiedValue::String("hello".to_string());
    println!("\n字符串值: {:?}", val2);
    println!("  as_string(): {}", val2.as_string());
    assert_eq!(val2.as_string(), "hello");
    
    // 字节数组类型
    let val3 = UnifiedValue::Bytes(vec![65, 66, 67]); // "ABC"
    println!("\n字节数组值: {:?}", val3);
    println!("  as_string(): {}", val3.as_string());
    assert_eq!(val3.as_string(), "ABC");
}

#[test]
fn test_batch_operations_unified() {
    println!("\n╔════════════════════════════════════════╗");
    println!("║   批量操作性能测试                     ║");
    println!("╚════════════════════════════════════════╝\n");
    
    let mut mpt = MptAdapter::new();
    let mut db = MemoryDatabase::new();
    
    let num_ops = 100;
    let start = std::time::Instant::now();
    
    // 批量插入
    for i in 0..num_ops {
        let key = UnifiedKey::from_string(format!("key_{}", i));
        let value = UnifiedValue::Integer(i as i64);
        mpt.insert(key, value, Some(&mut db)).unwrap();
    }
    
    let insert_duration = start.elapsed();
    println!("✓ 插入 {} 条记录", num_ops);
    println!("  耗时: {:?}", insert_duration);
    println!("  吞吐量: {:.2} ops/sec\n", num_ops as f64 / insert_duration.as_secs_f64());
    
    // 批量查询
    let start = std::time::Instant::now();
    let mut success_count = 0;
    
    for i in 0..num_ops {
        let key = UnifiedKey::from_string(format!("key_{}", i));
        if mpt.query(&key, Some(&mut db)).unwrap().is_some() {
            success_count += 1;
        }
    }
    
    let query_duration = start.elapsed();
    println!("✓ 查询 {} 条记录 (成功: {})", num_ops, success_count);
    println!("  耗时: {:?}", query_duration);
    println!("  吞吐量: {:.2} ops/sec", num_ops as f64 / query_duration.as_secs_f64());
    
    assert_eq!(success_count, num_ops);
}
