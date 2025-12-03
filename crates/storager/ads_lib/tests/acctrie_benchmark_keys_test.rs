/// 测试AccTrie在benchmark键格式下的行为
use ads_rust::acctrie::AccTrie;

#[test]
fn test_benchmark_key_format() {
    let mut trie = AccTrie::new();

    // 使用和benchmark相同的键格式
    let keys: Vec<Vec<u8>> = (0..10)
        .map(|i| format!("test{:08}", i).into_bytes())
        .collect();

    println!("=== 插入10个键 ===");
    for (i, key) in keys.iter().enumerate() {
        let value = (i + 1) as i64 * 100;
        println!("{}. 插入: {:?}", i + 1, String::from_utf8_lossy(key));
        let result = trie.insert(key.clone(), value);
        assert!(result.is_ok(), "插入失败: {:?}", result.err());
    }

    println!("\n=== 测试find_leaf ===");
    let mut found = 0;
    for (i, key) in keys.iter().enumerate() {
        let result = trie.find_leaf(key);
        if result.is_some() {
            found += 1;
            println!("  ✓ 键{}: 找到", i + 1);
        } else {
            println!(
                "  ❌ 键{}: 未找到 - {:?}",
                i + 1,
                String::from_utf8_lossy(key)
            );
        }
    }
    println!("\n找到: {}/{}", found, keys.len());

    println!("\n=== 测试删除 ===");
    let mut deleted = 0;
    for (i, key) in keys.iter().enumerate() {
        let value = (i + 1) as i64 * 100;
        let result = trie.delete(key, Some(value));
        if result.is_ok() {
            deleted += 1;
            println!("  ✓ 键{}: 删除成功", i + 1);
        } else {
            println!("  ❌ 键{}: 删除失败 - {}", i + 1, result.err().unwrap());
        }
    }
    println!("\n删除成功: {}/{}", deleted, keys.len());
}
