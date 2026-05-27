//! # Consistent Hash Ring
//!
//! 高性能的零依赖一致性哈希环实现。
//!
//! ## 特性
//!
//! - ✅ 标准的一致性哈希环算法
//! - ✅ 虚拟节点支持，实现负载均衡
//! - ✅ 动态添加/删除节点
//! - ✅ 最小化数据迁移
//! - ✅ 线程安全（支持 Arc + RwLock）
//! - ⚡ **零依赖核心** - 仅依赖 xxhash-rust
//! - ⚡ **零拷贝查询** - `get_node(&str) -> Option<&str>`，无内存分配
//! - ⚡ **O(log n) 查找** - 基于二分查找
//! - ⚡ **xxHash3 算法** - 比 SipHash 快 10 倍
//! - 🛡️ **输入验证** - 防止节点名称包含分隔符
//!
//! ## 快速开始
//!
//! ```rust
//! use manager::ConsistentHashRing;
//!
//! // 创建一个包含3个节点的哈希环，每个节点150个虚拟节点
//! let mut ring = ConsistentHashRing::new();
//! ring.add_node("node1", 150).unwrap();
//! ring.add_node("node2", 150).unwrap();
//! ring.add_node("node3", 150).unwrap();
//!
//! // 零拷贝查询
//! let node = ring.get_node("my_key");
//! assert!(node.is_some());
//! ```
//!
//! ## 实现原理
//!
//! ### 数据结构
//! - `Vec<(u64, String)>`: 存储 (哈希值, 虚拟节点名) 的有序列表
//! - 虚拟节点名格式: `"node_name#vnodeN"`
//!
//! ### 查找流程
//! 1. 计算 key 的哈希值 (使用 SipHash)
//! 2. 二分查找第一个 >= hash 的虚拟节点
//! 3. 从虚拟节点名中解析物理节点名（零拷贝）
//!
//! ### 性能优化
//! - ⚡ `get_node()` 返回 `&str` 而非 `String` - 零内存分配
//! - ⚡ 使用 `binary_search_by_key()` - O(log n) 查找
//! - ⚡ Copy-On-Write 更新 - 减少锁竞争

use std::collections::HashMap;
use xxhash_rust::xxh3::xxh3_64;

/// 添加节点时的错误类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddNodeError {
    /// 节点名称无效（例如包含分隔符 '#'）
    InvalidNodeName(String),
    /// 节点已存在
    NodeAlreadyExists(String),
}

impl std::fmt::Display for AddNodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddNodeError::InvalidNodeName(msg) => write!(f, "无效的节点名称: {}", msg),
            AddNodeError::NodeAlreadyExists(node) => write!(f, "节点已存在: {}", node),
        }
    }
}

impl std::error::Error for AddNodeError {}

/// 一致性哈希环
///
/// 使用虚拟节点实现负载均衡的一致性哈希环。
/// 零依赖实现，支持零拷贝查询。
///
/// # 示例
///
/// ```
/// use manager::ConsistentHashRing;
///
/// let mut ring = ConsistentHashRing::new();
/// ring.add_node("server1", 100).unwrap();
/// ring.add_node("server2", 100).unwrap();
///
/// let node = ring.get_node("user123").unwrap();
/// println!("user123 应该路由到: {}", node);
/// ```
#[derive(Debug, Clone)]
pub struct ConsistentHashRing {
    /// 哈希环：(hash, virtual_node_key) 的有序列表
    /// virtual_node_key 格式: "node_name#vnodeN"
    /// 保持按 hash 升序排列
    ring: Vec<(u64, String)>,

    /// 节点及其虚拟节点数量: node_name -> virtual_node_count
    nodes: HashMap<String, usize>,
}

impl ConsistentHashRing {
    /// 创建一个新的空哈希环
    ///
    /// # 示例
    ///
    /// ```
    /// use manager::ConsistentHashRing;
    ///
    /// let ring = ConsistentHashRing::new();
    /// ```
    pub fn new() -> Self {
        ConsistentHashRing {
            ring: Vec::new(),
            nodes: HashMap::new(),
        }
    }

