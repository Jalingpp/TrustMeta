use accumulator_ads::{DynamicAccumulator, G1Affine, Set, digest_set_from_set};
use std::sync::Arc;

use crate::result::{PathElement, TreeMatchResult};
use crate::utils::{Hash, nonleaf_hash};

#[derive(Debug, Clone)]
pub enum Node {
    Leaf {
        hash: Hash,
        key: String,
        fid: String,
        level: usize,
    },
    NonLeaf {
        hash: Hash,
        keys: Arc<Set<String>>,
        acc: G1Affine,
        level: usize,
        left: Box<Node>,
        right: Box<Node>,
    },
}

impl Node {
    pub fn new(key: String, fid: String) -> Box<Self> {
        Box::new(Node::Leaf {
            hash: crate::utils::leaf_hash(&key, &fid),
            key,
            fid,
            level: 0,
        })
    }

    pub fn level(&self) -> usize {
        match self {
            Node::Leaf { level, .. } => *level,
            Node::NonLeaf { level, .. } => *level,
        }
    }

    pub fn hash(&self) -> &Hash {
        match self {
            Node::Leaf { hash, .. } => hash,
            Node::NonLeaf { hash, .. } => hash,
        }
    }

    pub fn acc(&self) -> G1Affine {
        match self {
            Node::Leaf { key, .. } => {
                let digest_set = digest_set_from_set(&Set::from_vec(vec![key.to_string()]));
                DynamicAccumulator::calculate_commitment(&digest_set)
            }
            Node::NonLeaf { acc, .. } => *acc,
        }
    }

    pub fn keys(&self) -> Arc<Set<String>> {
        match self {
            Node::Leaf { key, .. } => Arc::new(Set::from_vec(vec![key.clone()])),
            Node::NonLeaf { keys, .. } => keys.clone(),
        }
    }

    pub fn has_key(&self, target_key: &str) -> bool {
        match self {
            Node::Leaf { key, .. } => key == target_key,
            Node::NonLeaf { keys, .. } => keys.contains(target_key),
        }
    }

    pub fn collect_leaves(
        &self,
        exclude_target: Option<(&str, &str)>,
    ) -> std::vec::IntoIter<(String, String)> {
        let mut values: Vec<(String, String)> = Vec::new();
        match self {
            Node::Leaf { key, fid, .. } => {
                if let Some((exclude_key, exclude_fid)) = exclude_target {
                    if key == exclude_key && fid == exclude_fid {
                        return values.into_iter();
                    }
                }
                values.push((key.clone(), fid.clone()));
            }
            Node::NonLeaf { left, right, .. } => {
                values.extend(left.collect_leaves(exclude_target));
                values.extend(right.collect_leaves(exclude_target));
            }
        }
        values.into_iter()
    }

    pub fn select(&self, target_key: &str) -> Option<&str> {
        match self {
            Node::Leaf { key, fid, .. } => {
                if key == target_key {
                    Some(fid.as_str())
                } else {
                    None
                }
            }
            Node::NonLeaf { left, right, .. } => {
                if left.has_key(target_key) {
                    left.select(target_key)
                } else if right.has_key(target_key) {
                    right.select(target_key)
                } else {
                    None
                }
            }
        }
    }

    pub fn collect_matches(&self, target_key: &str) -> Vec<(String, Vec<PathElement>)> {
        let mut results = Vec::new();
        let mut stack = vec![(self, Vec::new())];

        while let Some((node, path)) = stack.pop() {
            match node {
                Node::Leaf { key, fid, .. } => {
                    if key == target_key {
                        results.push((fid.clone(), path));
                    }
                }
                Node::NonLeaf {
                    left,
                    right,
                    keys,
                    acc,
                    ..
                } => {
                    let left_keys = left.keys();
                    let right_keys = right.keys();

                    let mut left_path = path.clone();
                    left_path.push(PathElement {
                        sibling_hash: *right.hash(),
                        is_left_sibling: false,
                        sibling_keys: right_keys.as_ref().clone(),
                        sibling_acc: right.acc(),
                        parent_keys: keys.as_ref().clone(),
                        parent_acc: *acc,
                    });
                    stack.push((left, left_path));

                    let mut right_path = path;
                    right_path.push(PathElement {
                        sibling_hash: *left.hash(),
                        is_left_sibling: true,
                        sibling_keys: left_keys.as_ref().clone(),
                        sibling_acc: left.acc(),
                        parent_keys: keys.as_ref().clone(),
                        parent_acc: *acc,
                    });
                    stack.push((right, right_path));
                }
            }
        }

        results
    }

    pub fn find_all_matches(&self, target_key: &str, tree_root_hash: Hash) -> Vec<TreeMatchResult> {
        self.collect_matches(target_key)
            .into_iter()
            .map(|(fid, path)| TreeMatchResult {
                fid,
                path,
                tree_root_hash,
            })
            .collect()
    }

    pub fn merge(left: Box<Node>, right: Box<Node>, level: Option<usize>) -> Box<Node> {
        let new_keys = Arc::new(left.keys().union(&right.keys()));
        let left_acc = left.acc();
        let diff_elements = right.keys().difference(&left.keys());
        let diff_fr = digest_set_from_set(&diff_elements);
        let new_acc = DynamicAccumulator::incremental_add_with_default_trapdoor(left_acc, &diff_fr);

        Box::new(Node::NonLeaf {
            hash: nonleaf_hash(*left.hash(), *right.hash(), &new_keys, &new_acc),
            keys: new_keys,
            acc: new_acc,
            level: level.unwrap_or_else(|| right.level() + 1),
            left,
            right,
        })
    }
}
