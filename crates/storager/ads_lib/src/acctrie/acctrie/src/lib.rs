use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use acc::{Acc, MultiSet, G1Affine};
use acc::acc_mod::DigestSet;
use acc::Accumulator;

type LeafRef = Rc<RefCell<LeafNode>>;

#[derive(Debug)]
pub struct LeafNode {
    pub suffix: String,
    pub values: Vec<i64>,
    pub acc: Option<G1Affine>,
    pub prev: Option<LeafRef>,
    pub next: Option<LeafRef>,
}

impl LeafNode {
    pub fn new(suffix: String) -> Self {
        Self {
            suffix,
            values: Vec::new(),
            acc: None,
            prev: None,
            next: None,
        }
    }

    pub fn add_value(&mut self, v: i64) {
        self.values.push(v);
    }

    /// Acc is simply the sum of `values` per specification.
    pub fn recompute_acc(&mut self) {
        if self.values.is_empty() {
            self.acc = None;
            return;
        }
        let ms = MultiSet::from_vec(self.values.clone());
        let ds = DigestSet::new(&ms);
        let ag1 = Acc::cal_acc_g1_d(&ds);
        self.acc = Some(ag1);
    }
}

#[derive(Debug)]
pub struct ExtensionNode {
    pub key_part: String,
    pub children: HashMap<char, Box<ExtensionNode>>,
    pub leaf: Option<LeafRef>,
}

impl ExtensionNode {
    pub fn new(key_part: String) -> Self {
        Self {
            key_part,
            children: HashMap::new(),
            leaf: None,
        }
    }
}

#[derive(Debug)]
pub struct AccTrie {
    pub root: Box<ExtensionNode>,
    pub first_leaf: Option<LeafRef>,
    pub last_leaf: Option<LeafRef>,
}

#[derive(Debug)]
pub struct InsertResult {
    pub ln_acc_old: Option<G1Affine>,
    pub ln_acc_new: Option<G1Affine>,
    pub ln_elem_proof: Option<G1Affine>,
    pub keyp: String,
    pub lnp_acc: Option<G1Affine>,
    pub keyn: Option<String>,
    pub lnn_acc_old: Option<G1Affine>,
    pub lnn_acc_new: Option<G1Affine>,
}

#[derive(Debug)]
pub struct DeleteResult {
    pub ln_acc_old: Option<G1Affine>,
    pub ln_acc_new: Option<G1Affine>,
    pub ln_elem_proof: Option<G1Affine>,
    pub keyp: String,
    pub keyn: Option<String>,
    pub lnn_acc_old: Option<G1Affine>,
    pub lnn_acc_new: Option<G1Affine>,
}

#[derive(Debug)]
pub struct UpdateResult {
    pub ln_acc_old: Option<G1Affine>,
    pub ln_acc_new: Option<G1Affine>,
    pub ln_elem_proof: Option<G1Affine>,
    pub keyp: String,
    pub keyn: Option<String>,
    pub lnn_acc_old: Option<G1Affine>,
    pub lnn_acc_new: Option<G1Affine>,
}

impl AccTrie {
    pub fn new() -> Self {
        Self {
            root: Box::new(ExtensionNode::new(String::new())),
            first_leaf: None,
            last_leaf: None,
        }
    }