    /// 使用默认虚拟节点数创建哈希环并添加节点
    ///
    /// # 参数
    ///
    /// * `node_names` - 节点名称列表
    /// * `virtual_nodes_per_node` - 每个节点的虚拟节点数量（推荐 100-200）
    ///
    /// # 示例
    ///
    /// ```
    /// use manager::ConsistentHashRing;
    ///
    /// let nodes = vec!["node1", "node2", "node3"];
    /// let ring = ConsistentHashRing::with_nodes(&nodes, 150).unwrap();
    /// ```
    pub fn with_nodes(
        node_names: &[&str],
        virtual_nodes_per_node: usize,
    ) -> Result<Self, AddNodeError> {
        let mut ring = Self::new();
        for name in node_names {
            ring.add_node(name, virtual_nodes_per_node)?;
        }
        Ok(ring)
    }

    /// 计算字符串的哈希值
    ///
    /// 使用 xxHash3 算法（比 SipHash 快 10 倍）
    fn hash_key(key: &str) -> u64 {
        xxh3_64(key.as_bytes())
    }

    /// 添加一个节点到哈希环
    ///
    /// # 参数
    ///
    /// * `node_name` - 节点名称（必须唯一，不能包含 '#' 字符）
    /// * `virtual_nodes` - 该节点的虚拟节点数量
    ///
    /// # 返回
    ///
    /// * `Ok(())` - 成功添加
    /// * `Err(AddNodeError)` - 添加失败（节点已存在或名称无效）
    ///
    /// # 示例
    ///
    /// ```
    /// use manager::ConsistentHashRing;
    ///
    /// let mut ring = ConsistentHashRing::new();
    /// assert!(ring.add_node("node1", 150).is_ok());
    /// assert!(ring.add_node("node1", 150).is_err()); // 重复添加失败
    /// assert!(ring.add_node("bad#name", 150).is_err()); // 包含 # 失败
    /// ```
    pub fn add_node(&mut self, node_name: &str, virtual_nodes: usize) -> Result<(), AddNodeError> {
        // 🛡️ 输入验证：禁止节点名称包含分隔符 '#'
        if node_name.contains('#') {
            return Err(AddNodeError::InvalidNodeName(
                "节点名称不能包含 '#' 字符，避免虚拟节点名称冲突".to_string(),
            ));
        }

        // 检查节点是否已存在
        if self.nodes.contains_key(node_name) {
            return Err(AddNodeError::NodeAlreadyExists(node_name.to_string()));
        }

        // ⚡ Copy-On-Write: 先在临时向量中准备所有虚拟节点
        let mut new_vnodes = Vec::with_capacity(virtual_nodes);
        for i in 0..virtual_nodes {
            let vnode_key = format!("{}#vnode{}", node_name, i);
            let hash = Self::hash_key(&vnode_key);
            new_vnodes.push((hash, vnode_key));
        }

        // 克隆现有环并合并新节点
        let mut new_ring = self.ring.clone();
        new_ring.extend(new_vnodes);

        // 排序以保持环的有序性
        new_ring.sort_by_key(|(hash, _)| *hash);

        // 原子替换（单次赋值，极短的锁时间）
        self.ring = new_ring;

        // 记录节点信息
        self.nodes.insert(node_name.to_string(), virtual_nodes);

        Ok(())
    }

    /// 从哈希环中移除一个节点
    ///
    /// # 参数
    ///
    /// * `node_name` - 要移除的节点名称
    ///
    /// # 返回
    ///
    /// * `true` - 成功移除
    /// * `false` - 节点不存在
    ///
    /// # 示例
    ///
    /// ```
    /// use manager::ConsistentHashRing;
    ///
    /// let mut ring = ConsistentHashRing::new();
    /// ring.add_node("node1", 150).unwrap();
    /// assert!(ring.remove_node("node1"));
    /// assert!(!ring.remove_node("node1")); // 已经移除
    /// ```
    pub fn remove_node(&mut self, node_name: &str) -> bool {
        // 检查节点是否存在
        if !self.nodes.remove(node_name).is_some() {
            return false;
        }

        // ⚡ Copy-On-Write: 过滤掉该节点的所有虚拟节点
        let prefix = format!("{}#", node_name);
        self.ring
            .retain(|(_, vnode_key)| !vnode_key.starts_with(&prefix));

        true
    }

