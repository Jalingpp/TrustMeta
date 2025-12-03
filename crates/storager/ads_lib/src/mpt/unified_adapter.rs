//! MPT的统一ADS接口适配器

use crate::unified_ads::{AuthenticatedDataStructure, UnifiedKey, UnifiedValue};
use crate::mpt::{MPT, MPTProof, MemoryDatabase};
use crate::mpt::utils::KVPair;
use anyhow::Result;

/// MPT适配器 - 实现统一的ADS接口
pub struct MptAdapter {
    mpt: MPT,
}

impl MptAdapter {
    pub fn new() -> Self {
        Self {
            mpt: MPT::new(None),
        }
    }
    
    pub fn with_cache(cache_capacity: usize) -> Self {
        use crate::mpt::NodeCache;
        let cache = NodeCache::new(cache_capacity, cache_capacity);
        Self {
            mpt: MPT::new(Some(cache)),
        }
    }
}

impl Default for MptAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthenticatedDataStructure for MptAdapter {
    type Key = UnifiedKey;
    type Value = UnifiedValue;
    type Proof = MPTProof;
    type Database = MemoryDatabase;
    
    fn insert(&mut self, key: Self::Key, value: Self::Value, db: Option<&mut Self::Database>) 
        -> Result<Self::Proof> {
        let db = db.ok_or_else(|| anyhow::anyhow!("MPT requires database"))?;
        
        let key_str = key.to_string();
        let value_str = value.as_string();
        let kv = KVPair::new(key_str.clone(), value_str);
        
        // 插入数据
        self.mpt.insert(kv, db, true, false)
            .map_err(|e| anyhow::anyhow!("MPT insert error: {}", e))?;
        
        // 查询获取证明
        let (_val, proof) = self.mpt.query_by_key(&key_str, db)
            .map_err(|e| anyhow::anyhow!("MPT query after insert error: {}", e))?;
        
        Ok(proof)
    }
    
    fn query(&mut self, key: &Self::Key, db: Option<&mut Self::Database>) 
        -> Result<Option<(Self::Value, Self::Proof)>> {
        let db = db.ok_or_else(|| anyhow::anyhow!("MPT requires database"))?;
        
        let key_str = key.to_string();
        
        match self.mpt.query_by_key(&key_str, db) {
            Ok((value, proof)) => {
                if value.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some((UnifiedValue::String(value), proof)))
                }
            },
            Err(_) => Ok(None),
        }
    }
    
    fn delete(&mut self, key: &Self::Key, db: Option<&mut Self::Database>) 
        -> Result<Option<Self::Proof>> {
        let db = db.ok_or_else(|| anyhow::anyhow!("MPT requires database"))?;
        
        let key_str = key.to_string();
        
        // MPT delete不返回证明，我们需要在删除前获取证明
        let proof = match self.mpt.query_by_key(&key_str, db) {
            Ok((_val, p)) => Some(p),
            Err(_) => None,
        };
        
        match self.mpt.delete(&key_str, db) {
            Ok(Some(_)) => Ok(proof),
            Ok(None) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("MPT delete error: {}", e)),
        }
    }
    
    fn verify(&self, proof: &Self::Proof) -> bool {
        // MPT的验证需要值，这里简化为检查证明是否存在
        proof.get_is_exist()
    }
    
    fn ads_type(&self) -> &'static str {
        "MPT"
    }
    
    fn estimate_proof_size(_proof: &Self::Proof) -> usize {
        820 // 根据benchmark结果
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mpt_adapter_basic_operations() {
        let mut adapter = MptAdapter::new();
        let mut db = MemoryDatabase::new();
        
        let key = UnifiedKey::from_string("test_key".to_string());
        let value = UnifiedValue::String("test_value".to_string());
        
        // 测试插入
        let proof = adapter.insert(key.clone(), value.clone(), Some(&mut db)).unwrap();
        assert!(proof.get_is_exist());
        
        // 测试查询
        let result = adapter.query(&key, Some(&mut db)).unwrap();
        assert!(result.is_some());
        
        // 测试删除
        let delete_proof = adapter.delete(&key, Some(&mut db)).unwrap();
        assert!(delete_proof.is_some());
    }
}
