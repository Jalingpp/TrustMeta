use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EPRingEntry {
    pub prefix: String,
    pub counter: u64,
    /// Leaf entry: storager node index.
    /// Split entry: array index of the first child entry in `entries`.
    pub owner_ref: usize,
    pub root_summary: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EPRingRoute {
    pub prefix: String,
    pub entry_index: usize,
    pub node_index: usize,
    pub key_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EPRingSplitEvent {
    pub parent_prefix: String,
    pub original_owner_index: usize,
    pub child_routes: Vec<EPRingRoute>,
}

#[derive(Debug, Clone)]
pub struct EPRing {
    entries: Vec<EPRingEntry>,
    prefix_index: HashMap<String, usize>,
    node_names: Vec<String>,
    root_prefix_len: usize,
    split_threshold: u64,
}

impl EPRing {
    const MAX_PREFIX_LEN: usize = 64;

    pub fn new(node_names: &[String], split_threshold: u64) -> Self {
        if node_names.is_empty() {
            return Self {
                entries: Vec::new(),
                prefix_index: HashMap::new(),
                node_names: Vec::new(),
                root_prefix_len: 0,
                split_threshold,
            };
        }

        let root_prefix_len = Self::minimal_prefix_len(node_names.len());
        let root_entry_count = 16usize.pow(root_prefix_len as u32);
        let mut entries = Vec::with_capacity(root_entry_count);
        let mut prefix_index = HashMap::with_capacity(root_entry_count);

        for idx in 0..root_entry_count {
            let prefix = Self::format_prefix(idx, root_prefix_len);
            prefix_index.insert(prefix.clone(), idx);
            entries.push(EPRingEntry {
                prefix,
                counter: 0,
                owner_ref: idx % node_names.len(),
                root_summary: Vec::new(),
            });
        }

        Self {
            entries,
            prefix_index,
            node_names: node_names.to_vec(),
            root_prefix_len,
            split_threshold,
        }
    }

    fn minimal_prefix_len(node_count: usize) -> usize {
        let mut prefix_len = 0usize;
        let mut capacity = 1usize;
        while capacity < node_count {
            prefix_len += 1;
            capacity *= 16;
        }
        prefix_len
    }

    fn format_prefix(value: usize, prefix_len: usize) -> String {
        if prefix_len == 0 {
            String::new()
        } else {
            format!("{value:0prefix_len$x}")
        }
    }

    pub fn keyword_to_hex(keyword: &str) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let digest = Sha256::digest(keyword.as_bytes());
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }

    pub fn keyword_matches_prefix(keyword: &str, prefix: &str) -> bool {
        Self::keyword_to_hex(keyword).starts_with(prefix)
    }

    fn padded_prefix(key_hex: &str, prefix_len: usize) -> String {
        let mut prefix: String = key_hex.chars().take(prefix_len).collect();
        while prefix.len() < prefix_len {
            prefix.push('0');
        }
        prefix
    }

    fn nibble_at(key_hex: &str, index: usize) -> usize {
        key_hex
            .as_bytes()
            .get(index)
            .map(|byte| match byte {
                b'0'..=b'9' => (byte - b'0') as usize,
                b'a'..=b'f' => (byte - b'a' + 10) as usize,
                b'A'..=b'F' => (byte - b'A' + 10) as usize,
                _ => 0,
            })
            .unwrap_or(0)
    }

    fn entry_is_split(&self, entry_index: usize) -> bool {
        let Some(entry) = self.entries.get(entry_index) else {
            return false;
        };
        if entry.owner_ref < self.node_names.len() {
            return false;
        }
        self.entries
            .get(entry.owner_ref)
            .map(|child| {
                child.prefix.starts_with(&entry.prefix)
                    && child.prefix.len() == entry.prefix.len() + 1
            })
            .unwrap_or(false)
    }

    fn split_entry(&mut self, entry_index: usize) -> Option<EPRingSplitEvent> {
        if self.node_names.is_empty() || self.entry_is_split(entry_index) {
            return None;
        }

        if self.entries.get(entry_index)?.prefix.len() >= Self::MAX_PREFIX_LEN {
            return None;
        }

        let parent_prefix = self.entries[entry_index].prefix.clone();
        let original_owner = self.entries[entry_index].owner_ref;
        let child_start = self.entries.len();
        let mut child_routes = Vec::with_capacity(16);

        for digit in 0..16usize {
            let prefix = format!("{parent_prefix}{digit:x}");
            let child_index = child_start + digit;
            let node_index = (original_owner + digit) % self.node_names.len();
            self.prefix_index.insert(prefix.clone(), child_index);
            self.entries.push(EPRingEntry {
                prefix: prefix.clone(),
                counter: 0,
                owner_ref: node_index,
                root_summary: Vec::new(),
            });
            child_routes.push(EPRingRoute {
                prefix,
                entry_index: child_index,
                node_index,
                key_hex: String::new(),
            });
        }

        self.entries[entry_index].owner_ref = child_start;
        Some(EPRingSplitEvent {
            parent_prefix,
            original_owner_index: original_owner,
            child_routes,
        })
    }

    pub fn maybe_presplit_empty_prefix(
        &mut self,
        prefix: &str,
        incoming_count: u64,
    ) -> Option<EPRingSplitEvent> {
        let entry_index = self.find_entry_index(prefix)?;
        let entry = self.entries.get(entry_index)?;
        if entry.counter != 0
            || incoming_count <= self.split_threshold
            || self.entry_is_split(entry_index)
        {
            return None;
        }
        self.split_entry(entry_index)
    }

    pub fn route_keyword(&self, keyword: &str) -> Option<EPRingRoute> {
        self.route_key_hex(&Self::keyword_to_hex(keyword))
    }

    fn route_key_hex(&self, key_hex: &str) -> Option<EPRingRoute> {
        if self.entries.is_empty() {
            return None;
        }

        let mut entry_index = if self.root_prefix_len == 0 {
            0
        } else {
            let prefix = Self::padded_prefix(key_hex, self.root_prefix_len);
            *self.prefix_index.get(&prefix)?
        };

        loop {
            let entry = self.entries.get(entry_index)?;
            if !self.entry_is_split(entry_index) {
                return Some(EPRingRoute {
                    prefix: entry.prefix.clone(),
                    entry_index,
                    node_index: entry.owner_ref,
                    key_hex: key_hex.to_string(),
                });
            }

            let child_digit = Self::nibble_at(key_hex, entry.prefix.len());
            entry_index = entry.owner_ref + child_digit;
        }
    }

    pub fn node_name(&self, node_index: usize) -> Option<&str> {
        self.node_names.get(node_index).map(|name| name.as_str())
    }

    pub fn find_entry_index(&self, prefix: &str) -> Option<usize> {
        self.prefix_index.get(prefix).copied()
    }

    pub fn update_root_summary(&mut self, prefix: &str, root_summary: Vec<u8>) {
        if let Some(entry_index) = self.find_entry_index(prefix) {
            self.entries[entry_index].root_summary = root_summary;
        }
    }

    pub fn record_insert(
        &mut self,
        prefix: &str,
        root_summary: Vec<u8>,
    ) -> Option<EPRingSplitEvent> {
        if let Some(entry_index) = self.find_entry_index(prefix) {
            self.entries[entry_index].counter = self.entries[entry_index].counter.saturating_add(1);
            self.entries[entry_index].root_summary = root_summary;
            if self.entries[entry_index].counter > self.split_threshold {
                return self.split_entry(entry_index);
            }
        }
        None
    }

    pub fn record_delete(&mut self, prefix: &str, root_summary: Vec<u8>) {
        if let Some(entry_index) = self.find_entry_index(prefix) {
            self.entries[entry_index].counter = self.entries[entry_index].counter.saturating_sub(1);
            self.entries[entry_index].root_summary = root_summary;
        }
    }

    pub fn entries(&self) -> &[EPRingEntry] {
        &self.entries
    }

    pub fn root_prefix_len(&self) -> usize {
        self.root_prefix_len
    }

    pub fn structure_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "root_prefix_len={}, split_threshold={}, entries={}",
            self.root_prefix_len,
            self.split_threshold,
            self.entries.len()
        ));

        for (idx, entry) in self.entries.iter().enumerate() {
            let owner = if self.entry_is_split(idx) {
                format!("split->{}", entry.owner_ref)
            } else {
                let node_name = self.node_name(entry.owner_ref).unwrap_or("unknown");
                format!("{}({})", node_name, entry.owner_ref)
            };
            lines.push(format!(
                "[{}] prefix='{}' counter={} owner={} root_summary_len={}",
                idx,
                entry.prefix,
                entry.counter,
                owner,
                entry.root_summary.len()
            ));
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(count: usize) -> Vec<String> {
        (0..count).map(|idx| format!("storager-{idx}")).collect()
    }

    #[test]
    fn computes_minimal_root_prefix_length() {
        assert_eq!(EPRing::minimal_prefix_len(1), 0);
        assert_eq!(EPRing::minimal_prefix_len(2), 1);
        assert_eq!(EPRing::minimal_prefix_len(16), 1);
        assert_eq!(EPRing::minimal_prefix_len(17), 2);
    }

    #[test]
    fn routes_root_prefixes_cyclically() {
        let ring = EPRing::new(&nodes(3), 10);
        assert_eq!(ring.root_prefix_len(), 1);

        let route_0 = ring.route_key_hex("0abc").expect("route 0");
        let route_1 = ring.route_key_hex("1abc").expect("route 1");
        let route_2 = ring.route_key_hex("2abc").expect("route 2");
        let route_3 = ring.route_key_hex("3abc").expect("route 3");

        assert_eq!(ring.node_name(route_0.node_index), Some("storager-0"));
        assert_eq!(ring.node_name(route_1.node_index), Some("storager-1"));
        assert_eq!(ring.node_name(route_2.node_index), Some("storager-2"));
        assert_eq!(ring.node_name(route_3.node_index), Some("storager-0"));
    }

    #[test]
    fn split_keeps_pre0_on_original_owner_and_rotates_rest() {
        let mut ring = EPRing::new(&nodes(3), 1);
        ring.record_insert("a", vec![1]);
        ring.record_insert("a", vec![2]);

        let parent_index = ring.find_entry_index("a").expect("parent entry");
        assert!(ring.entry_is_split(parent_index));

        let route_a0 = ring.route_key_hex("a0ff").expect("a0 route");
        let route_a1 = ring.route_key_hex("a1ff").expect("a1 route");
        let route_af = ring.route_key_hex("afff").expect("af route");

        assert_eq!(route_a0.prefix, "a0");
        assert_eq!(route_a1.prefix, "a1");
        assert_eq!(ring.node_name(route_a0.node_index), Some("storager-1"));
        assert_eq!(ring.node_name(route_a1.node_index), Some("storager-2"));
        assert_eq!(ring.node_name(route_af.node_index), Some("storager-1"));
    }

    #[test]
    fn hashes_keywords_before_prefix_routing() {
        assert_eq!(
            EPRing::keyword_to_hex("alpha"),
            "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8"
        );
    }
}
