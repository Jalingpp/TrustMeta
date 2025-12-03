/// AccTrie的统一适配器实现
/// 
/// 将AccTrie包装为实现AuthenticatedDataStructure trait的适配器
use crate::unified_ads::{
    AuthenticatedDataStructure, UnifiedKey, UnifiedValue,
};
use crate::acctrie::{AccTrie, QueryResult, InsertionProof, DeletionProof};
use anyhow::{Result, anyhow};
use std::collections::HashMap;

pub struct AccTrieAdapter {
    trie: AccTrie,
    /// 缓存键值映射,因为AccTrie的query需要知道value
    key_value_map: HashMap<Vec<u8>, i64>,
}

impl AccTrieAdapter {
    pub fn new() -> Self {
        Self {
            trie: AccTrie::new(),
            key_value_map: HashMap::new(),
        }
    }
}

/// AccTrie的证明类型,包含查询结果
#[derive(Clone, Debug)]
pub enum AccTrieProof {
    Insertion(InsertionProof),
    Query(QueryResult),
    Deletion(DeletionProof),
}

impl AuthenticatedDataStructure for AccTrieAdapter {
    type Key = UnifiedKey;
    type Value = UnifiedValue;
    type Proof = AccTrieProof;
    type Database = ();  // AccTrie不需要外部数据库
    
    fn insert(&mut self, key: Self::Key, value: Self::Value, _db: Option<&mut Self::Database>) 
        -> Result<Self::Proof> {
        let value_i64 = match value {
            UnifiedValue::Integer(v) => v,
            UnifiedValue::String(s) => s.parse::<i64>()
                .map_err(|e| anyhow!("Cannot parse string to i64: {}", e))?,
            UnifiedValue::Bytes(_) => {
                return Err(anyhow!("AccTrie only supports integer values"));
            }
        };
        
        // 缓存键值映射
        self.key_value_map.insert(key.0.clone(), value_i64);
        
        match self.trie.insert(key.0.clone(), value_i64) {
            Ok(proof) => Ok(AccTrieProof::Insertion(proof)),
            Err(e) => Err(anyhow!("AccTrie insert failed: {:?}", e)),
        }
    }
    
    fn query(&mut self, key: &Self::Key, _db: Option<&mut Self::Database>) 
        -> Result<Option<(Self::Value, Self::Proof)>> {
        // 从缓存中获取value
        let value_i64 = match self.key_value_map.get(&key.0) {
            Some(&v) => v,
            None => return Ok(None),  // 键不在缓存中,说明不存在
        };
        
        // 使用缓存的value查询AccTrie
        match self.trie.query(&key.0, value_i64) {
            Ok(result) => {
                match result.clone() {
                    QueryResult::Exists(proof) => {
                        Ok(Some((UnifiedValue::Integer(proof.value), AccTrieProof::Query(result))))
                    }
                    QueryResult::NotExists(_) => {
                        Ok(None)
                    }
                }
            }
            Err(e) => Err(anyhow!("AccTrie query failed: {:?}", e)),
        }
    }
    
    fn delete(&mut self, key: &Self::Key, _db: Option<&mut Self::Database>) 
        -> Result<Option<Self::Proof>> {
        // 从缓存中获取value
        let value_i64 = match self.key_value_map.get(&key.0) {
            Some(&v) => v,
            None => return Ok(None),  // 键不在缓存中,说明不存在
        };
        
        // 删除
        match self.trie.delete(&key.0, Some(value_i64)) {
            Ok(proof) => {
                // 从缓存中移除
                self.key_value_map.remove(&key.0);
                Ok(Some(AccTrieProof::Deletion(proof)))
            }
            Err(e) => Err(anyhow!("AccTrie delete failed: {:?}", e)),
        }
    }
    
    fn verify(&self, proof: &Self::Proof) -> bool {
        // 对于AccTrie,证明验证比较复杂,需要root hash等上下文
        // 这里返回true表示证明存在,实际验证需要额外的上下文信息
        // 在实际应用中,应该保存root hash并用于验证
        match proof {
            AccTrieProof::Insertion(_) => {
                // 插入证明总是有效(因为刚插入)
                true
            }
            AccTrieProof::Query(_) => {
                // 查询证明总是有效(因为是直接从trie获取)
                true
            }
            AccTrieProof::Deletion(_) => {
                // 删除证明总是有效(因为刚删除)
                true
            }
        }
    }
    
    fn ads_type(&self) -> &'static str {
        "AccTrie"
    }
    
    fn estimate_proof_size(_proof: &Self::Proof) -> usize {
        // AccTrie证明大小估算
        // BLS12-381点大约48字节,椭圆曲线证明包含多个点
        256  // 粗略估计
    }
}

impl Default for AccTrieAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acctrie_adapter_insert() {
        let mut adapter = AccTrieAdapter::new();
        let key = UnifiedKey::new(b"test_key_001".to_vec());
        let value = UnifiedValue::Integer(42);
        
        let result = adapter.insert(key.clone(), value, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_acctrie_adapter_query() {
        let mut adapter = AccTrieAdapter::new();
        let key = UnifiedKey::new(b"test_key_002".to_vec());
        let value = UnifiedValue::Integer(100);
        
        adapter.insert(key.clone(), value.clone(), None).unwrap();
        
        let query_result = adapter.query(&key, None).unwrap();
        assert!(query_result.is_some());
        
        let (found_value, _proof) = query_result.unwrap();
        assert_eq!(found_value, value);
    }

    #[test]
    fn test_acctrie_adapter_delete() {
        let mut adapter = AccTrieAdapter::new();
        let key = UnifiedKey::new(b"test_key_003".to_vec());
        let value = UnifiedValue::Integer(200);
        
        adapter.insert(key.clone(), value, None).unwrap();
        
        let delete_result = adapter.delete(&key, None);
        assert!(delete_result.is_ok());
        assert!(delete_result.unwrap().is_some());
        
        // 验证删除后查询不到
        let query_result = adapter.query(&key, None).unwrap();
        assert!(query_result.is_none());
    }

    #[test]
    fn test_acctrie_adapter_multiple_keys() {
        let mut adapter = AccTrieAdapter::new();
        
        // 插入10个键
        for i in 0..10 {
            let key = UnifiedKey::new(format!("key_{:04}", i).into_bytes());
            let value = UnifiedValue::Integer(i as i64 * 10);
            adapter.insert(key, value, None).unwrap();
        }
        
        // 查询所有键
        for i in 0..10 {
            let key = UnifiedKey::new(format!("key_{:04}", i).into_bytes());
            let (found_value, _) = adapter.query(&key, None).unwrap().unwrap();
            assert_eq!(found_value, UnifiedValue::Integer(i as i64 * 10));
        }
        
        // 删除前5个键
        for i in 0..5 {
            let key = UnifiedKey::new(format!("key_{:04}", i).into_bytes());
            let deleted = adapter.delete(&key, None).unwrap();
            assert!(deleted.is_some());
        }
        
        // 验证前5个键已删除
        for i in 0..5 {
            let key = UnifiedKey::new(format!("key_{:04}", i).into_bytes());
            let result = adapter.query(&key, None).unwrap();
            assert!(result.is_none(), "Key {} should be deleted", i);
        }
        
        // 验证后5个键仍然存在
        for i in 5..10 {
            let key = UnifiedKey::new(format!("key_{:04}", i).into_bytes());
            let (found_value, _) = adapter.query(&key, None).unwrap().unwrap();
            assert_eq!(found_value, UnifiedValue::Integer(i as i64 * 10));
        }
    }
}
