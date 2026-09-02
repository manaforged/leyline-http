//! Caller → preset header interleave logic.

use std::borrow::Cow;

use crate::profile::preset::HeaderPair;
use crate::profile::{HeaderAnchor, infer_anchor};

use crate::core::headers::HeaderList;

/// Merge caller-supplied headers into the preset-built list.
pub(crate) fn apply_extra_headers(
    headers: &mut Vec<HeaderPair>,
    extra: &HeaderList,
    strip_sensitive: bool,
    sensitive: &dyn Fn(&str) -> bool,
) {
    let kept: Vec<&crate::core::headers::HeaderEntry> = extra
        .entries()
        .filter(|e| !(strip_sensitive && sensitive(e.name.as_str())))
        .collect();

    let mut consumed_names: Vec<String> = Vec::new();
    for entry in &kept {
        if entry.anchor.is_some() {
            continue;
        }
        let lower = entry.name.as_str().to_string();
        if consumed_names.contains(&lower) {
            continue;
        }
        let values_in_order: Vec<HeaderPair> = kept
            .iter()
            .filter(|e| e.anchor.is_none() && e.name.as_str() == lower)
            .map(|e| (Cow::Owned(e.name.as_str().to_string()), text(&e.value)))
            .collect();
        if let Some(pos) = headers
            .iter()
            .position(|(k, _)| k.eq_ignore_ascii_case(&lower))
        {
            headers.remove(pos);
            for (offset, item) in values_in_order.into_iter().enumerate() {
                let insert_at = (pos + offset).min(headers.len());
                headers.insert(insert_at, item);
            }
            consumed_names.push(lower);
        }
    }

    let mut insertions: Vec<(usize, Vec<HeaderPair>)> = Vec::new();
    for entry in &kept {
        if entry.anchor.is_none() && consumed_names.contains(&entry.name.as_str().to_string()) {
            continue;
        }
        let anchor = entry.anchor.or_else(|| infer_anchor(entry.name.as_str()));
        let target_idx = anchor_target_index(headers, anchor);
        let item = (
            Cow::Owned(entry.name.as_str().to_string()),
            text(&entry.value),
        );
        if let Some((_, bucket)) = insertions.iter_mut().find(|(i, _)| *i == target_idx) {
            bucket.push(item);
        } else {
            insertions.push((target_idx, vec![item]));
        }
    }

    insertions.sort_by_key(|b| std::cmp::Reverse(b.0));
    for (idx, bucket) in insertions {
        for (offset, item) in bucket.into_iter().enumerate() {
            let insert_at = (idx + offset).min(headers.len());
            headers.insert(insert_at, item);
        }
    }
}

/// Header value as text; obs-text bytes become U+FFFD.
fn text(v: &http::HeaderValue) -> Cow<'static, str> {
    Cow::Owned(String::from_utf8_lossy(v.as_bytes()).into_owned())
}

/// Compute the target insertion index for a header at the given anchor.
fn anchor_target_index(headers: &[HeaderPair], anchor: Option<HeaderAnchor>) -> usize {
    let Some(anchor) = anchor else {
        return headers.len();
    };
    let Some(idx) = headers
        .iter()
        .position(|(k, _)| k.eq_ignore_ascii_case(anchor.anchor_name()))
    else {
        return headers.len();
    };
    if anchor.is_before() { idx } else { idx + 1 }
}

#[cfg(test)]
mod tests;