    /// Find or create leaf at appropriate (lexicographic) position, insert value,
    /// and return `InsertResult` containing neighbor keys and optional acc snapshots.
    pub fn insert(&mut self, full_key: &str, value: i64) -> InsertResult {
        // traverse extension nodes
        let mut node = &mut *self.root;
        for ch in full_key.chars() {
            node = node
                .children
                .entry(ch)
                .or_insert_with(|| Box::new(ExtensionNode::new(ch.to_string())));
        }

        // find or create leaf and insert into sorted doubly-linked list
        let leaf_ref = if let Some(l) = node.leaf.as_ref() {
            l.clone()
        } else {
            let new_leaf = Rc::new(RefCell::new(LeafNode::new(full_key.to_string())));
            match self.first_leaf.as_ref() {
                None => {
                    self.first_leaf = Some(new_leaf.clone());
                    self.last_leaf = Some(new_leaf.clone());
                }
                Some(first) => {
                    let mut cur_opt = Some(first.clone());
                    let mut inserted = false;
                    while let Some(cur) = cur_opt {
                        if cur.borrow().suffix.as_str() >= full_key {
                            let prev_opt = cur.borrow().prev.clone();
                            new_leaf.borrow_mut().next = Some(cur.clone());
                            new_leaf.borrow_mut().prev = prev_opt.clone();
                            if let Some(prev) = prev_opt {
                                prev.borrow_mut().next = Some(new_leaf.clone());
                            } else {
                                self.first_leaf = Some(new_leaf.clone());
                            }
                            cur.borrow_mut().prev = Some(new_leaf.clone());
                            inserted = true;
                            break;
                        }
                        cur_opt = cur.borrow().next.clone();
                    }
                    if !inserted {
                        let last = self.last_leaf.as_ref().unwrap().clone();
                        last.borrow_mut().next = Some(new_leaf.clone());
                        new_leaf.borrow_mut().prev = Some(last.clone());
                        self.last_leaf = Some(new_leaf.clone());
                    }
                }
            }
            node.leaf = Some(new_leaf.clone());
            new_leaf
        };

        // capture neighbors and existing acc snapshots
        let lnp_opt = leaf_ref.borrow().prev.clone();
        let keyp = lnp_opt
            .as_ref()
            .map(|p| p.borrow().suffix.clone())
            .unwrap_or_else(|| "NoPrev".to_string());
        let lnp_acc: Option<G1Affine> = lnp_opt.as_ref().and_then(|p| p.borrow().acc);

        let lnn_opt = leaf_ref.borrow().next.clone();
        let keyn = lnn_opt.as_ref().map(|n| n.borrow().suffix.clone());
        let lnn_acc_old: Option<G1Affine> = lnn_opt.as_ref().and_then(|n| n.borrow().acc);

        // capture old ln acc, update values and recompute acc
        let ln_old = leaf_ref.borrow().acc;

        // perform single-element insertion using dynamic Acc API when possible
        let mut ln_elem_proof: Option<G1Affine> = None;
        if let Some(old_acc) = ln_old {
            // have old_acc: use Acc::add_element to compute new acc and proof
            leaf_ref.borrow_mut().add_value(value);
            let (new_acc, proof) = Acc::add_element(&old_acc, &value);
            leaf_ref.borrow_mut().acc = Some(new_acc);
            ln_elem_proof = Some(proof);
        } else {
            // no old accumulator: fall back to full recompute
            leaf_ref.borrow_mut().add_value(value);
            leaf_ref.borrow_mut().recompute_acc();
            ln_elem_proof = None;
        }

        // extract new accumulator snapshots into locals to avoid borrow conflicts
        let ln_new = { let b = leaf_ref.borrow(); b.acc };
        let lnn_acc_new = lnn_opt.as_ref().and_then(|n| n.borrow().acc);

        InsertResult {
            ln_acc_old: ln_old,
            ln_acc_new: ln_new,
            ln_elem_proof,
            keyp,
            lnp_acc,
            keyn,
            lnn_acc_old,
            lnn_acc_new,
        }
    }

