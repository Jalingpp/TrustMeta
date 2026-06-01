use xxhash_rust::xxh3::xxh3_128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EPRingEntry {
    pub prefix: String,
    pub prefix_len: u8,
    pub counter: u64,
    /// Leaf entry: storager node index.
    /// Split entry: array index of the first child entry in `entries`.
    pub owner_ref: usize,
    pub split_child_start: Option<usize>,
    pub root_summary: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EPRingRoute {
    pub entry_index: usize,
    pub node_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EPRingSplitEvent {
    pub parent_entry_index: usize,
    pub original_owner_index: usize,
    pub child_routes: Vec<EPRingRoute>,
}

#[derive(Debug, Clone)]
pub struct EPRing {
    entries: Vec<EPRingEntry>,
    node_names: Vec<String>,
    root_prefix_len: usize,
    split_threshold: u64,
}

impl EPRing {
    const MAX_PREFIX_LEN: usize = 32;

    pub fn new(node_names: &[String], split_threshold: u64) -> Self {
        if node_names.is_empty() {
            return Self {
                entries: Vec::new(),
                node_names: Vec::new(),
                root_prefix_len: 0,
                split_threshold,
            };
        }

        let root_prefix_len = Self::minimal_prefix_len(node_names.len());
        let root_entry_count = 16usize.pow(root_prefix_len as u32);
        let mut entries = Vec::with_capacity(root_entry_count);

        for idx in 0..root_entry_count {
            entries.push(EPRingEntry {
                prefix: Self::format_prefix(idx, root_prefix_len),
                prefix_len: root_prefix_len as u8,
                counter: 0,
                owner_ref: idx % node_names.len(),
                split_child_start: None,
                root_summary: Vec::new(),
            });
        }

        Self {
            entries,
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

    fn hex_nibble(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => 0,
        }
    }

    fn digest_from_keyword(keyword: &str) -> [u8; 16] {
        xxh3_128(keyword.as_bytes()).to_le_bytes()
    }

    fn digest_nibble_at(digest: &[u8; 16], index: usize) -> u8 {
        let byte = digest.get(index / 2).copied().unwrap_or_default();
        if index % 2 == 0 {
            byte >> 4
        } else {
            byte & 0x0f
        }
    }

    fn root_entry_index_from_prefix(prefix: &str, len: usize) -> Option<usize> {
        if len == 0 {
            return Some(0);
        }

        let bytes = prefix.as_bytes();
        if bytes.len() < len {
            return None;
        }

        let mut index = 0usize;
        for byte in bytes.iter().take(len.min(Self::MAX_PREFIX_LEN)) {
            index = (index << 4) | Self::hex_nibble(*byte) as usize;
        }
        Some(index)
    }

    pub fn keyword_to_hex(keyword: &str) -> String {
        let digest = Self::digest_from_keyword(keyword);
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            out.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
            out.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
        }
        out
    }

    pub fn keyword_matches_prefix(keyword: &str, prefix: &str) -> bool {
        let digest = Self::digest_from_keyword(keyword);
        let capped_len = prefix.len().min(Self::MAX_PREFIX_LEN);
        for (idx, byte) in prefix.as_bytes().iter().take(capped_len).enumerate() {
            if Self::hex_nibble(*byte) != Self::digest_nibble_at(&digest, idx) {
                return false;
            }
        }
        true
    }

    fn entry_is_split(&self, entry_index: usize) -> bool {
        self.entries
            .get(entry_index)
            .and_then(|entry| entry.split_child_start)
            .is_some()
    }

    fn split_entry(&mut self, entry_index: usize) -> Option<EPRingSplitEvent> {
        if self.node_names.is_empty() || self.entry_is_split(entry_index) {
            return None;
        }

        let parent_prefix = self.entries.get(entry_index)?.prefix.clone();
        let parent_prefix_len = self.entries.get(entry_index)?.prefix_len as usize;
        if parent_prefix_len >= Self::MAX_PREFIX_LEN {
            return None;
        }

        let original_owner = self.entries[entry_index].owner_ref;
        let child_start = self.entries.len();
        let mut child_routes = Vec::with_capacity(16);

        for digit in 0..16usize {
            let child_index = child_start + digit;
            let node_index = (original_owner + digit) % self.node_names.len();
            let mut child_prefix = String::with_capacity(parent_prefix.len() + 1);
            child_prefix.push_str(&parent_prefix);
            child_prefix.push(char::from(b"0123456789abcdef"[digit]));
            self.entries.push(EPRingEntry {
                prefix: child_prefix,
                prefix_len: (parent_prefix_len + 1) as u8,
                counter: 0,
                owner_ref: node_index,
                split_child_start: None,
                root_summary: Vec::new(),
            });
            child_routes.push(EPRingRoute {
                entry_index: child_index,
                node_index,
            });
        }

        self.entries[entry_index].owner_ref = child_start;
        self.entries[entry_index].split_child_start = Some(child_start);
        Some(EPRingSplitEvent {
            parent_entry_index: entry_index,
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
        let key_hex = Self::keyword_to_hex(keyword);
        self.route_key_hex(&key_hex)
    }

    pub fn route_key_hex(&self, key_hex: &str) -> Option<EPRingRoute> {
        if self.entries.is_empty() {
            return None;
        }

        let capped_len = key_hex.len().min(Self::MAX_PREFIX_LEN);
        if capped_len < self.root_prefix_len {
            return None;
        }

        let mut entry_index = Self::root_entry_index_from_prefix(key_hex, self.root_prefix_len)?;
        let bytes = key_hex.as_bytes();

        loop {
            let entry = self.entries.get(entry_index)?;
            let Some(child_start) = entry.split_child_start else {
                return Some(EPRingRoute {
                    entry_index,
                    node_index: entry.owner_ref,
                });
            };

            let child_digit = Self::hex_nibble(bytes[entry.prefix_len as usize]) as usize;
            entry_index = child_start + child_digit;
        }
    }

    pub fn entry_prefix(&self, entry_index: usize) -> Option<&str> {
        self.entries
            .get(entry_index)
            .map(|entry| entry.prefix.as_str())
    }

    pub fn node_name(&self, node_index: usize) -> Option<&str> {
        self.node_names.get(node_index).map(|name| name.as_str())
    }

    pub fn find_entry_index(&self, prefix: &str) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }

        let capped_len = prefix.len().min(Self::MAX_PREFIX_LEN);
        if capped_len < self.root_prefix_len {
            return None;
        }

        let mut entry_index = Self::root_entry_index_from_prefix(prefix, self.root_prefix_len)?;
        for depth in self.root_prefix_len..capped_len {
            let entry = self.entries.get(entry_index)?;
            let Some(child_start) = entry.split_child_start else {
                return None;
            };
            let nibble = Self::hex_nibble(prefix.as_bytes()[depth]) as usize;
            entry_index = child_start + nibble;
        }

        Some(entry_index)
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

        assert_eq!(ring.entry_prefix(route_a0.entry_index), Some("a0"));
        assert_eq!(ring.entry_prefix(route_a1.entry_index), Some("a1"));
        assert_eq!(ring.node_name(route_a0.node_index), Some("storager-1"));
        assert_eq!(ring.node_name(route_a1.node_index), Some("storager-2"));
        assert_eq!(ring.node_name(route_af.node_index), Some("storager-1"));
    }

    #[test]
    fn hashes_keywords_before_prefix_routing() {
        let digest = EPRing::keyword_to_hex("alpha");
        assert_eq!(digest.len(), 32);
        assert_eq!(digest, EPRing::keyword_to_hex("alpha"));
    }
}
