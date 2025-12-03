//! AccTrie 完整证明验证测试
//! 
//! 测试完整的密码学证明验证，包括插入、删除和更新操作的审计验证

use esa_rust::acctrie::acc::DynamicAccumulator;
use esa_rust::acctrie::trie::AccTrie;

#[test]
fn test_insertion_proof_verification() {
    let mut trie = AccTrie::new();
    let mut root_acc = DynamicAccumulator::new();
    
    // 插入第一个键值对
    let key1 = b"apple".to_vec();
    let value1 = 100i64;
    let proof1 = trie.insert(key1.clone(), value1).unwrap();
    
    // 验证插入证明
    let result1 = trie.audit_insertion(&proof1, &mut root_acc).unwrap();
    assert!(result1.valid, "First insertion proof should be valid");
    
    // 插入第二个键值对
    let key2 = b"cherry".to_vec();
    let value2 = 200i64;
    let proof2 = trie.insert(key2.clone(), value2).unwrap();
    
    // 验证插入证明
    let result2 = trie.audit_insertion(&proof2, &mut root_acc).unwrap();
    assert!(result2.valid, "Second insertion proof should be valid");
    
    // 插入第三个键值对（在中间）- banana 在 apple 和 cherry 之间
    let key3 = b"banana".to_vec();
    let value3 = 150i64;
    let proof3 = trie.insert(key3.clone(), value3).unwrap();
    
    // 验证插入证明（应该有前序和后序节点）
    let result3 = trie.audit_insertion(&proof3, &mut root_acc).unwrap();
    if !result3.valid {
        println!("Validation error: {:?}", result3.error);
    }
    assert!(result3.valid, "Third insertion proof should be valid");
    
    // 验证证明包含必要的成员证明
    assert!(proof3.key_prev.is_some(), "Should have previous key");
    assert!(proof3.key_next.is_some(), "Should have next key");
    assert!(proof3.keyp_in_ln_proof.is_some(), "Should have keyp membership proof in LN");
    assert!(proof3.keyp_in_ln_next_old_proof.is_some(), "Should have keyp membership proof in old LNn");
    assert!(proof3.key_in_ln_next_new_proof.is_some(), "Should have key membership proof in new LNn");
    assert!(proof3.value_in_ln_proof.is_some(), "Should have value membership proof");
}

#[test]
fn test_deletion_proof_verification() {
    let mut trie = AccTrie::new();
    let mut root_acc = DynamicAccumulator::new();
    
    // 插入三个键值对
    let key1 = b"apple".to_vec();
    let key2 = b"banana".to_vec();
    let key3 = b"cherry".to_vec();
    
    let proof1 = trie.insert(key1.clone(), 100).unwrap();
    let proof2 = trie.insert(key2.clone(), 200).unwrap();
    let proof3 = trie.insert(key3.clone(), 300).unwrap();
    
    trie.audit_insertion(&proof1, &mut root_acc).unwrap();
    trie.audit_insertion(&proof2, &mut root_acc).unwrap();
    trie.audit_insertion(&proof3, &mut root_acc).unwrap();
    
    // 删除中间的键（整个叶子节点）
    let del_proof = trie.delete(&key2, None).unwrap();
    
    // 验证删除证明
    let result = trie.audit_deletion(&del_proof, &mut root_acc).unwrap();
    assert!(result.valid, "Deletion proof should be valid");
    
    // 验证证明包含必要的成员证明
    assert!(del_proof.delete_entire_leaf, "Should delete entire leaf");
    assert!(del_proof.key_prev.is_some(), "Should have previous key");
    assert!(del_proof.key_next.is_some(), "Should have next key");
    assert!(del_proof.keyp_in_ln_proof.is_some(), "Should have keyp membership proof in LN");
    assert!(del_proof.key_in_ln_next_old_proof.is_some(), "Should have key membership proof in old LNn");
    assert!(del_proof.keyp_in_ln_next_new_proof.is_some(), "Should have keyp membership proof in new LNn");
}

#[test]
fn test_partial_deletion_proof_verification() {
    let mut trie = AccTrie::new();
    let mut root_acc = DynamicAccumulator::new();
    
    // 插入键值对
    let key = b"apple".to_vec();
    let value1 = 100i64;
    let value2 = 200i64;
    
    let proof1 = trie.insert(key.clone(), value1).unwrap();
    let proof2 = trie.insert(key.clone(), value2).unwrap();
    
    trie.audit_insertion(&proof1, &mut root_acc).unwrap();
    trie.audit_insertion(&proof2, &mut root_acc).unwrap();
    
    // 部分删除（只删除一个值）
    let del_proof = trie.delete(&key, Some(value1)).unwrap();
    
    // 验证删除证明
    let result = trie.audit_deletion(&del_proof, &mut root_acc).unwrap();
    assert!(result.valid, "Partial deletion proof should be valid");
    
    // 验证证明包含必要的成员证明
    assert!(!del_proof.delete_entire_leaf, "Should not delete entire leaf");
    assert!(del_proof.value_in_ln_old_proof.is_some(), "Should have value membership proof in old LN");
    assert!(del_proof.ln_acc_new.is_some(), "Should have new accumulator value");
}

