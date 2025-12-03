/// 统一ADS接口演示
/// 
/// 展示如何使用统一的接口操作三种不同的ADS实现

use ads_rust::unified_ads::{AuthenticatedDataStructure, UnifiedKey, UnifiedValue};
use ads_rust::mest::MestAdapter;
use ads_rust::acctrie::AccTrieAdapter;

fn main() {
    println!("=== 统一ADS接口演示 ===\n");
    
    // 演示1: MEST适配器
    println!("1. MEST适配器:");
    demo_mest();
    
    println!("\n");
    
    // 演示2: AccTrie适配器
    println!("2. AccTrie适配器:");
    demo_acctrie();
}

fn demo_mest() {
    let mut mest = MestAdapter::new(4, 16, 32);
    
    // 插入
    let key = UnifiedKey::new(b"user:1001".to_vec());
    let value = UnifiedValue::Integer(42);
    
    print!("  插入 {:?} = {:?} ... ", 
        String::from_utf8_lossy(key.as_bytes()), value);
    
    match mest.insert(key.clone(), value.clone(), None) {
        Ok(_) => println!("✓ 成功"),
        Err(e) => println!("✗ 失败: {}", e),
    }
    
    // 查询
    print!("  查询 {:?} ... ", String::from_utf8_lossy(key.as_bytes()));
    match mest.query(&key, None) {
        Ok(Some((found_value, _proof))) => {
            println!("✓ 找到: {:?}", found_value);
            assert_eq!(found_value, value);
        }
        Ok(None) => println!("✗ 未找到"),
        Err(e) => println!("✗ 错误: {}", e),
    }
    
    // 删除
    print!("  删除 {:?} ... ", String::from_utf8_lossy(key.as_bytes()));
    match mest.delete(&key, None) {
        Ok(Some(_)) => println!("✓ 成功"),
        Ok(None) => println!("✗ 键不存在"),
        Err(e) => println!("✗ 错误: {}", e),
    }
    
    println!("  ADS类型: {}", mest.ads_type());
}

fn demo_acctrie() {
    let mut acctrie = AccTrieAdapter::new();
    
    // 插入
    let key = UnifiedKey::new(b"account:2002".to_vec());
    let value = UnifiedValue::Integer(100);
    
    print!("  插入 {:?} = {:?} ... ", 
        String::from_utf8_lossy(key.as_bytes()), value);
    
    match acctrie.insert(key.clone(), value.clone(), None) {
        Ok(_) => println!("✓ 成功"),
        Err(e) => println!("✗ 失败: {}", e),
    }
    
    // 查询
    print!("  查询 {:?} ... ", String::from_utf8_lossy(key.as_bytes()));
    match acctrie.query(&key, None) {
        Ok(Some((found_value, _proof))) => {
            println!("✓ 找到: {:?}", found_value);
            assert_eq!(found_value, value);
        }
        Ok(None) => println!("✗ 未找到"),
        Err(e) => println!("✗ 错误: {}", e),
    }
    
    // 删除
    print!("  删除 {:?} ... ", String::from_utf8_lossy(key.as_bytes()));
    match acctrie.delete(&key, None) {
        Ok(Some(_)) => println!("✓ 成功"),
        Ok(None) => println!("✗ 键不存在"),
        Err(e) => println!("✗ 错误: {}", e),
    }
    
    // 验证删除后查询
    print!("  查询已删除的键 ... ");
    match acctrie.query(&key, None) {
        Ok(None) => println!("✓ 确认已删除"),
        Ok(Some(_)) => println!("✗ 键仍然存在!"),
        Err(e) => println!("✗ 错误: {}", e),
    }
    
    println!("  ADS类型: {}", acctrie.ads_type());
}
