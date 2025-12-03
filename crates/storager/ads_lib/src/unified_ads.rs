//! 统一的认证数据结构(ADS)抽象接口
//!
//! 提供三种ADS实现的统一操作接口，便于在不同场景下灵活选择和切换
//! 
//! ## 设计目标
//! - 统一的操作接口 (insert, query, delete, verify)
//! - 类型安全的证明系统
//! - 支持内存数据库的统一存储层
//! - 性能开销最小化

use std::fmt::Debug;
use anyhow::Result;

// ================================================================================================
// 核心Trait定义
// ================================================================================================

/// 统一的ADS操作接口
/// 
/// 所有ADS实现都应该实现此trait，提供一致的操作方法
pub trait AuthenticatedDataStructure {
    /// 键类型 - 不同ADS可能使用不同的键类型
    type Key: Clone + Debug;
    
    /// 值类型 - 不同ADS可能使用不同的值类型
    type Value: Clone + Debug;
    
    /// 证明类型 - 每种ADS有自己的证明结构
    type Proof: Clone + Debug;
    
    /// 数据库类型 - 统一使用Database trait
    type Database;
    
    /// 插入键值对
    /// 
    /// # 参数
    /// - `key`: 要插入的键
    /// - `value`: 要插入的值
    /// - `db`: 数据库引用 (某些ADS需要)
    /// 
    /// # 返回
    /// - 插入操作的证明
    fn insert(&mut self, key: Self::Key, value: Self::Value, db: Option<&mut Self::Database>) 
        -> Result<Self::Proof>;
    
    /// 查询键对应的值
    /// 
    /// # 参数
    /// - `key`: 要查询的键
    /// - `db`: 数据库引用 (某些ADS需要)
    /// 
    /// # 返回
    /// - Some((value, proof)): 查询成功，返回值和证明
    /// - None: 键不存在
    fn query(&mut self, key: &Self::Key, db: Option<&mut Self::Database>) 
        -> Result<Option<(Self::Value, Self::Proof)>>;
    
    /// 删除键值对
    /// 
    /// # 参数
    /// - `key`: 要删除的键
    /// - `db`: 数据库引用 (某些ADS需要)
    /// 
    /// # 返回
    /// - Some(proof): 删除成功，返回证明
    /// - None: 键不存在
    fn delete(&mut self, key: &Self::Key, db: Option<&mut Self::Database>) 
        -> Result<Option<Self::Proof>>;
    
    /// 验证证明的有效性
    /// 
    /// # 参数
    /// - `proof`: 要验证的证明
    /// 
    /// # 返回
    /// - true: 证明有效
    /// - false: 证明无效
    fn verify(&self, proof: &Self::Proof) -> bool;
    
    /// 获取ADS类型名称
    fn ads_type(&self) -> &'static str;
    
    /// 估算证明大小（字节）
    fn estimate_proof_size(proof: &Self::Proof) -> usize;
}

// ================================================================================================
// ADS选择器 - 工厂模式
// ================================================================================================

/// ADS类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdsType {
    /// Merkle Patricia Trie - 最快的插入/删除，最小的证明
    MPT,
    /// Merkle Extendible Segmented Hash Tree - 平衡的性能
    MEST,
    /// Accumulator-based Trie - 最快的查询/验证
    AccTrie,
}

impl AdsType {
    /// 获取ADS类型的描述
    pub fn description(&self) -> &'static str {
        match self {
            AdsType::MPT => "Merkle Patricia Trie - 高吞吐写入，紧凑证明",
            AdsType::MEST => "Merkle Segmented Hash Tree - 平衡读写性能",
            AdsType::AccTrie => "Accumulator Trie - 超快查询验证",
        }
    }
    
    /// 根据工作负载特征推荐ADS类型
    pub fn recommend(write_heavy: bool, read_heavy: bool, proof_size_critical: bool) -> Self {
        if write_heavy && !read_heavy {
            AdsType::MPT
        } else if read_heavy && !write_heavy {
            AdsType::AccTrie
        } else if proof_size_critical {
            AdsType::MPT
        } else {
            AdsType::MEST
        }
    }
}

// ================================================================================================
// 统一的键值对类型
// ================================================================================================

/// 统一的键类型 - 内部使用字节数组
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnifiedKey(pub Vec<u8>);

impl UnifiedKey {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    
    pub fn from_string(s: String) -> Self {
        Self(s.into_bytes())
    }
    
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    
    pub fn to_string(&self) -> String {
        String::from_utf8_lossy(&self.0).to_string()
    }
}

/// 统一的值类型 - 支持多种值表示
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedValue {
    /// 整数值 (用于AccTrie)
    Integer(i64),
    /// 字符串值 (用于MPT/MEST)
    String(String),
    /// 字节数组值
    Bytes(Vec<u8>),
}

impl UnifiedValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            UnifiedValue::Integer(v) => Some(*v),
            UnifiedValue::String(s) => s.parse().ok(),
            UnifiedValue::Bytes(_) => None,
        }
    }
    
    pub fn as_string(&self) -> String {
        match self {
            UnifiedValue::Integer(v) => v.to_string(),
            UnifiedValue::String(s) => s.clone(),
            UnifiedValue::Bytes(b) => String::from_utf8_lossy(b).to_string(),
        }
    }
    
    pub fn as_bytes(&self) -> Vec<u8> {
        match self {
            UnifiedValue::Integer(v) => v.to_string().into_bytes(),
            UnifiedValue::String(s) => s.clone().into_bytes(),
            UnifiedValue::Bytes(b) => b.clone(),
        }
    }
}

// ================================================================================================
// 性能指标收集
// ================================================================================================

/// 操作性能指标
#[derive(Debug, Clone)]
pub struct OperationMetrics {
    pub operation_type: String,
    pub duration_micros: u64,
    pub proof_size_bytes: usize,
    pub success: bool,
}

/// ADS性能监控器
pub struct PerformanceMonitor {
    metrics: Vec<OperationMetrics>,
}

impl PerformanceMonitor {
    pub fn new() -> Self {
        Self {
            metrics: Vec::new(),
        }
    }
    
    pub fn record(&mut self, metric: OperationMetrics) {
        self.metrics.push(metric);
    }
    
    pub fn get_metrics(&self) -> &[OperationMetrics] {
        &self.metrics
    }
    
    pub fn average_latency(&self, operation: &str) -> Option<f64> {
        let ops: Vec<_> = self.metrics.iter()
            .filter(|m| m.operation_type == operation)
            .collect();
        
        if ops.is_empty() {
            return None;
        }
        
        let total: u64 = ops.iter().map(|m| m.duration_micros).sum();
        Some(total as f64 / ops.len() as f64)
    }
}

impl Default for PerformanceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unified_key() {
        let key = UnifiedKey::from_string("test_key".to_string());
        assert_eq!(key.to_string(), "test_key");
    }

    #[test]
    fn test_unified_value() {
        let val1 = UnifiedValue::Integer(42);
        assert_eq!(val1.as_i64(), Some(42));
        assert_eq!(val1.as_string(), "42");

        let val2 = UnifiedValue::String("hello".to_string());
        assert_eq!(val2.as_string(), "hello");
    }

    #[test]
    fn test_ads_type_recommendation() {
        assert_eq!(AdsType::recommend(true, false, false), AdsType::MPT);
        assert_eq!(AdsType::recommend(false, true, false), AdsType::AccTrie);
        assert_eq!(AdsType::recommend(false, false, true), AdsType::MPT);
    }
}