    /// 获取键应该路由到的节点（零拷贝）
    ///
    /// # 参数
    ///
    /// * `key` - 要查找的键
    ///
    /// # 返回
    ///
    /// * `Some(&str)` - 找到的节点名称（引用，无内存分配）
    /// * `None` - 环为空
    ///
    /// # 示例
    ///
    /// ```
    /// use manager::ConsistentHashRing;
    ///
    /// let mut ring = ConsistentHashRing::new();
    /// ring.add_node("node1", 150).unwrap();
    /// ring.add_node("node2", 150).unwrap();
    ///
    /// let node = ring.get_node("my_key");
    /// assert!(node.is_some());
    /// ```
    pub fn get_node(&self, key: &str) -> Option<&str> {
        if self.ring.is_empty() {
            return None;
        }

        // ⚡ 零拷贝：直接对 &str 计算哈希，无需 to_string()
        let hash = Self::hash_key(key);

        // 使用二分查找找到第一个 >= hash 的虚拟节点
        let idx = match self.ring.binary_search_by_key(&hash, |(h, _)| *h) {
            Ok(i) => i,                    // 精确匹配
            Err(i) => i % self.ring.len(), // 顺时针下一个（如果超出末尾则回到开头）
        };

        // 解析虚拟节点名称，提取物理节点名
        let vnode_key = &self.ring[idx].1;

        // rsplit_once('#') 从右边找到第一个 '#'，返回左边的部分
        // ⚡ 零拷贝：返回的是对 String 内部切片的引用
        vnode_key.rsplit_once('#').map(|(node_name, _)| node_name)
    }

    /// 获取多个副本节点
    ///
    /// 对于需要数据冗余的场景，返回多个不同的物理节点。
    ///
    /// # 参数
    ///
    /// * `key` - 要查找的键
    /// * `count` - 需要的副本数量
    ///
    /// # 返回
    ///
    /// 返回最多 `count` 个不同的物理节点（去重）
    ///
    /// # 示例
    ///
    /// ```
    /// use manager::ConsistentHashRing;
    ///
    /// let mut ring = ConsistentHashRing::new();
    /// ring.add_node("node1", 150).unwrap();
    /// ring.add_node("node2", 150).unwrap();
    /// ring.add_node("node3", 150).unwrap();
    ///
    /// // 获取3个副本节点
    /// let replicas = ring.get_nodes("my_key", 3);
    /// assert!(replicas.len() >= 1);
    /// ```
    pub fn get_nodes(&self, key: &str, count: usize) -> Vec<String> {
        if self.ring.is_empty() || count == 0 {
            return Vec::new();
        }

        let hash = Self::hash_key(key);
        let start_idx = match self.ring.binary_search_by_key(&hash, |(h, _)| *h) {
            Ok(i) => i,
            Err(i) => i % self.ring.len(),
        };

        let mut result = Vec::new();
        let mut seen_nodes = std::collections::HashSet::new();

        // 从 start_idx 开始顺时针遍历环
        for i in 0..self.ring.len() {
            let idx = (start_idx + i) % self.ring.len();
            let vnode_key = &self.ring[idx].1;

            if let Some((node_name, _)) = vnode_key.rsplit_once('#') {
                // 只添加未见过的物理节点
                if seen_nodes.insert(node_name) {
                    result.push(node_name.to_string());
                    if result.len() >= count {
                        break;
                    }
                }
            }
        }

        result
    }

    /// 获取环中所有物理节点的名称
    pub fn get_all_nodes(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }

