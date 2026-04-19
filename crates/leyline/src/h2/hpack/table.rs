//! HPACK header table — static (RFC 7541 Appendix A) + dynamic.

use std::collections::VecDeque;

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
pub struct DynamicTable {
    entries: VecDeque<(String, String)>,
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
    pub fn insert(&mut self, name: String, value: String) {
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

    /// Get an entry by dynamic index (0 = newest).
    pub fn get(&self, index: usize) -> Option<(&str, &str)> {
        self.entries
            .get(index)
            .map(|(n, v)| (n.as_str(), v.as_str()))
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
pub fn lookup(index: usize, dynamic: &DynamicTable) -> Option<(&str, &str)> {
    if index < STATIC_TABLE.len() {
        let (name, value) = STATIC_TABLE[index];
        Some((name, value))
    } else {
        let dyn_index = index - STATIC_TABLE.len();
        dynamic.get(dyn_index)
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
mod tests {
    use super::*;

    #[test]
    fn static_table_size() {
        assert_eq!(STATIC_TABLE.len(), 62); // 0-61
    }

    #[test]
    fn static_table_common_entries() {
        assert_eq!(STATIC_TABLE[2], (":method", "GET"));
        assert_eq!(STATIC_TABLE[3], (":method", "POST"));
        assert_eq!(STATIC_TABLE[4], (":path", "/"));
        assert_eq!(STATIC_TABLE[7], (":scheme", "https"));
        assert_eq!(STATIC_TABLE[8], (":status", "200"));
    }

    #[test]
    fn dynamic_table_insert_and_get() {
        let mut dt = DynamicTable::new();
        dt.insert("custom-header".into(), "value1".into());
        assert_eq!(dt.get(0), Some(("custom-header", "value1")));
        assert_eq!(dt.len(), 1);
    }

    #[test]
    fn dynamic_table_eviction() {
        // "aa" + "bb" + 32 = 36 bytes per entry. Max 70 = room for 1, not 2.
        let mut dt = DynamicTable::with_max_size(70);
        dt.insert("aa".into(), "bb".into()); // 36 bytes
        assert_eq!(dt.len(), 1);

        dt.insert("cc".into(), "dd".into()); // 36 bytes, total would be 72 > 70, evicts first
        assert_eq!(dt.len(), 1);
        assert_eq!(dt.get(0), Some(("cc", "dd")));
    }

    #[test]
    fn dynamic_table_oversized_entry_clears() {
        let mut dt = DynamicTable::with_max_size(32); // too small for any entry
        dt.insert("x".into(), "y".into()); // 1+1+32 = 34, exceeds 32
        assert_eq!(dt.len(), 0);
    }

    #[test]
    fn lookup_static_and_dynamic() {
        let mut dt = DynamicTable::new();
        dt.insert("x-custom".into(), "val".into());

        // Static lookup.
        assert_eq!(lookup(2, &dt), Some((":method", "GET")));
        // Dynamic lookup (index 62 = first dynamic entry).
        assert_eq!(lookup(62, &dt), Some(("x-custom", "val")));
        // Out of range.
        assert_eq!(lookup(63, &dt), None);
    }

    #[test]
    fn find_static_exact() {
        assert_eq!(find_static(":method", "GET"), Some((2, true)));
        assert_eq!(find_static(":method", "POST"), Some((3, true)));
    }

    #[test]
    fn find_static_name_only() {
        assert_eq!(find_static(":method", "DELETE"), Some((2, false)));
        assert_eq!(
            find_static("content-type", "application/json"),
            Some((31, false))
        );
    }

    #[test]
    fn find_static_not_found() {
        assert_eq!(find_static("x-custom-header", ""), None);
    }
}