    /// Delete by key. If `remove_values` is `None` the entire leaf is removed;
    /// if `Some(&[...])` the listed values are removed from the leaf (partial delete).
    pub fn delete(&mut self, full_key: &str, remove_values: Option<&[i64]>) -> Result<DeleteResult, String> {
        // locate node by traversing extension nodes
        let mut node = &mut *self.root;
        for ch in full_key.chars() {
            match node.children.get_mut(&ch) {
                Some(child) => node = child,
                None => return Err(format!("key not found: {}", full_key)),
            }
        }

        let leaf_ref = if let Some(l) = node.leaf.as_ref() {
            l.clone()
        } else {
            return Err(format!("leaf not found for key: {}", full_key));
        };

        // capture old acc and neighbors
        let ln_old = leaf_ref.borrow().acc;
        let lnp_opt = leaf_ref.borrow().prev.clone();
        let keyp = lnp_opt
            .as_ref()
            .map(|p| p.borrow().suffix.clone())
            .unwrap_or_else(|| "NoPrev".to_string());
        let lnn_opt = leaf_ref.borrow().next.clone();
        let keyn = lnn_opt.as_ref().map(|n| n.borrow().suffix.clone());

        match remove_values {
            None => {
                // full delete: relink prev and next
                if let Some(prev) = leaf_ref.borrow().prev.clone() {
                    prev.borrow_mut().next = leaf_ref.borrow().next.clone();
                } else {
                    self.first_leaf = leaf_ref.borrow().next.clone();
                }

                if let Some(next) = leaf_ref.borrow().next.clone() {
                    next.borrow_mut().prev = leaf_ref.borrow().prev.clone();
                } else {
                    self.last_leaf = leaf_ref.borrow().prev.clone();
                }

                // detach from extension node
                node.leaf = None;

                let lnn_acc_old = lnn_opt.as_ref().and_then(|n| n.borrow().acc);
                let lnn_acc_new = lnn_acc_old;

                Ok(DeleteResult {
                    ln_acc_old: ln_old,
                    ln_acc_new: None,
                    ln_elem_proof: None,
                    keyp,
                    keyn,
                    lnn_acc_old,
                    lnn_acc_new,
                })
            }
            Some(vals) => {
                // partial delete: remove listed values
                    // capture old values before mutation for proof construction
                    let old_vals = leaf_ref.borrow().values.clone();
                    {
                        let mut leaf = leaf_ref.borrow_mut();
                        for &v in vals {
                            leaf.values.retain(|x| *x != v);
                        }
                        leaf.recompute_acc();
                    }

                    // build per-element removal proof(s) when possible
                    let mut ln_elem_proof: Option<G1Affine> = None;
                    if let Some(mut acc_cur) = ln_old {
                        // apply removes sequentially; update accumulator via Acc::remove_element
                        let mut acc_val = acc_cur;
                        for &v in vals.iter() {
                            let (new_acc, proof) = Acc::remove_element(&acc_val, &v);
                            acc_val = new_acc;
                            ln_elem_proof = Some(proof);
                        }
                        // update stored acc (if resulting set empty, set None)
                        if leaf_ref.borrow().values.is_empty() {
                            leaf_ref.borrow_mut().acc = None;
                        } else {
                            leaf_ref.borrow_mut().acc = Some(acc_val);
                        }
                    } else {
                        // no old accumulator: just recompute
                        leaf_ref.borrow_mut().recompute_acc();
                        ln_elem_proof = None;
                    }

                    let ln_new = leaf_ref.borrow().acc;

                    Ok(DeleteResult {
                        ln_acc_old: ln_old,
                        ln_acc_new: ln_new,
                        ln_elem_proof,
                        keyp,
                        keyn,
                        lnn_acc_old: None,
                        lnn_acc_new: None,
                    })
            }
        }
    }