#[test]
fn test_update_proof_verification() {
    let mut trie = AccTrie::new();
    let mut root_acc = DynamicAccumulator::new();
    
    // 插入键值对
    let key = b"apple".to_vec();
    let old_value = 100i64;
    let new_value = 200i64;
    
    let insert_proof = trie.insert(key.clone(), old_value).unwrap();
    trie.audit_insertion(&insert_proof, &mut root_acc).unwrap();
    
    // 更新值
    let update_proof = trie.update(&key, old_value, new_value).unwrap();
    
    // 验证更新证明
    let result = trie.audit_update(&update_proof, &mut root_acc).unwrap();
    assert!(result.valid, "Update proof should be valid");
    
    // 验证证明包含必要的证明
    assert!(update_proof.delete_value_proof.is_some(), "Should have delete proof");
    assert!(update_proof.add_value_proof.is_some(), "Should have add proof");
    
    // 验证删除和添加证明的有效性
    let delete_proof = update_proof.delete_value_proof.as_ref().unwrap();
    let add_proof = update_proof.add_value_proof.as_ref().unwrap();
    
    assert!(delete_proof.verify(), "Delete proof should verify");
    assert!(add_proof.verify(), "Add proof should verify");
}

#[test]
fn test_query_existence_proof() {
    let mut trie = AccTrie::new();
    
    // 插入键值对
    let key = b"apple".to_vec();
    let value = 100i64;
    
    trie.insert(key.clone(), value).unwrap();
    
    // 查询存在的值
    let query_result = trie.query(&key, value).unwrap();
    
    // 验证查询结果
    let audit_result = AccTrie::audit_query(&query_result).unwrap();
    assert!(audit_result.valid, "Query existence proof should be valid");
    
    // 验证是存在证明
    match query_result {
        esa_rust::acctrie::trie::QueryResult::Exists(proof) => {
            assert_eq!(proof.key, key);
            assert_eq!(proof.value, value);
            assert!(proof.membership_proof.is_some(), "Should have membership proof");
        }
        _ => panic!("Expected existence proof"),
    }
}

#[test]
fn test_query_non_existence_proof() {
    let mut trie = AccTrie::new();
    
    // 插入两个键值对
    let key1 = b"apple".to_vec();
    let key2 = b"cherry".to_vec();
    let value = 100i64;
    
    trie.insert(key1.clone(), value).unwrap();
    trie.insert(key2.clone(), value).unwrap();
    
    // 查询不存在的键（在中间）
    let query_key = b"banana".to_vec();
    let query_result = trie.query(&query_key, value).unwrap();
    
    // 验证查询结果
    let audit_result = AccTrie::audit_query(&query_result).unwrap();
    assert!(audit_result.valid, "Query non-existence proof should be valid");
    
    // 验证是不存在证明
    match query_result {
        esa_rust::acctrie::trie::QueryResult::NotExists(proof) => {
            assert_eq!(proof.key, query_key);
            assert!(proof.key_prev.is_some(), "Should have previous key");
            assert!(proof.key_next.is_some(), "Should have next key");
            assert_eq!(proof.key_prev.unwrap(), key1);
            assert_eq!(proof.key_next.unwrap(), key2);
            assert!(proof.ln_next_acc.is_some(), "Should have next accumulator");
            assert!(proof.prev_in_next_proof.is_some(), "Should have prev membership proof");
        }
        _ => panic!("Expected non-existence proof"),
    }
}

#[test]
fn test_complete_workflow_with_proofs() {
    let mut trie = AccTrie::new();
    let mut root_acc = DynamicAccumulator::new();
    
    // 1. 插入多个键值对
    let keys = vec![
        (b"apple".to_vec(), 100i64),
        (b"banana".to_vec(), 200i64),
        (b"cherry".to_vec(), 300i64),
        (b"date".to_vec(), 400i64),
    ];
    
    for (key, value) in &keys {
        let proof = trie.insert(key.clone(), *value).unwrap();
        let result = trie.audit_insertion(&proof, &mut root_acc).unwrap();
        assert!(result.valid, "Insertion proof should be valid for key {:?}", key);
    }
    
    // 2. 查询存在的值
    for (key, value) in &keys {
        let query_result = trie.query(key, *value).unwrap();
        let audit_result = AccTrie::audit_query(&query_result).unwrap();
        assert!(audit_result.valid, "Query proof should be valid for key {:?}", key);
    }
    
    // 3. 更新一个值
    let update_key = &keys[1].0;
    let old_value = keys[1].1;
    let new_value = 250i64;
    
    let update_proof = trie.update(update_key, old_value, new_value).unwrap();
    let result = trie.audit_update(&update_proof, &mut root_acc).unwrap();
    assert!(result.valid, "Update proof should be valid");
    
    // 4. 删除一个叶子节点
    let delete_key = &keys[2].0;
    let del_proof = trie.delete(delete_key, None).unwrap();
    let result = trie.audit_deletion(&del_proof, &mut root_acc).unwrap();
    assert!(result.valid, "Deletion proof should be valid");
    
    // 5. 验证删除后查询不存在
    let query_result = trie.query(delete_key, keys[2].1).unwrap();
    let audit_result = AccTrie::audit_query(&query_result).unwrap();
    assert!(audit_result.valid, "Non-existence proof should be valid");
    
    match query_result {
        esa_rust::acctrie::trie::QueryResult::NotExists(_) => {
            // 正确，值不存在
        }
        _ => panic!("Expected non-existence proof after deletion"),
    }
}
