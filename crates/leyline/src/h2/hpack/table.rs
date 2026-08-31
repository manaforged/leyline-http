//! HPACK header table — static (RFC 7541 Appendix A) + dynamic.

use std::collections::VecDeque;

use bytes::Bytes;

/// Static table: 61 pre-defined header entries (RFC 7541 Appendix A).
pub static STATIC_TABLE: &[(&str, &str)] = &[
    ("", ""),
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

/// Dynamic table — FIFO with bounded size (RFC 7541 Section 2.3.2).
pub struct DynamicTable {
    entries: VecDeque<(Bytes, Bytes)>,
    /// Current size in bytes (name.len() + value.len() + 32 per entry).
    size: usize,
    /// Maximum size (set by SETTINGS_HEADER_TABLE_SIZE).
    max_size: usize,
}

impl DynamicTable {
    /// Create with default max size (4096 bytes).
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            size: 0,
            max_size: 4096,
        }
    }

    /// Create with a specific max size.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            size: 0,
            max_size,
        }
    }

    /// Update the max size (from SETTINGS).
    pub fn set_max_size(&mut self, max_size: usize) {
        self.max_size = max_size;
        self.evict();
    }

    /// Insert a new entry at the front.
    pub fn insert(&mut self, name: Bytes, value: Bytes) {
        let entry_size = name.len() + value.len() + 32;

        if entry_size > self.max_size {
            self.entries.clear();
            self.size = 0;
            return;
        }

        while self.size + entry_size > self.max_size {
            if let Some(old) = self.entries.pop_back() {
                self.size -= old.0.len() + old.1.len() + 32;
            } else {
                break;
            }
        }

        self.entries.push_front((name, value));
        self.size += entry_size;
    }

    /// Get an entry by dynamic index (0 = newest).
    pub fn get(&self, index: usize) -> Option<(&Bytes, &Bytes)> {
        self.entries.get(index).map(|(n, v)| (n, v))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Current size in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    fn evict(&mut self) {
        while self.size > self.max_size {
            if let Some(old) = self.entries.pop_back() {
                self.size -= old.0.len() + old.1.len() + 32;
            } else {
                break;
            }
        }
    }
}

impl Default for DynamicTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Look up a header by index across static + dynamic tables.
pub fn lookup(index: usize, dynamic: &DynamicTable) -> Option<(Bytes, Bytes)> {
    if index < STATIC_TABLE.len() {
        let (name, value) = STATIC_TABLE[index];
        Some((
            Bytes::from_static(name.as_bytes()),
            Bytes::from_static(value.as_bytes()),
        ))
    } else {
        let dyn_index = index - STATIC_TABLE.len();
        dynamic.get(dyn_index).map(|(n, v)| (n.clone(), v.clone()))
    }
}

/// Find the index for a header name+value in static table.
pub fn find_static(name: &str, value: &str) -> Option<(usize, bool)> {
    let mut name_match = None;
    for (i, &(n, v)) in STATIC_TABLE.iter().enumerate().skip(1) {
        if n == name {
            if v == value {
                return Some((i, true));
            }
            if name_match.is_none() {
                name_match = Some(i);
            }
        }
    }
    name_match.map(|i| (i, false))
}

#[cfg(test)]
mod tests;