    /// Update a leaf's values and/or key. Public wrapper that performs the operation.
    pub fn update(&mut self, full_key: &str, new_values: Option<&[i64]>, new_key: Option<&str>) -> Result<UpdateResult, String> {
        // locate node
        let mut node = &mut *self.root;
        for ch in full_key.chars() {
            match node.children.get_mut(&ch) {
                Some(child) => node = child,
                None => return Err(format!("key not found: {}", full_key)),
            }
        }

        let leaf_ref = if let Some(l) = node.leaf.as_ref() {
            l.clone()
        } else {
            return Err(format!("leaf not found for key: {}", full_key));
        };

        // capture old acc and neighbors
        let ln_old = leaf_ref.borrow().acc;
        let lnp_opt = leaf_ref.borrow().prev.clone();
        let keyp = lnp_opt
            .as_ref()
            .map(|p| p.borrow().suffix.clone())
            .unwrap_or_else(|| "NoPrev".to_string());
        let lnn_opt = leaf_ref.borrow().next.clone();
        let keyn = lnn_opt.as_ref().map(|n| n.borrow().suffix.clone());
        let lnn_acc_old = lnn_opt.as_ref().and_then(|n| n.borrow().acc);

            // apply value replacement if requested
            let mut ln_elem_proof_local: Option<G1Affine> = None;
            if let Some(vals) = new_values {
                let old_vals = leaf_ref.borrow().values.clone();
                let old_acc_opt = leaf_ref.borrow().acc;

                if let Some(mut acc_cur) = old_acc_opt {
                    // compute multiset differences (counts)
                    let mut cnt_old: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
                    for v in old_vals.iter() { *cnt_old.entry(*v).or_default() += 1; }
                    let mut cnt_new: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
                    for v in vals.iter() { *cnt_new.entry(*v).or_default() += 1; }

                    // removes: elements with count_old > count_new
                    for (val, &co) in cnt_old.iter() {
                        let cn = *cnt_new.get(val).unwrap_or(&0);
                        for _ in 0..(co - cn).max(0) {
                            let (new_acc, proof) = Acc::remove_element(&acc_cur, val);
                            acc_cur = new_acc;
                            ln_elem_proof_local = Some(proof);
                        }
                    }
                    // adds: elements with count_new > count_old
                    for (val, &cn) in cnt_new.iter() {
                        let co = *cnt_old.get(val).unwrap_or(&0);
                        for _ in 0..(cn - co).max(0) {
                            let (new_acc, proof) = Acc::add_element(&acc_cur, val);
                            acc_cur = new_acc;
                            ln_elem_proof_local = Some(proof);
                        }
                    }
                    // write resulting acc and values
                    leaf_ref.borrow_mut().values = vals.to_vec();
                    if leaf_ref.borrow().values.is_empty() {
                        leaf_ref.borrow_mut().acc = None;
                    } else {
                        leaf_ref.borrow_mut().acc = Some(acc_cur);
                    }
                } else {
                    // no old accumulator; just replace values and recompute
                    let mut leaf = leaf_ref.borrow_mut();
                    leaf.values = vals.to_vec();
                    leaf.recompute_acc();
                    ln_elem_proof_local = None;
                }
        }

        // handle key change (move)
        if let Some(nk) = new_key {
            if nk != full_key {
                // unlink from current position
                if let Some(prev) = leaf_ref.borrow().prev.clone() {
                    prev.borrow_mut().next = leaf_ref.borrow().next.clone();
                } else {
                    self.first_leaf = leaf_ref.borrow().next.clone();
                }
                if let Some(next) = leaf_ref.borrow().next.clone() {
                    next.borrow_mut().prev = leaf_ref.borrow().prev.clone();
                } else {
                    self.last_leaf = leaf_ref.borrow().prev.clone();
                }
                // detach from old extension node
                node.leaf = None;

                // change suffix/key
                leaf_ref.borrow_mut().suffix = nk.to_string();

                // insert at new position
                let mut ins_node = &mut *self.root;
                for ch in nk.chars() {
                    ins_node = ins_node
                        .children
                        .entry(ch)
                        .or_insert_with(|| Box::new(ExtensionNode::new(ch.to_string())));
                }
                // insert into sorted leaf list at new spot
                if let Some(existing) = ins_node.leaf.as_ref() {
                    // if leaf with new key exists, append values into it
                    existing
                        .borrow_mut()
                        .values
                        .extend(leaf_ref.borrow().values.iter());
                    existing.borrow_mut().recompute_acc();
                    // old leaf object removed
                } else {
                    let new_leaf = leaf_ref.clone();
                    // find insertion point starting at first
                    match self.first_leaf.as_ref() {
                        None => {
                            self.first_leaf = Some(new_leaf.clone());
                            self.last_leaf = Some(new_leaf.clone());
                        }
                        Some(first) => {
                            let mut cur_opt = Some(first.clone());
                            let mut inserted = false;
                            while let Some(cur) = cur_opt {
                                if cur.borrow().suffix.as_str() >= nk {
                                    let prev_opt = cur.borrow().prev.clone();
                                    new_leaf.borrow_mut().next = Some(cur.clone());
                                    new_leaf.borrow_mut().prev = prev_opt.clone();
                                    if let Some(prev) = prev_opt {
                                        prev.borrow_mut().next = Some(new_leaf.clone());
                                    } else {
                                        self.first_leaf = Some(new_leaf.clone());
                                    }
                                    cur.borrow_mut().prev = Some(new_leaf.clone());
                                    inserted = true;
                                    break;
                                }
                                cur_opt = cur.borrow().next.clone();
                            }
                            if !inserted {
                                let last = self.last_leaf.as_ref().unwrap().clone();
                                last.borrow_mut().next = Some(new_leaf.clone());
                                new_leaf.borrow_mut().prev = Some(last.clone());
                                self.last_leaf = Some(new_leaf.clone());
                            }
                        }
                    }
                    ins_node.leaf = Some(new_leaf.clone());
                }
            }
        }

        // after changes, capture new ln acc and lnn acc
        let ln_new = leaf_ref.borrow().acc;
        let lnn_acc_new = lnn_opt.as_ref().and_then(|n| n.borrow().acc);

        // use proof generated earlier (if any)
            let ln_elem_proof = ln_elem_proof_local;

        Ok(UpdateResult {
            ln_acc_old: ln_old,
            ln_acc_new: ln_new,
                ln_elem_proof,
            keyp,
            keyn,
            lnn_acc_old,
            lnn_acc_new,
        })
    }

