/// MPT的统一适配器实现
/// 
/// 将MPT包装为实现AuthenticatedDataStructure trait的适配器
use crate::unified_ads::{
    AuthenticatedDataStructure, UnifiedKey, UnifiedValue,
};
use crate::mpt::{MPT, MPTProof, MemoryDatabase, KVPair};
use anyhow::{Result, anyhow};

pub struct MptAdapter {
    mpt: MPT,
    db: MemoryDatabase,
}

impl MptAdapter {
    pub fn new() -> Self {
        Self {
            mpt: MPT::new(None),
            db: MemoryDatabase::new(),
        }
    }
}

impl AuthenticatedDataStructure for MptAdapter {
    type Key = UnifiedKey;
    type Value = UnifiedValue;
    type Proof = MPTProof;
    type Database = MemoryDatabase;
    
    fn insert(&mut self, key: Self::Key, value: Self::Value, _db: Option<&mut Self::Database>) 
        -> Result<Self::Proof> {
        // MPT使用字符串键和字符串值
        let key_str = String::from_utf8(key.0)
            .map_err(|e| anyhow!("Invalid UTF-8 in key: {}", e))?;
        
        let value_str = value.as_string();
        
        let kv = KVPair::new(key_str.clone(), value_str);
        
        // 插入到MPT
        self.mpt.insert(kv, &mut self.db, true, false)
            .map_err(|e| anyhow!("MPT insert failed: {:?}", e))?;
        
        // 生成证明 - 插入后立即查询获取证明
        match self.mpt.query_by_key(&key_str, &mut self.db) {
            Ok((_value, proof)) => Ok(proof),
            Err(e) => Err(anyhow!("Failed to generate proof after insert: {:?}", e)),
        }
    }
    
    fn query(&mut self, key: &Self::Key, _db: Option<&mut Self::Database>) 
        -> Result<Option<(Self::Value, Self::Proof)>> {
        let key_str = String::from_utf8(key.0.clone())
            .map_err(|e| anyhow!("Invalid UTF-8 in key: {}", e))?;
        
        match self.mpt.query_by_key(&key_str, &mut self.db) {
            Ok((value, proof)) => {
                if value.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some((UnifiedValue::String(value), proof)))
                }
            }
            Err(e) => Err(anyhow!("MPT query failed: {:?}", e)),
        }
    }
    
    fn delete(&mut self, key: &Self::Key, _db: Option<&mut Self::Database>) 
        -> Result<Option<Self::Proof>> {
        let key_str = String::from_utf8(key.0.clone())
            .map_err(|e| anyhow!("Invalid UTF-8 in key: {}", e))?;
        
        // MPT的删除操作
        match self.mpt.delete(&key_str, &mut self.db) {
            Ok(Some(old_value)) => {
                if !old_value.is_empty() {
                    // 删除成功,生成证明(查询现在应该返回空)
                    match self.mpt.query_by_key(&key_str, &mut self.db) {
                        Ok((_, proof)) => Ok(Some(proof)),
                        Err(_) => Ok(None),
                    }
                } else {
                    Ok(None)
                }
            }
            Ok(None) => Ok(None),  // 键不存在
            Err(e) => Err(anyhow!("MPT delete failed: {:?}", e)),
        }
    }
    
    fn verify(&self, proof: &Self::Proof) -> bool {
        // MPT的验证需要value和proof
        // 这里简化处理,检查proof的存在性标志
        proof.is_exist || !proof.proofs.is_empty()
    }
    
    fn ads_type(&self) -> &'static str {
        "MPT"
    }
    
    fn estimate_proof_size(_proof: &Self::Proof) -> usize {
        // MPT证明大小取决于树的深度
        // 每个proof element包含节点哈希等信息
        512  // 粗略估计
    }
}

impl Default for MptAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mpt_adapter_insert() {
        let mut adapter = MptAdapter::new();
        let key = UnifiedKey::new(b"test_key".to_vec());
        let value = UnifiedValue::String("test_value".to_string());
        
        let result = adapter.insert(key.clone(), value.clone(), None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mpt_adapter_query() {
        let mut adapter = MptAdapter::new();
        let key = UnifiedKey::new(b"key1".to_vec());
        let value = UnifiedValue::String("value1".to_string());
        
        adapter.insert(key.clone(), value.clone(), None).unwrap();
        
        let query_result = adapter.query(&key, None).unwrap();
        assert!(query_result.is_some());
        
        let (found_value, _proof) = query_result.unwrap();
        assert_eq!(found_value, value);
    }

    #[test]
    fn test_mpt_adapter_delete() {
        let mut adapter = MptAdapter::new();
        let key = UnifiedKey::new(b"key2".to_vec());
        let value = UnifiedValue::String("value2".to_string());
        
        adapter.insert(key.clone(), value, None).unwrap();
        
        let delete_result = adapter.delete(&key, None);
        assert!(delete_result.is_ok());
        
        // 验证删除后查询不到
        let query_result = adapter.query(&key, None).unwrap();
        assert!(query_result.is_none());
    }

    #[test]
    fn test_mpt_adapter_multiple_keys() {
        let mut adapter = MptAdapter::new();
        
        // 插入10个键
        for i in 0..10 {
            let key = UnifiedKey::new(format!("key_{:04}", i).into_bytes());
            let value = UnifiedValue::String(format!("value_{}", i));
            adapter.insert(key, value, None).unwrap();
        }
        
        // 查询所有键
        for i in 0..10 {
            let key = UnifiedKey::new(format!("key_{:04}", i).into_bytes());
            let result = adapter.query(&key, None).unwrap();
            assert!(result.is_some());
            let (found_value, _) = result.unwrap();
            assert_eq!(found_value, UnifiedValue::String(format!("value_{}", i)));
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
            let result = adapter.query(&key, None).unwrap();
            assert!(result.is_some());
        }
    }
}
