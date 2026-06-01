use crate::acctree::accumulator_ads::{DynamicAccumulator, G1Affine, Set, digest_set_from_set};
use crate::acctree::result::{PathElement, TreeMatchResult};
use crate::acctree::utils::{Hash, leaf_hash, nonleaf_hash};
use std::rc::Rc;

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
        keys: Rc<Set<String>>,
        acc: G1Affine,
        level: usize,
        left: Box<Node>,
        right: Box<Node>,
    },
}

impl Node {
    pub fn new(key: String, fid: String) -> Box<Self> {
        Box::new(Self::Leaf {
            hash: leaf_hash(&key, &fid),
            key,
            fid,
            level: 0,
        })
    }

    pub fn level(&self) -> usize {
        match self {
            Self::Leaf { level, .. } | Self::NonLeaf { level, .. } => *level,
        }
    }

    pub fn hash(&self) -> &Hash {
        match self {
            Self::Leaf { hash, .. } | Self::NonLeaf { hash, .. } => hash,
        }
    }

    pub fn acc(&self) -> G1Affine {
        match self {
            Self::Leaf { key, .. } => {
                let digest_set = digest_set_from_set(&Set::from_vec(vec![key.clone()]));
                DynamicAccumulator::calculate_commitment(&digest_set)
            }
            Self::NonLeaf { acc, .. } => *acc,
        }
    }

    pub fn keys(&self) -> Rc<Set<String>> {
        match self {
            Self::Leaf { key, .. } => Rc::new(Set::from_vec(vec![key.clone()])),
            Self::NonLeaf { keys, .. } => Rc::clone(keys),
        }
    }

    pub fn has_key(&self, target_key: &str) -> bool {
        match self {
            Self::Leaf { key, .. } => key == target_key,
            Self::NonLeaf { keys, .. } => keys.contains(target_key),
        }
    }

    pub fn collect_leaves(
        &self,
        exclude_target: Option<(&str, &str)>,
    ) -> std::vec::IntoIter<(String, String)> {
        let mut leaves = Vec::new();
        match self {
            Self::Leaf { key, fid, .. } => {
                if let Some((exclude_key, exclude_fid)) = exclude_target {
                    if key == exclude_key && fid == exclude_fid {
                        return leaves.into_iter();
                    }
                }
                leaves.push((key.clone(), fid.clone()));
            }
            Self::NonLeaf { left, right, .. } => {
                leaves.extend(left.collect_leaves(exclude_target));
                leaves.extend(right.collect_leaves(exclude_target));
            }
        }
        leaves.into_iter()
    }

    pub fn collect_matches(&self, target_key: &str) -> Vec<(String, Vec<PathElement>)> {
        let mut results = Vec::new();
        let mut stack = vec![(self, Vec::new())];

        while let Some((node, path)) = stack.pop() {
            match node {
                Self::Leaf { key, fid, .. } => {
                    if key == target_key {
                        results.push((fid.clone(), path));
                    }
                }
                Self::NonLeaf {
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
                        sibling_keys: right_keys,
                        sibling_acc: right.acc(),
                        parent_keys: Rc::clone(keys),
                        parent_acc: *acc,
                    });
                    stack.push((left.as_ref(), left_path));

                    let mut right_path = path;
                    right_path.push(PathElement {
                        sibling_hash: *left.hash(),
                        is_left_sibling: true,
                        sibling_keys: left_keys,
                        sibling_acc: left.acc(),
                        parent_keys: Rc::clone(keys),
                        parent_acc: *acc,
                    });
                    stack.push((right.as_ref(), right_path));
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
        let new_keys = Rc::new(left.keys().union(&right.keys()));
        let left_acc = left.acc();
        let diff_elements = right.keys().difference(&left.keys());
        let diff_fr = digest_set_from_set(&diff_elements);
        let new_acc = DynamicAccumulator::incremental_add_with_default_trapdoor(left_acc, &diff_fr);

        Box::new(Self::NonLeaf {
            hash: nonleaf_hash(*left.hash(), *right.hash(), &new_keys, &new_acc),
            keys: new_keys,
            acc: new_acc,
            level: level.unwrap_or_else(|| right.level() + 1),
            left,
            right,
        })
    }
}
