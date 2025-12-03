//! AccTrie 数据结构实现
//!
//! AccTrie 是一个结合了累加器的前缀树结构，每个叶子节点维护一个值集合及其密码学累加器。

use std::collections::HashSet;
use std::sync::{Arc, RwLock, Weak};

use anyhow::{anyhow, Result};

use crate::acctrie::acc::{DynamicAccumulator, G1Affine};
use crate::digest::Digestible;

// ================================================================================================
// Constants
// ================================================================================================

/// 内部节点的子节点数量（基于字节的完整ASCII范围）
const TRIE_FANOUT: usize = 256;

/// 表示没有前序叶子节点的特殊值
const NO_PREV: i64 = i64::MIN;

/// 表示没有后序叶子节点的特殊值
const NO_NEXT: i64 = i64::MAX;

// ================================================================================================
// Type Aliases
// ================================================================================================

/// 节点引用（强引用）
pub type NodeRef = Arc<RwLock<Node>>;

/// 节点弱引用（用于避免循环引用）
pub type WeakNodeRef = Weak<RwLock<Node>>;

/// 键类型（字节序列）
pub type Key = Vec<u8>;

/// 值类型（与 DynamicAccumulator 的 API 保持一致）
pub type Value = i64;

// ================================================================================================
// Proof Structures
// ================================================================================================

/// 插入操作的证明数据
///
/// 包含插入前后累加器状态的完整证明，用于审计验证
#[derive(Debug, Clone)]
pub struct InsertionProof {
    /// 插入的键
    pub key: Key,

    /// 插入的值
    pub value: Value,

    /// 前序叶子的键（如果存在）
    pub key_prev: Option<Key>,

    /// 后序叶子的键（如果存在）
    pub key_next: Option<Key>,

    /// 当前叶子节点的旧累加器值
    pub ln_acc_old: G1Affine,

    /// 当前叶子节点的新累加器值
    pub ln_acc_new: G1Affine,

    /// 前序叶子节点的累加器值（如果存在）
    pub ln_prev_acc: Option<G1Affine>,

    /// 后序叶子节点的旧累加器值（如果存在）
    pub ln_next_acc_old: Option<G1Affine>,

    /// 后序叶子节点的新累加器值（如果存在）
    pub ln_next_acc_new: Option<G1Affine>,

    /// keyp在旧LNn.Acc中的成员证明（如果存在后序节点）
    pub keyp_in_ln_next_old_proof:
        Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,

    /// keyp在LN.Acc中的成员证明（如果存在前序节点）
    pub keyp_in_ln_proof: Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,

    /// NO_PREV在LN.Acc中的成员证明（如果不存在前序节点）
    pub no_prev_in_ln_proof: Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,

    /// key在新LNn.Acc中的成员证明（如果存在后序节点）
    pub key_in_ln_next_new_proof: Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,

    /// keyp在新LNn.Acc中的成员证明（如果存在前序和后序节点）
    pub keyp_in_ln_next_new_proof:
        Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,

    /// value在新LN.Acc中的成员证明
    pub value_in_ln_proof: Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,
}

/// 删除操作的证明数据
///
/// 包含删除前后累加器状态的完整证明，用于审计验证
#[derive(Debug, Clone)]
pub struct DeletionProof {
    /// 删除的键
    pub key: Key,

    /// 删除的值（如果只删除部分值）
    pub value: Option<Value>,

    /// 是否删除整个叶子节点
    pub delete_entire_leaf: bool,

    /// 前序叶子的键（如果存在）
    pub key_prev: Option<Key>,

    /// 后序叶子的键（如果存在）
    pub key_next: Option<Key>,

    /// 当前叶子节点的旧累加器值
    pub ln_acc_old: G1Affine,

    /// 当前叶子节点的新累加器值（如果只删除部分值）
    pub ln_acc_new: Option<G1Affine>,

    /// 后序叶子节点的旧累加器值（如果删除整个节点）
    pub ln_next_acc_old: Option<G1Affine>,

    /// 后序叶子节点的新累加器值（如果删除整个节点）
    pub ln_next_acc_new: Option<G1Affine>,

    /// value在旧LN.Acc中的成员证明（如果只删除部分值）
    pub value_in_ln_old_proof: Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,

    /// keyp在LN.Acc中的成员证明（如果删除整个节点且存在前序节点）
    pub keyp_in_ln_proof: Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,

    /// key在旧LNn.Acc中的成员证明（如果删除整个节点且存在后序节点）
    pub key_in_ln_next_old_proof: Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,

    /// keyp在新LNn.Acc中的成员证明（如果删除整个节点且存在前序和后序节点）
    pub keyp_in_ln_next_new_proof:
        Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,
}

/// 修改操作的证明数据
///
/// 包含修改前后累加器状态的完整证明，用于审计验证
/// 修改操作实际上是删除旧值和添加新值的组合
#[derive(Debug, Clone)]
pub struct UpdateProof {
    /// 修改的键
    pub key: Key,

    /// 旧值
    pub old_value: Value,

    /// 新值
    pub new_value: Value,

    /// 叶子节点修改前的累加器值
    pub ln_acc_old: G1Affine,

    /// 叶子节点修改后的累加器值
    pub ln_acc_new: G1Affine,

    /// 旧值的删除证明（从累加器中删除）
    pub delete_value_proof: Option<crate::acctrie::acc::dynamic_accumulator::DeleteProof>,

    /// 新值的添加证明（添加到累加器中）
    pub add_value_proof: Option<crate::acctrie::acc::dynamic_accumulator::AddProof>,
}

/// 查询结果 - 存在证明
///
/// 当查询的值存在时返回此证明
#[derive(Debug, Clone)]
pub struct ExistenceProof {
    /// 查询的键
    pub key: Key,

    /// 查询的值
    pub value: Value,

    /// 叶子节点的累加器值
    pub ln_acc: G1Affine,

    /// 值在累加器中的成员证明
    pub membership_proof: Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,
}

/// 查询结果 - 不存在证明
///
/// 当查询的值不存在时返回此证明,包含前序和后序叶子节点信息用于验证
#[derive(Debug, Clone)]
pub struct NonExistenceProof {
    /// 查询的键
    pub key: Key,

    /// 前序叶子节点的键(如果存在)
    pub key_prev: Option<Key>,

    /// 后序叶子节点的键(如果存在)
    pub key_next: Option<Key>,

    /// 后序叶子节点的累加器值(如果存在)
    pub ln_next_acc: Option<G1Affine>,

    /// 前序键在后序累加器中的成员证明(如果存在)
    pub prev_in_next_proof: Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,
}

/// 查询结果的枚举类型
#[derive(Debug, Clone)]
pub enum QueryResult {
    /// 值存在的证明
    Exists(ExistenceProof),

    /// 值不存在的证明
    NotExists(NonExistenceProof),
}

/// 审计验证结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditResult {
    /// 验证是否通过
    pub valid: bool,

    /// 验证失败的原因（如果有）
    pub error: Option<String>,
}

impl AuditResult {
    /// 创建成功的审计结果
    pub fn success() -> Self {
        Self {
            valid: true,
            error: None,
        }
    }

    /// 创建失败的审计结果
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            valid: false,
            error: Some(error.into()),
        }
    }
}

// ================================================================================================
// Node Types
// ================================================================================================

/// AccTrie 节点类型
///
/// 节点可以是内部节点（包含指针数组）或叶子节点（包含值集合和累加器）
#[derive(Debug, Clone)]
pub enum Node {
    /// 内部节点，包含256个子节点指针
    Internal(InternalNode),
    /// 叶子节点，包含后缀、值集合、累加器和链表指针
    Leaf(LeafNode),
}

/// 内部节点结构
///
/// 包含一个固定大小的指针数组，支持基于字节的快速索引
#[derive(Debug, Clone)]
pub struct InternalNode {
    /// 子节点指针数组，索引对应字节值 (0-255)
    pub children: [Option<NodeRef>; TRIE_FANOUT],
}

