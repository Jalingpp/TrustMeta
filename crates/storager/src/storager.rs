use crate::ads::{AdsOperations, MptAds, MestAds};
use std::sync::{Arc, RwLock};

/// Storager 结构
///
/// 负责管理单个存储节点的 ADS 实例
pub struct Storager {
    pub(crate) ads: Arc<RwLock<Box<dyn AdsOperations>>>,
}

impl Storager {
    /// 创建新的 Storager 实例（默认使用 MEST）
    pub fn new() -> Self {
        Self::with_mest()
    }

    /// 使用 Merkle Patricia Trie 创建实例
    pub fn with_mpt() -> Self {
        let ads: Box<dyn AdsOperations> = Box::new(MptAds::new());
        Storager {
            ads: Arc::new(RwLock::new(ads)),
        }
    }

    /// 使用 MEST (Merkle-based Extendible Segmented Hash Tree) 创建实例
    pub fn with_mest() -> Self {
        let ads: Box<dyn AdsOperations> = Box::new(MestAds::new_default());
        Storager {
            ads: Arc::new(RwLock::new(ads)),
        }
    }

    /// 根据配置字符串创建实例
    ///
    /// # Arguments
    /// * `ads_type` - ADS 类型: "mpt" 或 "mest"
    ///
    /// # Examples
    /// ```
    /// let storager = Storager::from_config("mpt");
    /// ```
    pub fn from_config(ads_type: &str) -> Self {
        match ads_type.to_lowercase().as_str() {
            "mpt" => Self::with_mpt(),
            "mest" => Self::with_mest(),
            _ => {
                eprintln!(
                    "Unknown ADS type '{}', using default (MEST)",
                    ads_type
                );
                Self::with_mest()
            }
        }
    }
}

impl Default for Storager {
    fn default() -> Self {
        Self::new()
    }
}