    /// 获取环中物理节点的数量
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 获取环中虚拟节点的总数
    pub fn virtual_node_count(&self) -> usize {
        self.ring.len()
    }

    /// 获取指定节点的虚拟节点数量
    pub fn get_virtual_node_count(&self, node_name: &str) -> Option<usize> {
        self.nodes.get(node_name).copied()
    }

    /// 检查环是否为空
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 计算键在节点间的分布情况
    pub fn get_distribution(&self, keys: &[String]) -> HashMap<String, usize> {
        let mut distribution = HashMap::new();

        for key in keys {
            if let Some(node) = self.get_node(key) {
                *distribution.entry(node.to_string()).or_insert(0) += 1;
            }
        }

        distribution
    }
}

impl Default for ConsistentHashRing {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_empty_ring() {
        let ring = ConsistentHashRing::new();
        assert!(ring.is_empty());
        assert_eq!(ring.node_count(), 0);
        assert_eq!(ring.virtual_node_count(), 0);
    }

    #[test]
    fn test_add_node() {
        let mut ring = ConsistentHashRing::new();
        assert!(ring.add_node("node1", 100).is_ok());
        assert_eq!(ring.node_count(), 1);
        assert_eq!(ring.virtual_node_count(), 100);

        // 重复添加应该失败
        let result = ring.add_node("node1", 100);
        assert!(result.is_err());
        assert!(matches!(result, Err(AddNodeError::NodeAlreadyExists(_))));
    }

    #[test]
    fn test_reject_invalid_node_names() {
        let mut ring = ConsistentHashRing::new();

        // 🛡️ 应该拒绝包含 # 的节点名
        let result = ring.add_node("server#1", 100);
        assert!(result.is_err());
        assert!(matches!(result, Err(AddNodeError::InvalidNodeName(_))));

        // 正常的节点名应该成功
        assert!(ring.add_node("server-1", 100).is_ok());
        assert!(ring.add_node("server_2", 100).is_ok());
        assert!(ring.add_node("server.3", 100).is_ok());
    }

    #[test]
    fn test_remove_node() {
        let mut ring = ConsistentHashRing::new();
        ring.add_node("node1", 100).unwrap();
        assert!(ring.remove_node("node1"));
        assert!(ring.is_empty());

        // 移除不存在的节点应该失败
        assert!(!ring.remove_node("node2"));
    }

    #[test]
    fn test_get_node() {
        let mut ring = ConsistentHashRing::new();
        ring.add_node("node1", 150).unwrap();
        ring.add_node("node2", 150).unwrap();
        ring.add_node("node3", 150).unwrap();

        let node = ring.get_node("test_key");
        assert!(node.is_some());
        assert!(["node1", "node2", "node3"].contains(&node.unwrap()));
    }

    #[test]
    fn test_get_node_empty_ring() {
        let ring = ConsistentHashRing::new();
        assert!(ring.get_node("test_key").is_none());
    }

    #[test]
    fn test_consistent_mapping() {
        let mut ring = ConsistentHashRing::new();
        ring.add_node("node1", 150).unwrap();
        ring.add_node("node2", 150).unwrap();

        // 同一个键应该总是映射到同一个节点
        let node1 = ring.get_node("test_key").unwrap();
        let node2 = ring.get_node("test_key").unwrap();
        assert_eq!(node1, node2);
    }

    #[test]
    fn test_get_nodes_replicas() {
        let mut ring = ConsistentHashRing::new();
        ring.add_node("node1", 150).unwrap();
        ring.add_node("node2", 150).unwrap();
        ring.add_node("node3", 150).unwrap();

        let replicas = ring.get_nodes("test_key", 2);
        assert_eq!(replicas.len(), 2);

        // 确保副本是不同的节点
        assert_ne!(replicas[0], replicas[1]);
    }

