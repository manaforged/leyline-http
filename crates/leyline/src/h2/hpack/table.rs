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

/// First index and entry count of a static-table name, whose entries are always contiguous (RFC 7541 Appendix A).
fn span(name: &str) -> Option<(usize, usize)> {
    let found = match name.len() {
        3 => match name {
            "age" => (21, 1),
            "via" => (60, 1),
            _ => return None,
        },
        4 => match name {
            "date" => (33, 1),
            "etag" => (34, 1),
            "from" => (37, 1),
            "host" => (38, 1),
            "link" => (45, 1),
            "vary" => (59, 1),
            _ => return None,
        },
        5 => match name {
            ":path" => (4, 2),
            "allow" => (22, 1),
            "range" => (50, 1),
            _ => return None,
        },
        6 => match name {
            "accept" => (19, 1),
            "cookie" => (32, 1),
            "expect" => (35, 1),
            "server" => (54, 1),
            _ => return None,
        },
        7 => match name {
            ":method" => (2, 2),
            ":scheme" => (6, 2),
            ":status" => (8, 7),
            "expires" => (36, 1),
            "referer" => (51, 1),
            "refresh" => (52, 1),
            _ => return None,
        },
        8 => match name {
            "if-match" => (39, 1),
            "if-range" => (42, 1),
            "location" => (46, 1),
            _ => return None,
        },
        10 => match name {
            ":authority" => (1, 1),
            "set-cookie" => (55, 1),
            "user-agent" => (58, 1),
            _ => return None,
        },
        11 => match name {
            "retry-after" => (53, 1),
            _ => return None,
        },
        12 => match name {
            "content-type" => (31, 1),
            "max-forwards" => (47, 1),
            _ => return None,
        },
        13 => match name {
            "accept-ranges" => (18, 1),
            "authorization" => (23, 1),
            "cache-control" => (24, 1),
            "content-range" => (30, 1),
            "if-none-match" => (41, 1),
            "last-modified" => (44, 1),
            _ => return None,
        },
        14 => match name {
            "accept-charset" => (15, 1),
            "content-length" => (28, 1),
            _ => return None,
        },
        15 => match name {
            "accept-encoding" => (16, 1),
            "accept-language" => (17, 1),
            _ => return None,
        },
        16 => match name {
            "content-encoding" => (26, 1),
            "content-language" => (27, 1),
            "content-location" => (29, 1),
            "www-authenticate" => (61, 1),
            _ => return None,
        },
        17 => match name {
            "if-modified-since" => (40, 1),
            "transfer-encoding" => (57, 1),
            _ => return None,
        },
        18 => match name {
            "proxy-authenticate" => (48, 1),
            _ => return None,
        },
        19 => match name {
            "content-disposition" => (25, 1),
            "if-unmodified-since" => (43, 1),
            "proxy-authorization" => (49, 1),
            _ => return None,
        },
        25 => match name {
            "strict-transport-security" => (56, 1),
            _ => return None,
        },
        27 => match name {
            "access-control-allow-origin" => (20, 1),
            _ => return None,
        },
        _ => return None,
    };
    Some(found)
}

/// Find the index for a header name+value in static table.
pub fn find_static(name: &str, value: &str) -> Option<(usize, bool)> {
    let (first, count) = span(name)?;
    for i in first..first + count {
        if let Some(&(_, v)) = STATIC_TABLE.get(i)
            && v == value
        {
            return Some((i, true));
        }
    }
    Some((first, false))
}

#[cfg(test)]
mod tests;