    /// Query a leaf by key, returning (values, acc, prev_key, next_key).
    pub fn query(&self, full_key: &str) -> Option<(Vec<i64>, Option<G1Affine>, Option<String>, Option<String>)> {
        let mut node = &*self.root;
        for ch in full_key.chars() {
            match node.children.get(&ch) {
                Some(child) => node = &**child,
                None => return None,
            }
        }
        if let Some(l) = node.leaf.as_ref() {
            let b = l.borrow();
            let prev_key = b.prev.as_ref().map(|p| p.borrow().suffix.clone());
            let next_key = b.next.as_ref().map(|n| n.borrow().suffix.clone());
            return Some((b.values.clone(), b.acc, prev_key, next_key));
        }
        None
    }

    /// Return a vector of (key_part, optional_leaf_key) for each step towards `full_key`.
    pub fn path_to_leaf(&self, full_key: &str) -> Vec<(String, Option<String>)> {
        let mut path = Vec::new();
        let mut node = &*self.root;
        for ch in full_key.chars() {
            // look ahead to the child for this character and report whether that child has a leaf
            if let Some(child) = node.children.get(&ch) {
                let leaf_key = child.leaf.as_ref().map(|l| l.borrow().suffix.clone());
                path.push((ch.to_string(), leaf_key));
                node = &**child;
            } else {
                // still include the key part even if the path breaks
                path.push((ch.to_string(), None));
                break;
            }
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_insert_and_acc() {
        let mut t = AccTrie::new();
        t.insert("abc", 1);
        t.insert("abd", 2);
        t.insert("abe", 3);

        // traverse leaves in order and check values and ordering
        let mut node = t.first_leaf.clone();
        let mut keys = Vec::new();
        let mut sums = Vec::new();
        while let Some(n) = node {
            let b = n.borrow();
            let ssum: i64 = b.values.iter().copied().sum();
            sums.push(ssum);
            keys.push(b.suffix.clone());
            node = b.next.clone();
        }

        assert_eq!(keys, vec!["abc", "abd", "abe"]);
        assert_eq!(sums, vec![1, 2, 3]);
    }

    #[test]
    fn path_to_leaf_returns_key_parts() {
        let mut t = AccTrie::new();
        t.insert("xy", 5);
        let path = t.path_to_leaf("xy");
        // should have 'x' and 'y' as parts
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].0, "x");
        assert_eq!(path[1].0, "y");
        assert!(path[0].1.is_none());
    }
}