    #[test]
    fn test_get_nodes_all_replicas() {
        let mut ring = ConsistentHashRing::new();
        ring.add_node("node1", 50).unwrap();
        ring.add_node("node2", 50).unwrap();
        ring.add_node("node3", 50).unwrap();

        // 请求 3 个副本，应该返回所有 3 个节点
        let replicas = ring.get_nodes("test_key", 3);
        assert_eq!(replicas.len(), 3);

        // 确保所有节点都不同
        let unique: std::collections::HashSet<_> = replicas.iter().collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn test_with_nodes() {
        let nodes = vec!["node1", "node2", "node3"];
        let ring = ConsistentHashRing::with_nodes(&nodes, 100).unwrap();

        assert_eq!(ring.node_count(), 3);
        assert_eq!(ring.virtual_node_count(), 300);
    }

    #[test]
    fn test_distribution() {
        let mut ring = ConsistentHashRing::new();
        ring.add_node("node1", 150).unwrap();
        ring.add_node("node2", 150).unwrap();
        ring.add_node("node3", 150).unwrap();

        // 生成1000个测试键
        let keys: Vec<String> = (0..1000).map(|i| format!("key{}", i)).collect();
        let distribution = ring.get_distribution(&keys);

        println!("Distribution: {:?}", distribution);

        // 每个节点应该得到一部分键
        assert_eq!(distribution.len(), 3);
        for (node, count) in distribution.iter() {
            println!(
                "{}: {} keys ({:.1}%)",
                node,
                count,
                (*count as f64 / 1000.0 * 100.0)
            );
        }

        // 检查分布的均衡性（允许一定偏差）
        let avg = 1000 / 3;
        for count in distribution.values() {
            let diff = (*count as i32 - avg as i32).abs();
            let deviation = diff as f64 / avg as f64;
            assert!(
                deviation < 0.3,
                "Distribution too unbalanced: {}",
                deviation
            );
        }
    }

    #[test]
    fn test_node_addition_minimal_disruption() {
        let mut ring = ConsistentHashRing::new();
        ring.add_node("node1", 150).unwrap();
        ring.add_node("node2", 150).unwrap();

        // 记录添加前的映射
        let keys: Vec<String> = (0..1000).map(|i| format!("key{}", i)).collect();
        let before: Vec<_> = keys
            .iter()
            .map(|k| ring.get_node(k).map(|s| s.to_string()))
            .collect();

        // 添加新节点
        ring.add_node("node3", 150).unwrap();

        // 记录添加后的映射
        let after: Vec<_> = keys
            .iter()
            .map(|k| ring.get_node(k).map(|s| s.to_string()))
            .collect();

        // 计算变化的键数量
        let changed = before
            .iter()
            .zip(after.iter())
            .filter(|(b, a)| b != a)
            .count();

        let change_ratio = changed as f64 / keys.len() as f64;
        println!(
            "Changed: {} / {} ({:.1}%)",
            changed,
            keys.len(),
            change_ratio * 100.0
        );

        // 理论上应该只有 1/3 的键需要迁移（从2个节点变成3个节点）
        // 允许一定误差
        assert!(
            change_ratio < 0.5,
            "Too many keys changed: {:.1}%",
            change_ratio * 100.0
        );
    }

    #[test]
    fn test_virtual_node_count() {
        let mut ring = ConsistentHashRing::new();
        ring.add_node("node1", 100).unwrap();
        ring.add_node("node2", 200).unwrap();

        assert_eq!(ring.get_virtual_node_count("node1"), Some(100));
        assert_eq!(ring.get_virtual_node_count("node2"), Some(200));
        assert_eq!(ring.get_virtual_node_count("node3"), None);
    }

    #[test]
    fn test_zero_copy_get_node() {
        let mut ring = ConsistentHashRing::new();
        ring.add_node("node1", 100).unwrap();

        // ⚡ 测试返回的是引用，无内存分配
        let node_ref = ring.get_node("test_key").unwrap();

        // 验证是 &str 类型
        let _: &str = node_ref;

        // 如果需要 String，调用方可以手动调用 to_string()
        let _node_owned: String = node_ref.to_string();
    }
}
