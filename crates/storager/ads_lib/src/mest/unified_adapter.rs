use crate::mest::{KVPair, KeyProof, MEHT};
/// MEST的统一适配器实现
///
/// 将MEST包装为实现AuthenticatedDataStructure trait的适配器
use crate::unified_ads::{AuthenticatedDataStructure, UnifiedKey, UnifiedValue};
use anyhow::{anyhow, Result};
use std::sync::{Arc, RwLock};

pub struct MestAdapter {
    meht: Arc<RwLock<MEHT>>,
}

impl MestAdapter {
    pub fn new(radix: i32, bucket_size: i32, max_depth: i32) -> Self {
        Self {
            meht: MEHT::new_simple(radix, bucket_size, max_depth),
        }
    }
}

impl AuthenticatedDataStructure for MestAdapter {
    type Key = UnifiedKey;
    type Value = UnifiedValue;
    type Proof = KeyProof;
    type Database = (); // MEST不需要外部数据库

    fn insert(
        &mut self,
        key: Self::Key,
        value: Self::Value,
        _db: Option<&mut Self::Database>,
    ) -> Result<Self::Proof> {
        // MEST只支持字符串键和整数值
        let key_str =
            String::from_utf8(key.0).map_err(|e| anyhow!("Invalid UTF-8 in key: {}", e))?;

        let value_i64 = match value {
            UnifiedValue::Integer(v) => v,
            UnifiedValue::String(s) => s
                .parse::<i64>()
                .map_err(|e| anyhow!("Cannot parse string to i64: {}", e))?,
            UnifiedValue::Bytes(_) => {
                return Err(anyhow!("MEST does not support byte array values"));
            }
        };

        // 插入到MEST
        let kvpair = KVPair {
            key: key_str,
            value: value_i64.to_string(),
        };

        let proof = self.meht.read().unwrap().insert(kvpair);
        Ok(proof)
    }

    fn query(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> Result<Option<(Self::Value, Self::Proof)>> {
        let key_str =
            String::from_utf8(key.0.clone()).map_err(|e| anyhow!("Invalid UTF-8 in key: {}", e))?;

        match self.meht.read().unwrap().query(&key_str) {
            Some(proof) => {
                // 从证明中提取值
                let value_i64: i64 = proof
                    .bucket_proof
                    .value
                    .parse()
                    .map_err(|e| anyhow!("Cannot parse value: {}", e))?;
                Ok(Some((UnifiedValue::Integer(value_i64), proof)))
            }
            None => Ok(None),
        }
    }

    fn delete(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> Result<Option<Self::Proof>> {
        let key_str =
            String::from_utf8(key.0.clone()).map_err(|e| anyhow!("Invalid UTF-8 in key: {}", e))?;

        // 先查询获取值,因为delete需要键和值
        let value_str = match self.meht.read().unwrap().query(&key_str) {
            Some(proof) => proof.bucket_proof.value,
            None => return Ok(None), // 键不存在
        };

        // 删除
        let deleted = self.meht.write().unwrap().delete(&key_str, &value_str);

        if deleted {
            // 删除成功,返回None(MEST delete不返回证明)
            Ok(None)
        } else {
            Ok(None)
        }
    }

    fn verify(&self, proof: &Self::Proof) -> bool {
        // 使用MEST的验证函数
        use crate::mest::verify_key_proof;
        verify_key_proof(proof)
    }

    fn ads_type(&self) -> &'static str {
        "MEST"
    }

    fn estimate_proof_size(_proof: &Self::Proof) -> usize {
        // MEST证明大小估算 (包含bucket proof + MGT proof)
        // 这是一个粗略估计
        1024 // 大约1KB
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mest_adapter_insert() {
        let mut adapter = MestAdapter::new(4, 16, 32);
        let key = UnifiedKey::new(b"test_key".to_vec());
        let value = UnifiedValue::Integer(42);

        let result = adapter.insert(key.clone(), value.clone(), None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mest_adapter_query() {
        let mut adapter = MestAdapter::new(4, 16, 32);
        let key = UnifiedKey::new(b"key1".to_vec());
        let value = UnifiedValue::Integer(100);

        adapter.insert(key.clone(), value.clone(), None).unwrap();

        let query_result = adapter.query(&key, None).unwrap();
        assert!(query_result.is_some());

        let (found_value, _proof) = query_result.unwrap();
        assert_eq!(found_value, value);
    }
}