impl InternalNode {
    /// 创建新的内部节点，所有子节点初始化为 None
    #[inline]
    pub fn new() -> Self {
        const INIT: Option<NodeRef> = None;
        Self {
            children: [INIT; TRIE_FANOUT],
        }
    }

    /// 获取指定字节索引的子节点
    #[inline]
    pub fn get_child(&self, byte: u8) -> Option<&NodeRef> {
        self.children[byte as usize].as_ref()
    }

    /// 设置指定字节索引的子节点
    #[inline]
    pub fn set_child(&mut self, byte: u8, node: NodeRef) {
        self.children[byte as usize] = Some(node);
    }
}

impl Default for InternalNode {
    fn default() -> Self {
        Self::new()
    }
}

/// 叶子节点结构
///
/// 包含键后缀、值集合、密码学累加器以及双向链表指针
#[derive(Debug, Clone)]
pub struct LeafNode {
    /// 完整的键（用于获取准确的键值）
    pub full_key: Key,

    /// 键的后缀部分
    pub suffix: Key,

    /// 值的集合
    pub values: HashSet<Value>,

    /// 覆盖当前值集合的密码学累加器
    pub acc: DynamicAccumulator,

    /// 指向前一个叶子节点的弱引用（双向链表）
    pub prev: Option<WeakNodeRef>,

    /// 指向下一个叶子节点的强引用（双向链表）
    pub next: Option<NodeRef>,
}

impl LeafNode {
    /// 创建新的叶子节点
    ///
    /// # Arguments
    ///
    /// * `full_key` - 完整的键
    /// * `suffix` - 键的后缀
    /// * `values` - 初始值集合
    ///
    /// # Returns
    ///
    /// 返回初始化完成的叶子节点，累加器已包含所有初始值
    pub fn new(full_key: Key, suffix: Key, values: HashSet<Value>) -> Self {
        let mut acc = DynamicAccumulator::new();

        // 将所有初始值添加到累加器
        for value in &values {
            // 注意：初始化时忽略重复值错误
            let _ = acc.add(value);
        }

        Self {
            full_key,
            suffix,
            values,
            acc,
            prev: None,
            next: None,
        }
    }

    /// 创建空的叶子节点
    ///
    /// # Arguments
    ///
    /// * `full_key` - 完整的键
    /// * `suffix` - 键的后缀
    pub fn new_empty(full_key: Key, suffix: Key) -> Self {
        Self::new(full_key, suffix, HashSet::new())
    }

    /// 向叶子节点添加值
    ///
    /// 如果值已存在，则不执行任何操作
    ///
    /// # Arguments
    ///
    /// * `value` - 要添加的值
    ///
    /// # Errors
    ///
    /// 当累加器操作失败时返回错误
    pub fn add_value(&mut self, value: Value) -> Result<()> {
        if self.values.contains(&value) {
            return Ok(()); // 值已存在，直接返回
        }

        // 先添加到累加器（可能失败）
        self.acc.add(&value)?;

        // 累加器成功后再添加到值集合
        self.values.insert(value);

        Ok(())
    }

    /// 从叶子节点移除值
    ///
    /// 如果值不存在，返回错误
    ///
    /// # Arguments
    ///
    /// * `value` - 要移除的值
    ///
    /// # Errors
    ///
    /// 当值不存在或累加器操作失败时返回错误
    pub fn remove_value(&mut self, value: &Value) -> Result<()> {
        if !self.values.contains(value) {
            return Err(anyhow!("Value {} not found in leaf node", value));
        }

        // 先从累加器删除（可能失败）
        self.acc.delete(value)?;

        // 累加器成功后再从值集合删除
        self.values.remove(value);

        Ok(())
    }

    /// 检查值是否存在
    #[inline]
    pub fn contains_value(&self, value: &Value) -> bool {
        self.values.contains(value)
    }

    /// 获取值集合的大小
    #[inline]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// 检查是否为空
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 获取当前累加器的值
    #[inline]
    pub fn accumulator_value(&self) -> G1Affine {
        self.acc.acc_value
    }

    /// 获取累加器的引用
    #[inline]
    pub fn accumulator(&self) -> &DynamicAccumulator {
        &self.acc
    }

    /// 将键的哈希值添加到累加器
    ///
    /// # Arguments
    ///
    /// * `key` - 要添加的键
    pub fn add_key_to_acc(&mut self, key: &[u8]) -> Result<()> {
        let key_hash = Self::key_to_value(key);
        self.acc.add(&key_hash)?;
        Ok(())
    }

    /// 从累加器中删除键的哈希值
    ///
    /// # Arguments
    ///
    /// * `key` - 要删除的键
    pub fn remove_key_from_acc(&mut self, key: &[u8]) -> Result<()> {
        let key_hash = Self::key_to_value(key);
        self.acc.delete(&key_hash)?;
        Ok(())
    }

    /// 将键转换为累加器可以处理的值
    fn key_to_value(key: &[u8]) -> Value {
        // 使用键的哈希值作为累加器的元素
        let digest = key.to_digest();
        // 取哈希的前8字节作为i64
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest.0[..8]);
        i64::from_le_bytes(bytes)
    }

    /// 获取完整的键
    pub fn get_full_key(&self) -> &Key {
        &self.full_key
    }

    /// 获取键的后缀部分
    pub fn get_suffix(&self) -> &Key {
        &self.suffix
    }

    /// 获取只包含后缀的键（用于简化测试）
    pub fn suffix_key(&self) -> Key {
        self.suffix.clone()
    }

    /// 获取完整的键（路径 + 后缀）- 已弃用，使用 get_full_key() 代替
    #[deprecated(note = "Use get_full_key() instead")]
    pub fn full_key(&self, _path: &[u8]) -> Key {
        self.full_key.clone()
    }
}

/// AccTrie 主结构
///
/// 维护一个前缀树和叶子节点的双向链表
#[derive(Debug, Clone)]
pub struct AccTrie {
    /// 根节点（始终是内部节点）
    pub root: NodeRef,

    /// 叶子链表的头节点
    pub head_leaf: Option<NodeRef>,

    /// 叶子链表的尾节点（弱引用）
    pub tail_leaf: Option<WeakNodeRef>,
}

impl AccTrie {
    /// 将 G1Affine 累加器值转换为可存储的 i64 值
    ///
    /// 使用标准的非压缩点序列化格式（符合密码学最佳实践）
    fn acc_to_value(acc: &G1Affine) -> Value {
        use crate::digest::Digestible;
        use ark_serialize::CanonicalSerialize;

        // 使用标准序列化方法（arkworks 提供的规范序列化）
        let mut bytes = Vec::new();
        acc.serialize_uncompressed(&mut bytes)
            .expect("G1Affine 序列化不应失败");

        // 哈希序列化字节以获得确定性的 i64 值
        let digest = bytes.as_slice().to_digest();

        // 取哈希的前8字节作为i64
        let mut value_bytes = [0u8; 8];
        value_bytes.copy_from_slice(&digest.0[..8]);
        i64::from_le_bytes(value_bytes)
    }

    /// 创建新的 AccTrie
    ///
    /// 初始化一个空的前缀树，根节点为空的内部节点
    pub fn new() -> Self {
        let root = Arc::new(RwLock::new(Node::Internal(InternalNode::new())));
        Self {
            root,
            head_leaf: None,
            tail_leaf: None,
        }
    }

