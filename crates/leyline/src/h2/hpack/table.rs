//! HPACK header table — static (RFC 7541 Appendix A) + dynamic.

use std::collections::VecDeque;

use bytes::Bytes;

/// Static table: 61 pre-defined header entries (RFC 7541 Appendix A).
/// Index 1-61. Index 0 is unused.
pub static STATIC_TABLE: &[(&str, &str)] = &[
    ("", ""),                             // 0 (unused)
    (":authority", ""),                   // 1
    (":method", "GET"),                   // 2
    (":method", "POST"),                  // 3
    (":path", "/"),                       // 4
    (":path", "/index.html"),             // 5
    (":scheme", "http"),                  // 6
    (":scheme", "https"),                 // 7
    (":status", "200"),                   // 8
    (":status", "204"),                   // 9
    (":status", "206"),                   // 10
    (":status", "304"),                   // 11
    (":status", "400"),                   // 12
    (":status", "404"),                   // 13
    (":status", "500"),                   // 14
    ("accept-charset", ""),               // 15
    ("accept-encoding", "gzip, deflate"), // 16
    ("accept-language", ""),              // 17
    ("accept-ranges", ""),                // 18
    ("accept", ""),                       // 19
    ("access-control-allow-origin", ""),  // 20
    ("age", ""),                          // 21
    ("allow", ""),                        // 22
    ("authorization", ""),                // 23
    ("cache-control", ""),                // 24
    ("content-disposition", ""),          // 25
    ("content-encoding", ""),             // 26
    ("content-language", ""),             // 27
    ("content-length", ""),               // 28
    ("content-location", ""),             // 29
    ("content-range", ""),                // 30
    ("content-type", ""),                 // 31
    ("cookie", ""),                       // 32
    ("date", ""),                         // 33
    ("etag", ""),                         // 34
    ("expect", ""),                       // 35
    ("expires", ""),                      // 36
    ("from", ""),                         // 37
    ("host", ""),                         // 38
    ("if-match", ""),                     // 39
    ("if-modified-since", ""),            // 40
    ("if-none-match", ""),                // 41
    ("if-range", ""),                     // 42
    ("if-unmodified-since", ""),          // 43
    ("last-modified", ""),                // 44
    ("link", ""),                         // 45
    ("location", ""),                     // 46
    ("max-forwards", ""),                 // 47
    ("proxy-authenticate", ""),           // 48
    ("proxy-authorization", ""),          // 49
    ("range", ""),                        // 50
    ("referer", ""),                      // 51
    ("refresh", ""),                      // 52
    ("retry-after", ""),                  // 53
    ("server", ""),                       // 54
    ("set-cookie", ""),                   // 55
    ("strict-transport-security", ""),    // 56
    ("transfer-encoding", ""),            // 57
    ("user-agent", ""),                   // 58
    ("vary", ""),                         // 59
    ("via", ""),                          // 60
    ("www-authenticate", ""),             // 61
];

/// Dynamic table — FIFO with bounded size (RFC 7541 Section 2.3.2).
///
/// Entries are indexed starting at STATIC_TABLE.len() (62).
/// Newest entries have the lowest dynamic index.
///
/// Entries are stored as `Bytes` so a table hit materializes a decoded
/// header by a refcount clone — no heap copy of the name/value.
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

    /// Update the max size (from SETTINGS). Evicts entries if needed.
    pub fn set_max_size(&mut self, max_size: usize) {
        self.max_size = max_size;
        self.evict();
    }

    /// Insert a new entry at the front. Evicts old entries if needed.
    pub fn insert(&mut self, name: Bytes, value: Bytes) {
        let entry_size = name.len() + value.len() + 32;

        // If the entry is larger than the table, clear everything.
        if entry_size > self.max_size {
            self.entries.clear();
            self.size = 0;
            return;
        }

        // Evict until there's room.
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

    /// Get an entry by dynamic index (0 = newest). Borrows the stored
    /// `Bytes` so a comparison scan does not refcount-churn; callers that
    /// materialize clone explicitly (a refcount bump, no heap alloc).
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
/// Static: 1-61. Dynamic: 62+.
///
/// Returns owned `Bytes`: a static entry borrows its `'static` literal via
/// `Bytes::from_static` (no allocation), a dynamic entry refcount-clones the
/// stored `Bytes` (no allocation). Either way the indexed-header decode path
/// materializes a header without a heap copy.
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
/// Returns `Some((index, exact_match))` — exact_match is true if both name and value matched.
pub fn find_static(name: &str, value: &str) -> Option<(usize, bool)> {
    let mut name_match = None;
    for (i, &(n, v)) in STATIC_TABLE.iter().enumerate().skip(1) {
        if n == name {
            if v == value {
                return Some((i, true)); // exact match
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