    /// 检查树是否为空（没有叶子节点）
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head_leaf.is_none()
    }

    /// 插入键值对到 AccTrie
    ///
    /// 实现完整的插入逻辑，包括：
    /// 1. 找到或创建叶子节点
    /// 2. 更新叶子节点的值和累加器
    /// 3. 更新前序和后序叶子节点的累加器
    /// 4. 生成插入证明
    ///
    /// # Arguments
    ///
    /// * `key` - 要插入的键
    /// * `value` - 要插入的值
    ///
    /// # Returns
    ///
    /// 返回插入证明，用于审计验证
    pub fn insert(&mut self, key: Key, value: Value) -> Result<InsertionProof> {
        // 步骤2: 在存储节点的AccTrie树中合适位置找到或创建一个叶子节点LN
        let (leaf_ref, _path) = self.find_or_create_leaf(&key)?;

        // 记录旧的累加器值
        let ln_acc_old = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                ln.accumulator_value()
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 检查叶子是否为新创建（首次插入值）
        let is_new_leaf = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                ln.is_empty()
            } else {
                false
            }
        };

        // 步骤3: 更新LN的Values值和Acc值
        {
            let mut leaf = leaf_ref.write().unwrap();
            if let Node::Leaf(ln) = &mut *leaf {
                ln.add_value(value)?;
            }
        }

        // 3. 只有在首次插入时才更新前序/后序累加器
        if !is_new_leaf {
            // 叶子已存在，只更新值，不更新前序/后序累加器
            let (ln_acc_new, value_in_ln_proof) = {
                let leaf = leaf_ref.read().unwrap();
                if let Node::Leaf(ln) = &*leaf {
                    let acc_new = ln.accumulator_value();
                    let proof = ln.acc.prove_membership(&value).ok();
                    (acc_new, proof)
                } else {
                    return Err(anyhow!("Expected leaf node"));
                }
            };

            return Ok(InsertionProof {
                key,
                value,
                key_prev: None,
                key_next: None,
                ln_acc_old,
                ln_acc_new,
                ln_prev_acc: None,
                ln_next_acc_old: None,
                ln_next_acc_new: None,
                keyp_in_ln_next_old_proof: None,
                keyp_in_ln_proof: None,
                no_prev_in_ln_proof: None,
                key_in_ln_next_new_proof: None,
                keyp_in_ln_next_new_proof: None,
                value_in_ln_proof,
            });
        }

        // 步骤4: 获取前序叶子LNp和keyp，将keyp加入到LN.Acc中
        // 若前序叶子不存在，则将"NoPrev"添加到LN.Acc中（仅首次插入）
        let (key_prev, ln_prev_acc, _prev_ref) = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                if let Some(prev_weak) = &ln.prev {
                    if let Some(prev_ref) = prev_weak.upgrade() {
                        let prev = prev_ref.read().unwrap();
                        if let Node::Leaf(ln_prev) = &*prev {
                            let key_p = ln_prev.get_full_key().clone();
                            (
                                Some(key_p),
                                Some(ln_prev.accumulator_value()),
                                Some(prev_ref.clone()),
                            )
                        } else {
                            (None, None, None)
                        }
                    } else {
                        (None, None, None)
                    }
                } else {
                    (None, None, None)
                }
            } else {
                (None, None, None)
            }
        };

        // 将keyp或NO_PREV添加到LN.Acc中（仅首次插入）
        let (keyp_in_ln_proof, no_prev_in_ln_proof) = {
            let mut leaf = leaf_ref.write().unwrap();
            if let Node::Leaf(ln) = &mut *leaf {
                if let Some(ref k_prev) = key_prev {
                    ln.add_key_to_acc(k_prev)?;
                    let proof = ln
                        .acc
                        .prove_membership(&LeafNode::key_to_value(k_prev))
                        .ok();
                    (proof, None)
                } else {
                    ln.acc.add(&NO_PREV)?;
                    let proof = ln.acc.prove_membership(&NO_PREV).ok();
                    (None, proof)
                }
            } else {
                (None, None)
            }
        };

        // 步骤5: 若LNp的后序不为空，则返回LNn和keyn，
        // 将key加入到LNn.Acc中，同时在LNn.Acc中删除keyp（仅首次插入）
        let (
            key_next,
            ln_next_acc_old,
            ln_next_acc_new,
            keyp_in_ln_next_old_proof,
            key_in_ln_next_new_proof,
            keyp_in_ln_next_new_proof,
        ) = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                if let Some(next_ref) = &ln.next {
                    // 记录LNn的旧累加器值并生成keyp在旧LNn.Acc中的证明
                    let (old_acc, keyp_proof_old) = {
                        let next = next_ref.read().unwrap();
                        if let Node::Leaf(ln_next) = &*next {
                            let old_acc_val = ln_next.accumulator_value();
                            let proof = if let Some(ref k_prev) = key_prev {
                                ln_next
                                    .acc
                                    .prove_membership(&LeafNode::key_to_value(k_prev))
                                    .ok()
                            } else {
                                ln_next.acc.prove_membership(&NO_PREV).ok()
                            };
                            (old_acc_val, proof)
                        } else {
                            return Err(anyhow!("Expected leaf node for next"));
                        }
                    };

                    // 在LNn.Acc中删除keyp，添加key
                    {
                        let mut next = next_ref.write().unwrap();
                        if let Node::Leaf(ln_next) = &mut *next {
                            if let Some(ref k_prev) = key_prev {
                                ln_next.remove_key_from_acc(k_prev)?;
                            } else {
                                ln_next.acc.delete(&NO_PREV)?;
                            }
                            ln_next.add_key_to_acc(&key)?;
                        }
                    }

                    // 获取keyn和LNn的新累加器值，以及生成证明
                    let (key_n, new_acc, key_proof_new, keyp_proof_new) = {
                        let next = next_ref.read().unwrap();
                        if let Node::Leaf(ln_next) = &*next {
                            let key_n = ln_next.get_full_key().clone();
                            let new_acc_val = ln_next.accumulator_value();
                            let key_proof = ln_next
                                .acc
                                .prove_membership(&LeafNode::key_to_value(&key))
                                .ok();
                            // 如果有前序节点，证明keyp在新LNn.Acc中；否则证明NO_PREV在新LNn.Acc中（但存储在keyp_proof字段）
                            let keyp_proof = if let Some(ref k_prev) = key_prev {
                                ln_next
                                    .acc
                                    .prove_membership(&LeafNode::key_to_value(k_prev))
                                    .ok()
                            } else {
                                // 当没有前序节点时，新LNn.Acc中应该包含NO_PREV而不是keyp
                                // 但由于我们删除了NO_PREV并添加了key，这里不需要证明
                                None
                            };
                            (Some(key_n), Some(new_acc_val), key_proof, keyp_proof)
                        } else {
                            (None, None, None, None)
                        }
                    };
                    (
                        key_n,
                        Some(old_acc),
                        new_acc,
                        keyp_proof_old,
                        key_proof_new,
                        keyp_proof_new,
                    )
                } else {
                    // 后序叶子不存在
                    (None, None, None, None, None, None)
                }
            } else {
                (None, None, None, None, None, None)
            }
        };

        // 若后序不存在，添加NO_NEXT到LN.Acc（仅首次插入）
        if key_next.is_none() {
            let mut leaf = leaf_ref.write().unwrap();
            if let Node::Leaf(ln) = &mut *leaf {
                ln.acc.add(&NO_NEXT)?;
            }
        }

        // 记录新的累加器值并生成value的成员证明
        let (ln_acc_new, value_in_ln_proof) = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                let acc_new = ln.accumulator_value();
                let proof = ln.acc.prove_membership(&value).ok();
                (acc_new, proof)
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 步骤6: 返回LN.Acc，keyp，LNp.Acc，keyn，LNn.Acc的新旧值给Auditor
        Ok(InsertionProof {
            key,
            value,
            key_prev,
            key_next,
            ln_acc_old,
            ln_acc_new,
            ln_prev_acc,
            ln_next_acc_old,
            ln_next_acc_new,
            keyp_in_ln_next_old_proof,
            keyp_in_ln_proof,
            no_prev_in_ln_proof,
            key_in_ln_next_new_proof,
            keyp_in_ln_next_new_proof,
            value_in_ln_proof,
        })
    }

    /// 查找或创建叶子节点
    ///
    /// 沿着前缀树路径查找，如果不存在则创建新的叶子节点
    fn find_or_create_leaf(&mut self, key: &[u8]) -> Result<(NodeRef, Vec<u8>)> {
        let mut current = self.root.clone();
        let mut path = Vec::new();
        let mut key_pos = 0;

        loop {
            let node = current.read().unwrap();
            match &*node {
                Node::Internal(internal) => {
                    if key_pos >= key.len() {
                        // 到达键的末尾，需要创建叶子节点
                        drop(node);
                        return self.create_leaf_at(current.clone(), &key[key_pos..], path);
                    }

                    let byte = key[key_pos];
                    if let Some(child) = internal.get_child(byte) {
                        let child_clone = child.clone();
                        drop(node);
                        current = child_clone;
                        path.push(byte);
                        key_pos += 1;
                    } else {
                        // 需要创建新的分支
                        drop(node);
                        return self.create_leaf_at(current.clone(), &key[key_pos..], path);
                    }
                }
                Node::Leaf(_) => {
                    // 找到叶子节点
                    drop(node);
                    return Ok((current.clone(), path));
                }
            }
        }
    }

    /// 在指定位置创建叶子节点
    fn create_leaf_at(
        &mut self,
        parent: NodeRef,
        suffix: &[u8],
        mut path: Vec<u8>,
    ) -> Result<(NodeRef, Vec<u8>)> {
        if suffix.is_empty() {
            return Err(anyhow!("Cannot create leaf with empty suffix"));
        }

        let first_byte = suffix[0];
        // 叶子节点的后缀不包含第一个字节（第一个字节用作索引）
        let leaf_suffix = if suffix.len() > 1 {
            suffix[1..].to_vec()
        } else {
            Vec::new()
        };

        // 完整的键 = path + suffix
        let mut full_key = path.clone();
        full_key.extend_from_slice(suffix);

        let leaf = Arc::new(RwLock::new(Node::Leaf(LeafNode::new_empty(
            full_key,
            leaf_suffix,
        ))));

        // 设置双向链表
        self.insert_into_leaf_list(leaf.clone());

        // 将叶子节点添加到父节点
        {
            let mut parent_node = parent.write().unwrap();
            if let Node::Internal(internal) = &mut *parent_node {
                internal.set_child(first_byte, leaf.clone());
            }
        }

        path.push(first_byte);
        Ok((leaf, path))
    }

    /// 将新叶子节点插入到叶子链表中（按字典序）
    fn insert_into_leaf_list(&mut self, new_leaf: NodeRef) {
        if self.head_leaf.is_none() {
            // 第一个叶子节点
            self.head_leaf = Some(new_leaf.clone());
            self.tail_leaf = Some(Arc::downgrade(&new_leaf));
            return;
        }

        // 获取新叶子的键
        let new_key = {
            let new = new_leaf.read().unwrap();
            if let Node::Leaf(new_node) = &*new {
                new_node.get_full_key().clone()
            } else {
                return; // 不应该发生
            }
        };

        // 遍历链表找到正确的插入位置
        let mut current = self.head_leaf.clone();

        while let Some(current_ref) = current {
            let current_key = {
                let node = current_ref.read().unwrap();
                if let Node::Leaf(leaf_node) = &*node {
                    leaf_node.get_full_key().clone()
                } else {
                    return; // 不应该发生
                }
            };

            if new_key < current_key {
                // 在当前节点之前插入
                let prev_weak = {
                    let node = current_ref.read().unwrap();
                    if let Node::Leaf(leaf_node) = &*node {
                        leaf_node.prev.clone()
                    } else {
                        None
                    }
                };

                // 设置新节点的指针
                {
                    let mut new = new_leaf.write().unwrap();
                    if let Node::Leaf(new_node) = &mut *new {
                        new_node.prev = prev_weak.clone();
                        new_node.next = Some(current_ref.clone());
                    }
                }

                // 更新前序节点的next指针
                if let Some(prev_weak_ref) = prev_weak {
                    if let Some(prev_ref) = prev_weak_ref.upgrade() {
                        let mut prev = prev_ref.write().unwrap();
                        if let Node::Leaf(prev_node) = &mut *prev {
                            prev_node.next = Some(new_leaf.clone());
                        }
                    }
                } else {
                    // 插入到头部
                    self.head_leaf = Some(new_leaf.clone());
                }

                // 更新当前节点的prev指针
                {
                    let mut curr = current_ref.write().unwrap();
                    if let Node::Leaf(curr_node) = &mut *curr {
                        curr_node.prev = Some(Arc::downgrade(&new_leaf));
                    }
                }

                return;
            }

            // 检查是否到达链表末尾
            let next = {
                let node = current_ref.read().unwrap();
                if let Node::Leaf(leaf_node) = &*node {
                    leaf_node.next.clone()
                } else {
                    None
                }
            };

            if next.is_none() {
                // 插入到末尾
                {
                    let mut curr = current_ref.write().unwrap();
                    if let Node::Leaf(curr_node) = &mut *curr {
                        curr_node.next = Some(new_leaf.clone());
                    }
                }

                {
                    let mut new = new_leaf.write().unwrap();
                    if let Node::Leaf(new_node) = &mut *new {
                        new_node.prev = Some(Arc::downgrade(&current_ref));
                    }
                }

                self.tail_leaf = Some(Arc::downgrade(&new_leaf));
                return;
            }

            current = next;
        }
    }

    /// 步骤7: Auditor验证插入的有效性并更新根Acc
    ///
    /// 验证步骤：
    /// ① 比较keyp<key<keyn三者存在字典序
    /// ② 验证旧LNn.Acc在根Acc中是否存在，存在则删除
    /// ③ 验证keyp在旧LNn.Acc中是否存在（使用成员证明）
    /// ④ 验证keyp在LN.Acc中是否存在（使用成员证明）
    /// ⑤ 验证key和keyp在新LNn.Acc中是否存在（使用成员证明）
    /// ⑥ 在根Acc中添加LN.Acc和新LNn.Acc
    /// ⑦ 验证value在新LN.Acc中是否存在（使用成员证明）
    pub fn audit_insertion(
        &self,
        proof: &InsertionProof,
        root_acc: &mut DynamicAccumulator,
    ) -> Result<AuditResult> {
        // ① 比较keyp<key<keyn三者存在字典序
        if let Some(ref key_prev) = proof.key_prev {
            if key_prev >= &proof.key {
                return Ok(AuditResult::failure("key_prev must be less than key"));
            }
        }

        if let Some(ref key_next) = proof.key_next {
            if &proof.key >= key_next {
                return Ok(AuditResult::failure("key must be less than key_next"));
            }
        }

        // ⑦ 验证value在新LN.Acc中是否存在
        if let Some(ref value_proof) = proof.value_in_ln_proof {
            if !value_proof.verify(proof.ln_acc_new) {
                return Ok(AuditResult::failure(
                    "Value membership proof verification failed in LN.Acc",
                ));
            }
        }

        // ② 验证旧LNn.Acc在根Acc中是否存在，存在则删除
        // ⑥ 在根Acc中添加LN.Acc和新LNn.Acc
        if let (Some(ln_next_acc_old), Some(ln_next_acc_new)) =
            (proof.ln_next_acc_old, proof.ln_next_acc_new)
        {
            // ② 从根累加器删除旧的LNn.Acc值
            let old_acc_value = Self::acc_to_value(&ln_next_acc_old);
            if root_acc.len() > 0 {
                let _ = root_acc.delete(&old_acc_value);
            }

            // ③ 验证keyp在旧LNn.Acc中是否存在
            if let Some(ref keyp_proof) = proof.keyp_in_ln_next_old_proof {
                if !keyp_proof.verify(ln_next_acc_old) {
                    return Ok(AuditResult::failure(
                        "keyp membership proof verification failed in old LNn.Acc",
                    ));
                }
            } else {
                return Ok(AuditResult::failure(
                    "Missing keyp membership proof for old LNn.Acc",
                ));
            }

            // ⑤ 验证key在新LNn.Acc中是否存在
            if let Some(ref key_proof) = proof.key_in_ln_next_new_proof {
                if !key_proof.verify(ln_next_acc_new) {
                    return Ok(AuditResult::failure(
                        "key membership proof verification failed in new LNn.Acc",
                    ));
                }
            } else {
                return Ok(AuditResult::failure(
                    "Missing key membership proof for new LNn.Acc",
                ));
            }

            // ⑤ 验证keyp在新LNn.Acc中是否存在（仅当存在前序节点时）
            // 当第一个叶子节点插入时（没有前序节点），新的后序节点应该包含当前key而不是keyp
            // 所以只有在有前序节点时才验证keyp在新LNn.Acc中
            if proof.key_prev.is_some() {
                if let Some(ref keyp_proof) = proof.keyp_in_ln_next_new_proof {
                    if !keyp_proof.verify(ln_next_acc_new) {
                        return Ok(AuditResult::failure(
                            "keyp membership proof verification failed in new LNn.Acc",
                        ));
                    }
                }
                // 注意：当没有前序节点时，不要求此证明，因为LNn.Acc中删除的是NO_PREV并添加了key
            }

            // ⑥ 添加新的LNn.Acc值到根
            let new_acc_value = Self::acc_to_value(&ln_next_acc_new);
            root_acc.add(&new_acc_value)?;
        } else {
            // ⑥ 新插入的叶子节点是最后一个或叶子已存在但只是添加值，添加LN.Acc到根
            let new_acc_value = Self::acc_to_value(&proof.ln_acc_new);
            root_acc.add(&new_acc_value)?;
        }

        // ④ 验证keyp或NO_PREV在LN.Acc中是否存在（仅在首次创建叶子时需要）
        // 如果没有前序/后序信息，说明只是向已存在的叶子添加值，不需要验证
        if proof.ln_next_acc_old.is_some() || proof.ln_next_acc_new.is_some() {
            if proof.key_prev.is_some() {
                if let Some(ref keyp_proof) = proof.keyp_in_ln_proof {
                    if !keyp_proof.verify(proof.ln_acc_new) {
                        return Ok(AuditResult::failure(
                            "keyp membership proof verification failed in LN.Acc",
                        ));
                    }
                } else {
                    return Ok(AuditResult::failure(
                        "Missing keyp membership proof for LN.Acc",
                    ));
                }
            } else {
                // 验证NO_PREV在LN.Acc中
                if let Some(ref no_prev_proof) = proof.no_prev_in_ln_proof {
                    if !no_prev_proof.verify(proof.ln_acc_new) {
                        return Ok(AuditResult::failure(
                            "NO_PREV membership proof verification failed in LN.Acc",
                        ));
                    }
                } else {
                    return Ok(AuditResult::failure(
                        "Missing NO_PREV membership proof for LN.Acc",
                    ));
                }
            }
        }

        Ok(AuditResult::success())
    }

    /// 删除键值对或整个叶子节点
    ///
    /// 实现完整的删除逻辑，包括：
    /// 步骤2: 在存储节点的AccTrie树中找到key对应的叶子节点LN
    /// 步骤3: 若只删除部分Values，则更新LN.Acc并将新旧LN.Acc返给Auditor更新根Acc
    /// 步骤4: 若删除整个叶子节点LN，则获取前后序叶子，更新LNn.Acc和指针
    /// 步骤5: 返回LN.Acc、keyp、keyn、LNn.Acc的新旧值给Auditor
    ///
    /// # Arguments
    ///
    /// * `key` - 要删除的键
    /// * `value` - 要删除的值（如果为 None 则删除整个叶子节点）
    ///
    /// # Returns
    ///
    /// 返回删除证明，用于审计验证
    pub fn delete(&mut self, key: &Key, value: Option<Value>) -> Result<DeletionProof> {
        // 步骤2: 在存储节点的AccTrie树中找到key对应的叶子节点LN
        let leaf_ref = self
            .find_leaf(key)
            .ok_or_else(|| anyhow!("Key not found"))?;

        // 检查是否删除整个叶子节点
        let delete_entire = value.is_none() || {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                value.is_some() && ln.values.len() == 1 && ln.values.contains(&value.unwrap())
            } else {
                false
            }
        };

        if delete_entire {
            self.delete_entire_leaf(leaf_ref, key)
        } else {
            self.delete_partial_value(leaf_ref, key, value.unwrap())
        }
    }

    /// 步骤3: 删除部分值（不删除整个叶子节点）
    /// 更新LN.Acc并将新旧LN.Acc返给Auditor更新根Acc
    fn delete_partial_value(
        &mut self,
        leaf_ref: NodeRef,
        key: &Key,
        value: Value,
    ) -> Result<DeletionProof> {
        // 记录旧的LN.Acc值并生成value的成员证明
        let (ln_acc_old, value_in_ln_old_proof) = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                let acc_old = ln.accumulator_value();
                let proof = ln.acc.prove_membership(&value).ok();
                (acc_old, proof)
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 更新LN: 删除值
        {
            let mut leaf = leaf_ref.write().unwrap();
            if let Node::Leaf(ln) = &mut *leaf {
                ln.remove_value(&value)?;
            }
        }

        // 记录新的LN.Acc值
        let ln_acc_new = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                ln.accumulator_value()
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 返回新旧LN.Acc给Auditor
        Ok(DeletionProof {
            key: key.clone(),
            value: Some(value),
            delete_entire_leaf: false,
            key_prev: None,
            key_next: None,
            ln_acc_old,
            ln_acc_new: Some(ln_acc_new),
            ln_next_acc_old: None,
            ln_next_acc_new: None,
            value_in_ln_old_proof,
            keyp_in_ln_proof: None,
            key_in_ln_next_old_proof: None,
            keyp_in_ln_next_new_proof: None,
        })
    }

    /// 步骤4: 删除整个叶子节点
    /// 获取前序叶子LNp和keyp，获取后序叶子LNn和keyn，
    /// 在LNn.Acc中删除key并加入keyp，
    /// 将前序指针指向LNp，将LNp的后序指针指向LNn
    fn delete_entire_leaf(&mut self, leaf_ref: NodeRef, key: &Key) -> Result<DeletionProof> {
        // 记录旧的LN.Acc值并生成keyp的成员证明
        let (ln_acc_old, keyp_in_ln_proof) = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                let acc_old = ln.accumulator_value();
                // 获取前序键
                let key_prev_opt = if let Some(prev_weak) = &ln.prev {
                    prev_weak.upgrade().and_then(|p| {
                        let prev = p.read().unwrap();
                        if let Node::Leaf(ln_prev) = &*prev {
                            Some(ln_prev.get_full_key().clone())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };
                // 生成keyp在LN.Acc中的证明
                let proof = if let Some(ref k_prev) = key_prev_opt {
                    ln.acc
                        .prove_membership(&LeafNode::key_to_value(k_prev))
                        .ok()
                } else {
                    ln.acc.prove_membership(&NO_PREV).ok()
                };
                (acc_old, proof)
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 获取前序叶子LNp和keyp，后序叶子LNn和keyn
        let (key_prev, prev_ref, key_next, next_ref) = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                let prev = if let Some(prev_weak) = &ln.prev {
                    prev_weak.upgrade().and_then(|p| {
                        let prev = p.read().unwrap();
                        if let Node::Leaf(ln_prev) = &*prev {
                            let key_p = ln_prev.get_full_key().clone();
                            Some((key_p, p.clone()))
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };

                let next = ln.next.as_ref().and_then(|n| {
                    let next = n.read().unwrap();
                    if let Node::Leaf(ln_next) = &*next {
                        let key_n = ln_next.get_full_key().clone();
                        Some((key_n, n.clone()))
                    } else {
                        None
                    }
                });

                let (key_prev, prev_ref) = prev
                    .map(|(k, r)| (Some(k), Some(r)))
                    .unwrap_or((None, None));
                let (key_next, next_ref) = next
                    .map(|(k, r)| (Some(k), Some(r)))
                    .unwrap_or((None, None));

                (key_prev, prev_ref, key_next, next_ref)
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 处理后序节点LNn的累加器更新，并生成证明
        let (ln_next_acc_old, ln_next_acc_new, key_in_ln_next_old_proof, keyp_in_ln_next_new_proof) =
            if let Some(next_ref) = next_ref.clone() {
                // 记录旧的LNn.Acc值并生成key的成员证明
                let (old_acc, key_proof_old) = {
                    let next = next_ref.read().unwrap();
                    if let Node::Leaf(ln_next) = &*next {
                        let old_acc_val = ln_next.accumulator_value();
                        let proof = ln_next
                            .acc
                            .prove_membership(&LeafNode::key_to_value(key))
                            .ok();
                        (old_acc_val, proof)
                    } else {
                        return Err(anyhow!("Expected leaf node for next"));
                    }
                };

                // 在LNn.Acc中删除key并加入keyp
                {
                    let mut next = next_ref.write().unwrap();
                    if let Node::Leaf(ln_next) = &mut *next {
                        // 删除当前键key
                        ln_next.remove_key_from_acc(key)?;

                        // 添加前序键keyp或NO_PREV
                        if let Some(ref k_prev) = key_prev {
                            ln_next.add_key_to_acc(k_prev)?;
                        } else {
                            ln_next.acc.add(&NO_PREV)?;
                        }
                    }
                }

                // 记录新的LNn.Acc值并生成keyp的成员证明
                let (new_acc, keyp_proof_new) = {
                    let next = next_ref.read().unwrap();
                    if let Node::Leaf(ln_next) = &*next {
                        let new_acc_val = ln_next.accumulator_value();
                        let proof = if let Some(ref k_prev) = key_prev {
                            ln_next
                                .acc
                                .prove_membership(&LeafNode::key_to_value(k_prev))
                                .ok()
                        } else {
                            ln_next.acc.prove_membership(&NO_PREV).ok()
                        };
                        (new_acc_val, proof)
                    } else {
                        return Err(anyhow!("Expected leaf node for next"));
                    }
                };

                (Some(old_acc), Some(new_acc), key_proof_old, keyp_proof_new)
            } else {
                (None, None, None, None)
            };

        // 将前序指针指向LNp，将LNp的后序指针指向LNn
        // 更新双向链表指针
        if let Some(prev_ref) = prev_ref.clone() {
            let mut prev = prev_ref.write().unwrap();
            if let Node::Leaf(ln_prev) = &mut *prev {
                ln_prev.next = next_ref.clone();
            }
        } else {
            // 更新头节点
            self.head_leaf = next_ref.clone();
        }

        if let Some(next_ref) = next_ref.clone() {
            let mut next = next_ref.write().unwrap();
            if let Node::Leaf(ln_next) = &mut *next {
                ln_next.prev = prev_ref.as_ref().map(|p| Arc::downgrade(p));
            }
        } else {
            // 更新尾节点
            self.tail_leaf = prev_ref.as_ref().map(|p| Arc::downgrade(p));
        }

        // 从父节点中移除叶子节点引用
        self.remove_leaf_from_parent(&leaf_ref)?;

        // 步骤5: 返回LN.Acc、keyp、keyn、LNn.Acc的新旧值给Auditor
        Ok(DeletionProof {
            key: key.clone(),
            value: None,
            delete_entire_leaf: true,
            key_prev,
            key_next,
            ln_acc_old,
            ln_acc_new: None,
            ln_next_acc_old,
            ln_next_acc_new,
            value_in_ln_old_proof: None,
            keyp_in_ln_proof,
            key_in_ln_next_old_proof,
            keyp_in_ln_next_new_proof,
        })
    }

    /// 从trie树中移除叶子节点的引用
    fn remove_leaf_from_parent(&self, leaf_ref: &NodeRef) -> Result<()> {
        // 获取叶子的完整键
        let full_key = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                ln.get_full_key().clone()
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        if full_key.is_empty() {
            return Err(anyhow!("Empty key"));
        }

        // 遍历到父节点
        let mut current = self.root.clone();
        let mut key_pos = 0;

        while key_pos < full_key.len() {
            let byte = full_key[key_pos];
            let next = {
                let node = current.read().unwrap();
                if let Node::Internal(internal) = &*node {
                    internal.get_child(byte).cloned()
                } else {
                    return Err(anyhow!("Expected internal node"));
                }
            };

            if let Some(next_ref) = next {
                let is_leaf = {
                    let next_node = next_ref.read().unwrap();
                    matches!(&*next_node, Node::Leaf(_))
                };

                if is_leaf {
                    // 找到了叶子节点，current是其父节点
                    let mut parent = current.write().unwrap();
                    if let Node::Internal(internal) = &mut *parent {
                        internal.children[byte as usize] = None;
                    }
                    return Ok(());
                }

                current = next_ref;
                key_pos += 1;
            } else {
                return Err(anyhow!("Path not found"));
            }
        }

        Err(anyhow!("Leaf node not found in expected location"))
    }

    /// 查找键对应的叶子节点
    pub fn find_leaf(&self, key: &[u8]) -> Option<NodeRef> {
        let mut current = self.root.clone();
        let mut key_pos = 0;

        loop {
            let node = current.read().unwrap();
            match &*node {
                Node::Internal(internal) => {
                    if key_pos >= key.len() {
                        // 键已经用完了，但还在内部节点，说明没找到
                        return None;
                    }

                    let byte = key[key_pos];
                    if let Some(child) = internal.get_child(byte) {
                        let child_clone = child.clone();
                        drop(node);
                        current = child_clone;
                        key_pos += 1;
                    } else {
                        return None;
                    }
                }
                Node::Leaf(leaf) => {
                    // 比较剩余的键部分和叶子的后缀
                    // 注意：叶子的后缀不包含已经通过路径匹配的字节
                    let remaining_key = if key_pos < key.len() {
                        &key[key_pos..]
                    } else {
                        &[]
                    };

                    // 检查后缀是否完全匹配
                    if remaining_key == leaf.suffix.as_slice() {
                        drop(node);
                        return Some(current.clone());
                    }
                    return None;
                }
            }
        }
    }

    /// 步骤6: Auditor验证删除的有效性并更新Acc
    ///
    /// 验证步骤：
    /// ① 比较keyp<key<keyn三者存在字典序
    /// ② 验证keyp在LN.Acc中是否存在（使用成员证明）
    /// ③ 验证key在旧LNn.Acc中是否存在（使用成员证明）
    /// ④ 验证keyp在新LNn.Acc中是否存在（使用成员证明）
    /// ⑤ 在根Acc中删除LN.Acc和旧LNn.Acc
    /// ⑥ 在根Acc中添加新LNn.Acc
    /// ⑦ 验证value在旧LN.Acc中是否存在（部分删除时）
    pub fn audit_deletion(
        &self,
        proof: &DeletionProof,
        _root_acc: &mut DynamicAccumulator,
    ) -> Result<AuditResult> {
        // ① 比较keyp<key<keyn三者存在字典序
        if let Some(ref key_prev) = proof.key_prev {
            if key_prev >= &proof.key {
                return Ok(AuditResult::failure("key_prev must be less than key"));
            }
        }

        if let Some(ref key_next) = proof.key_next {
            if &proof.key >= key_next {
                return Ok(AuditResult::failure("key must be less than key_next"));
            }
        }

        // 如果删除整个叶子节点
        if proof.delete_entire_leaf {
            if let (Some(ln_next_acc_old), Some(ln_next_acc_new)) =
                (proof.ln_next_acc_old, proof.ln_next_acc_new)
            {
                // ② 验证keyp在LN.Acc中是否存在
                if let Some(ref keyp_proof) = proof.keyp_in_ln_proof {
                    if !keyp_proof.verify(proof.ln_acc_old) {
                        return Ok(AuditResult::failure(
                            "keyp membership proof verification failed in LN.Acc",
                        ));
                    }
                } else {
                    return Ok(AuditResult::failure(
                        "Missing keyp membership proof for LN.Acc",
                    ));
                }

                // ③ 验证key在旧LNn.Acc中是否存在
                if let Some(ref key_proof) = proof.key_in_ln_next_old_proof {
                    if !key_proof.verify(ln_next_acc_old) {
                        return Ok(AuditResult::failure(
                            "key membership proof verification failed in old LNn.Acc",
                        ));
                    }
                } else {
                    return Ok(AuditResult::failure(
                        "Missing key membership proof for old LNn.Acc",
                    ));
                }

                // ④ 验证keyp在新LNn.Acc中
                if let Some(ref keyp_proof) = proof.keyp_in_ln_next_new_proof {
                    if !keyp_proof.verify(ln_next_acc_new) {
                        return Ok(AuditResult::failure(
                            "keyp membership proof verification failed in new LNn.Acc",
                        ));
                    }
                } else {
                    return Ok(AuditResult::failure(
                        "Missing keyp membership proof for new LNn.Acc",
                    ));
                }

                // ⑤ 在根Acc中删除LN.Acc
                let ln_acc_value = Self::acc_to_value(&proof.ln_acc_old);
                if _root_acc.len() > 0 {
                    let _ = _root_acc.delete(&ln_acc_value);
                }

                // ⑤ 在根Acc中删除旧LNn.Acc
                let old_acc_value = Self::acc_to_value(&ln_next_acc_old);
                if _root_acc.len() > 0 {
                    let _ = _root_acc.delete(&old_acc_value);
                }

                // ⑥ 在根Acc中添加新LNn.Acc
                let new_acc_value = Self::acc_to_value(&ln_next_acc_new);
                _root_acc.add(&new_acc_value)?;
            } else {
                // 删除的是最后一个叶子节点，只需从根Acc中删除LN.Acc
                let ln_acc_value = Self::acc_to_value(&proof.ln_acc_old);
                if _root_acc.len() > 0 {
                    let _ = _root_acc.delete(&ln_acc_value);
                }
            }
        } else {
            // ⑦ 部分删除：验证value在旧LN.Acc中存在
            if let Some(ref value_proof) = proof.value_in_ln_old_proof {
                if !value_proof.verify(proof.ln_acc_old) {
                    return Ok(AuditResult::failure(
                        "value membership proof verification failed in old LN.Acc",
                    ));
                }
            } else {
                return Ok(AuditResult::failure(
                    "Missing value membership proof for old LN.Acc",
                ));
            }

            // 在根Acc中删除旧LN.Acc，添加新LN.Acc
            if let Some(ln_acc_new) = proof.ln_acc_new {
                let old_acc_value = Self::acc_to_value(&proof.ln_acc_old);
                if _root_acc.len() > 0 {
                    let _ = _root_acc.delete(&old_acc_value);
                }

                let new_acc_value = Self::acc_to_value(&ln_acc_new);
                _root_acc.add(&new_acc_value)?;
            }
        }

        Ok(AuditResult::success())
    }

    /// 修改键值对
    ///
    /// 实现完整的修改逻辑，包括：
    /// 步骤1: 在AccTrie树中找到key对应的叶子节点LN
    /// 步骤2: 验证旧值存在于LN中
    /// 步骤3: 从LN.Acc中删除旧值
    /// 步骤4: 向LN.Acc中添加新值
    /// 步骤5: 返回修改证明给Auditor验证
    ///
    /// # Arguments
    ///
    /// * `key` - 要修改的键
    /// * `old_value` - 旧值
    /// * `new_value` - 新值
    ///
    /// # Returns
    ///
    /// 返回修改证明，用于审计验证
    pub fn update(&mut self, key: &Key, old_value: Value, new_value: Value) -> Result<UpdateProof> {
        // 步骤1: 在AccTrie树中找到key对应的叶子节点LN
        let leaf_ref = self
            .find_leaf(key)
            .ok_or_else(|| anyhow!("Key not found"))?;

        // 步骤2: 验证旧值存在于LN中
        {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                if !ln.contains_value(&old_value) {
                    return Err(anyhow!("Old value {} not found in leaf node", old_value));
                }
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        }

        // 记录修改前的累加器值
        let ln_acc_old = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                ln.accumulator_value()
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 步骤3: 从LN.Acc中删除旧值，并获取删除证明
        let delete_proof = {
            let mut leaf = leaf_ref.write().unwrap();
            if let Node::Leaf(ln) = &mut *leaf {
                let proof = ln.acc.delete(&old_value)?;
                ln.values.remove(&old_value);
                Some(proof)
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 步骤4: 向LN.Acc中添加新值，并获取添加证明
        let add_proof = {
            let mut leaf = leaf_ref.write().unwrap();
            if let Node::Leaf(ln) = &mut *leaf {
                let proof = ln.acc.add(&new_value)?;
                ln.values.insert(new_value);
                Some(proof)
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 记录修改后的累加器值
        let ln_acc_new = {
            let leaf = leaf_ref.read().unwrap();
            if let Node::Leaf(ln) = &*leaf {
                ln.accumulator_value()
            } else {
                return Err(anyhow!("Expected leaf node"));
            }
        };

        // 步骤5: 返回修改证明给Auditor验证
        Ok(UpdateProof {
            key: key.clone(),
            old_value,
            new_value,
            ln_acc_old,
            ln_acc_new,
            delete_value_proof: delete_proof,
            add_value_proof: add_proof,
        })
    }

    /// Auditor验证修改操作的有效性
    ///
    /// 验证步骤：
    /// ① 验证删除证明的有效性（旧值从旧累加器中删除）
    /// ② 验证添加证明的有效性（新值添加到新累加器中）
    /// ③ 验证累加器状态转换的连续性
    /// ④ 更新根累加器：删除旧的LN.Acc，添加新的LN.Acc
    pub fn audit_update(
        &self,
        proof: &UpdateProof,
        root_acc: &mut DynamicAccumulator,
    ) -> Result<AuditResult> {
        // ① 验证删除证明
        if let Some(ref delete_proof) = proof.delete_value_proof {
            if !delete_proof.verify() {
                return Ok(AuditResult::failure("Delete proof verification failed"));
            }

            // 验证删除证明中的旧累加器值与记录的一致
            if delete_proof.old_acc_value != proof.ln_acc_old {
                return Ok(AuditResult::failure(
                    "Delete proof old accumulator value mismatch",
                ));
            }
        }

        // ② 验证添加证明
        if let Some(ref add_proof) = proof.add_value_proof {
            if !add_proof.verify() {
                return Ok(AuditResult::failure("Add proof verification failed"));
            }

            // 验证添加证明中的新累加器值与记录的一致
            if add_proof.new_acc_value != proof.ln_acc_new {
                return Ok(AuditResult::failure(
                    "Add proof new accumulator value mismatch",
                ));
            }
        }

        // ③ 验证累加器状态转换的连续性
        // 删除的新累加器值应该等于添加的旧累加器值
        if let (Some(ref delete_proof), Some(ref add_proof)) =
            (&proof.delete_value_proof, &proof.add_value_proof)
        {
            if delete_proof.new_acc_value != add_proof.old_acc_value {
                return Ok(AuditResult::failure(
                    "Accumulator state transition is not continuous",
                ));
            }
        }

        // ④ 更新根累加器：删除旧的LN.Acc，添加新的LN.Acc
        let old_acc_value = Self::acc_to_value(&proof.ln_acc_old);
        if root_acc.len() > 0 {
            let _ = root_acc.delete(&old_acc_value);
        }

        let new_acc_value = Self::acc_to_value(&proof.ln_acc_new);
        root_acc.add(&new_acc_value)?;

        Ok(AuditResult::success())
    }

    /// 查询键值对
    ///
    /// 实现查询逻辑：
    /// 步骤1: 根据路径定位至对应叶子节点
    /// 步骤2: 若所查询的值存在，则返回对应的累加器值
    /// 步骤3: 若所查询的值不存在，则构造不存在证明：
    ///   3.1 获取查询key的前序叶子节点的keyp和后序叶子节点LNn、keyn
    ///   3.2 返回keyp、keyn、LNn.Acc给Auditor进行验证
    ///
    /// # Arguments
    ///
    /// * `key` - 要查询的键
    /// * `value` - 要查询的值
    ///
    /// # Returns
    ///
    /// 返回查询结果，包含存在证明或不存在证明
    pub fn query(&self, key: &Key, value: Value) -> Result<QueryResult> {
        // 步骤1: 根据路径定位至对应叶子节点
        let leaf_opt = self.find_leaf(key);

        match leaf_opt {
            Some(leaf_ref) => {
                // 叶子节点存在，检查值是否存在
                let (contains_value, ln_acc) = {
                    let leaf = leaf_ref.read().unwrap();
                    if let Node::Leaf(ln) = &*leaf {
                        (ln.contains_value(&value), ln.accumulator_value())
                    } else {
                        return Err(anyhow!("Expected leaf node"));
                    }
                };

                if contains_value {
                    // 步骤2: 值存在，返回累加器值
                    // 生成成员证明
                    let membership_proof = {
                        let leaf = leaf_ref.read().unwrap();
                        if let Node::Leaf(ln) = &*leaf {
                            ln.acc.prove_membership(&value).ok()
                        } else {
                            None
                        }
                    };

                    Ok(QueryResult::Exists(ExistenceProof {
                        key: key.clone(),
                        value,
                        ln_acc,
                        membership_proof,
                    }))
                } else {
                    // 值不存在于叶子节点中
                    self.construct_non_existence_proof(key)
                }
            }
            None => {
                // 步骤3: 叶子节点不存在，构造不存在证明
                self.construct_non_existence_proof(key)
            }
        }
    }

    /// 构造不存在证明
    ///
    /// 步骤3.1: 获取查询key的前序叶子节点的keyp和后序叶子节点LNn、keyn
    /// 步骤3.2: 返回keyp、keyn、LNn.Acc给Auditor进行验证
    fn construct_non_existence_proof(&self, key: &Key) -> Result<QueryResult> {
        // 在链表中找到前序和后序叶子节点
        let (key_prev, key_next, ln_next_acc, prev_in_next_proof) =
            self.find_predecessor_and_successor(key)?;

        Ok(QueryResult::NotExists(NonExistenceProof {
            key: key.clone(),
            key_prev,
            key_next,
            ln_next_acc,
            prev_in_next_proof,
        }))
    }

    /// 查找给定键的前序和后序叶子节点
    ///
    /// # Returns
    ///
    /// 返回 (key_prev, key_next, ln_next_acc, prev_in_next_proof)
    fn find_predecessor_and_successor(
        &self,
        key: &Key,
    ) -> Result<(
        Option<Key>,
        Option<Key>,
        Option<G1Affine>,
        Option<crate::acctrie::acc::dynamic_accumulator::MembershipProof>,
    )> {
        let mut key_prev: Option<Key> = None;
        let mut key_next: Option<Key> = None;
        let mut ln_next_acc: Option<G1Affine> = None;
        let mut prev_in_next_proof: Option<
            crate::acctrie::acc::dynamic_accumulator::MembershipProof,
        > = None;

        // 遍历叶子链表
        let mut current = self.head_leaf.clone();

        while let Some(current_ref) = current {
            let (current_key, next_opt) = {
                let node = current_ref.read().unwrap();
                if let Node::Leaf(leaf) = &*node {
                    (leaf.get_full_key().clone(), leaf.next.clone())
                } else {
                    return Err(anyhow!("Expected leaf node in list"));
                }
            };

            if &current_key < key {
                // 当前节点的键小于查询键，可能是前序节点
                key_prev = Some(current_key.clone());

                // 检查下一个节点
                if let Some(ref next_ref) = next_opt {
                    let next_key = {
                        let next_node = next_ref.read().unwrap();
                        if let Node::Leaf(next_leaf) = &*next_node {
                            next_leaf.get_full_key().clone()
                        } else {
                            return Err(anyhow!("Expected leaf node"));
                        }
                    };

                    if &next_key > key {
                        // 找到了夹在中间的位置
                        key_next = Some(next_key);

                        // 获取后序节点的累加器值和证明
                        let next_node = next_ref.read().unwrap();
                        if let Node::Leaf(next_leaf) = &*next_node {
                            ln_next_acc = Some(next_leaf.accumulator_value());

                            // 生成前序键在后序累加器中的成员证明
                            // 注意：keyp应该在LNn.Acc中
                            let keyp_hash = LeafNode::key_to_value(&current_key);
                            prev_in_next_proof = next_leaf.acc.prove_membership(&keyp_hash).ok();
                        }

                        break;
                    }
                }
            } else if &current_key > key {
                // 当前节点的键大于查询键，当前节点就是后序节点
                key_next = Some(current_key.clone());

                // 获取后序节点的累加器值
                let node = current_ref.read().unwrap();
                if let Node::Leaf(leaf) = &*node {
                    ln_next_acc = Some(leaf.accumulator_value());

                    // 如果有前序键，生成证明
                    if let Some(ref kp) = key_prev {
                        let keyp_hash = LeafNode::key_to_value(kp);
                        prev_in_next_proof = leaf.acc.prove_membership(&keyp_hash).ok();
                    }
                }

                break;
            }

            current = next_opt;
        }

        Ok((key_prev, key_next, ln_next_acc, prev_in_next_proof))
    }

    /// Auditor验证查询结果
    ///
    /// 对于存在证明：验证值在累加器中的成员关系
    /// 对于不存在证明：
    ///   ① 验证keyp < key < keyn
    ///   ② 验证keyp在LNn.Acc中是否存在
    pub fn audit_query(result: &QueryResult) -> Result<AuditResult> {
        match result {
            QueryResult::Exists(proof) => {
                // 验证成员证明
                if let Some(ref membership_proof) = proof.membership_proof {
                    if membership_proof.verify(proof.ln_acc) {
                        Ok(AuditResult::success())
                    } else {
                        Ok(AuditResult::failure("Membership proof verification failed"))
                    }
                } else {
                    // 没有提供成员证明，只能基于累加器值进行基本验证
                    Ok(AuditResult::success())
                }
            }
            QueryResult::NotExists(proof) => {
                // ① 验证keyp < key < keyn
                if let Some(ref key_prev) = proof.key_prev {
                    if key_prev >= &proof.key {
                        return Ok(AuditResult::failure("key_prev must be less than key"));
                    }
                }

                if let Some(ref key_next) = proof.key_next {
                    if &proof.key >= key_next {
                        return Ok(AuditResult::failure("key must be less than key_next"));
                    }
                }

                // ② 验证keyp在LNn.Acc中是否存在
                if let (Some(_), Some(ln_next_acc), Some(ref prev_proof)) = (
                    &proof.key_prev,
                    proof.ln_next_acc,
                    &proof.prev_in_next_proof,
                ) {
                    if !prev_proof.verify(ln_next_acc) {
                        return Ok(AuditResult::failure(
                            "key_prev not found in next leaf accumulator",
                        ));
                    }
                }

                Ok(AuditResult::success())
            }
        }
    }
}

impl Default for AccTrie {
    fn default() -> Self {
        Self::new()
    }
}
