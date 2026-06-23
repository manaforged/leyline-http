//! Caller → preset header interleave logic.
//!
//! Separated from `execute.rs` so the redirect / cookie / retry code
//! doesn't have to carry the anchor-resolution details inline.

use std::borrow::Cow;

use crate::profile::preset::HeaderPair;
use crate::profile::{infer_anchor, HeaderAnchor};

use crate::core::headers::HeaderList;

/// Merge caller-supplied headers into the preset-built list.
///
/// Three rules, applied in order:
/// 1. A name the preset already emits is replaced in place, preserving
///    the preset's position. Prevents wire-coalesced duplicates like
///    `user-agent: a,b` (HTTP/1.1 §3.2.2).
/// 2. A caller-anchored header (`.anchored(anchor, ...)`) splices
///    immediately after (or before, for `BeforeAcceptEncoding`) its
///    anchor header.
/// 3. A plain caller header whose name has a universal Chrome slot
///    per `infer_anchor` rides at the inferred anchor; otherwise it
///    appends at the end.
///
/// `sensitive` is passed as a closure so the caller owns both the
/// sensitive-name set and the cross-origin strip decision.
pub(crate) fn apply_extra_headers(
    headers: &mut Vec<HeaderPair>,
    extra: &HeaderList,
    strip_sensitive: bool,
    sensitive: &dyn Fn(&str) -> bool,
) {
    // Snapshot the filtered entries — we iterate them twice below.
    let kept: Vec<&crate::core::headers::HeaderEntry> = extra
        .entries()
        .filter(|e| !(strip_sensitive && sensitive(&e.name)))
        .collect();

    // Pass 1 — in-place replacement for preset-owned names. For each
    // unique plain (non-anchored) name the caller supplies, if the
    // preset already emits it, remove the preset entry and splice in
    // every caller value with that name at the preset position —
    // preserving caller order for `.header(k, v1).append_header(k,
    // v2)`-style duplicates. Anchored entries never participate in
    // Pass 1; they always splice via Pass 2 so a caller can emit a
    // second `origin` via `.anchored(..., "origin", v)` without
    // losing the preset one.
    let mut consumed_names: Vec<String> = Vec::new();
    for (i, entry) in kept.iter().enumerate() {
        if entry.anchor.is_some() {
            continue;
        }
        let lower = entry.name.to_ascii_lowercase();
        if consumed_names.contains(&lower) {
            continue;
        }
        // Only the first occurrence of each name triggers the scan.
        // Check the remaining entries up to and including this one
        // were not already grouped; the `contains` check above does
        // the dedup, so we just skip non-first siblings.
        let _ = i;
        let values_in_order: Vec<HeaderPair> = kept
            .iter()
            .filter(|e| e.anchor.is_none() && e.name.eq_ignore_ascii_case(&lower))
            .map(|e| (Cow::Owned(e.name.clone()), Cow::Owned(e.value.clone())))
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

    // Pass 2 — interleave new caller headers at their anchor slots.
    //
    // We compute every target insertion index against the current
    // headers list first, then apply the inserts right-to-left so
    // earlier indices aren't shifted by later ones. Caller order
    // within a single anchor is preserved by inserting entries at
    // the same target index in caller-reversed order but advancing
    // the offset — see the loop body below.
    let mut insertions: Vec<(usize, Vec<HeaderPair>)> = Vec::new();
    for entry in &kept {
        if entry.anchor.is_none() && consumed_names.contains(&entry.name.to_ascii_lowercase()) {
            continue;
        }
        let anchor = entry.anchor.or_else(|| infer_anchor(&entry.name));
        let target_idx = anchor_target_index(headers, anchor);
        let item = (
            Cow::Owned(entry.name.clone()),
            Cow::Owned(entry.value.clone()),
        );
        if let Some((_, bucket)) = insertions.iter_mut().find(|(i, _)| *i == target_idx) {
            bucket.push(item);
        } else {
            insertions.push((target_idx, vec![item]));
        }
    }

    // Highest target index first so every remaining insert operates
    // on indices that are still valid in the original list.
    insertions.sort_by_key(|b| std::cmp::Reverse(b.0));
    for (idx, bucket) in insertions {
        for (offset, item) in bucket.into_iter().enumerate() {
            let insert_at = (idx + offset).min(headers.len());
            headers.insert(insert_at, item);
        }
    }
}

/// Compute the target insertion index for a header at the given
/// anchor. Missing anchors and absent target headers both fall back
/// to `headers.len()` (append at end).
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
    if anchor.is_before() {
        idx
    } else {
        idx + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset_xhr() -> Vec<HeaderPair> {
        vec![
            ("sec-ch-ua".into(), "chrome".into()),
            ("sec-ch-ua-mobile".into(), "?0".into()),
            ("sec-ch-ua-platform".into(), "\"Windows\"".into()),
            ("user-agent".into(), "Mozilla/5.0".into()),
            ("accept".into(), "application/json".into()),
            ("sec-fetch-site".into(), "same-origin".into()),
            ("sec-fetch-mode".into(), "cors".into()),
            ("sec-fetch-dest".into(), "empty".into()),
            ("accept-encoding".into(), "gzip".into()),
            ("accept-language".into(), "en-US".into()),
        ]
    }

    fn never_sensitive(_: &str) -> bool {
        false
    }

    #[test]
    fn plain_unknown_header_appends_at_end() {
        let mut headers = preset_xhr();
        let mut extra = HeaderList::new();
        extra.set("x-custom", "val");
        apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
        assert_eq!(headers.last().unwrap().0, "x-custom");
    }

    #[test]
    fn plain_known_header_rides_inferred_anchor() {
        // `authorization` has an inferred AfterUserAgent anchor.
        let mut headers = preset_xhr();
        let mut extra = HeaderList::new();
        extra.set("authorization", "Bearer t");
        apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
        let ua = headers.iter().position(|(k, _)| k == "user-agent").unwrap();
        assert_eq!(headers[ua + 1].0, "authorization");
    }

    #[test]
    fn anchored_header_splices_after_anchor() {
        let mut headers = preset_xhr();
        let mut extra = HeaderList::new();
        extra.append_anchored(HeaderAnchor::AfterUserAgent, "x-extra-6", "c");
        apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
        let ua = headers.iter().position(|(k, _)| k == "user-agent").unwrap();
        assert_eq!(headers[ua + 1].0, "x-extra-6");
    }

    #[test]
    fn multiple_anchored_same_anchor_preserve_caller_order() {
        let mut headers = preset_xhr();
        let mut extra = HeaderList::new();
        extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-a0", "0");
        extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-b", "b");
        extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-a", "a");
        apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
        let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_ref()).collect();
        let mobile = names.iter().position(|&n| n == "sec-ch-ua-mobile").unwrap();
        assert_eq!(&names[mobile + 1..mobile + 4], &["x-a0", "x-b", "x-a"]);
    }

    #[test]
    fn preset_owned_name_replaced_in_place() {
        let mut headers = preset_xhr();
        let original_ua_idx = headers.iter().position(|(k, _)| k == "user-agent").unwrap();
        let mut extra = HeaderList::new();
        extra.set("user-agent", "custom-agent");
        apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
        assert_eq!(headers[original_ua_idx].0, "user-agent");
        assert_eq!(headers[original_ua_idx].1, "custom-agent");
    }

    #[test]
    fn anchored_headers_interleave_at_each_anchor() {
        // Seven caller headers at five anchors.
        //
        let mut headers = vec![
            ("sec-ch-ua".into(), "chrome".into()),
            ("sec-ch-ua-mobile".into(), "?0".into()),
            ("sec-ch-ua-platform".into(), "\"Windows\"".into()),
            ("user-agent".into(), "Mozilla/5.0".into()),
            ("accept".into(), "*/*".into()),
            ("content-type".into(), "application/json".into()),
            ("origin".into(), "https://api.example.com".into()),
            ("accept-encoding".into(), "gzip".into()),
        ];
        let mut extra = HeaderList::new();
        extra.append_anchored(HeaderAnchor::AfterCchUaPlatform, "x-extra-5", "z");
        extra.append_anchored(HeaderAnchor::AfterCchUa, "x-extra-1", "f");
        extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-extra-2", "a0");
        extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-extra-3", "b");
        extra.append_anchored(HeaderAnchor::AfterCchUaMobile, "x-extra-4", "a");
        extra.append_anchored(HeaderAnchor::AfterUserAgent, "x-extra-6", "c");
        extra.append_anchored(HeaderAnchor::AfterContentType, "x-extra-7", "d");
        apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
        let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_ref()).collect();
        assert_eq!(
            names,
            vec![
                "sec-ch-ua",
                "x-extra-1",
                "sec-ch-ua-mobile",
                "x-extra-2",
                "x-extra-3",
                "x-extra-4",
                "sec-ch-ua-platform",
                "x-extra-5",
                "user-agent",
                "x-extra-6",
                "accept",
                "content-type",
                "x-extra-7",
                "origin",
                "accept-encoding",
            ]
        );
    }

    #[test]
    fn before_anchor_inserts_before_target() {
        let mut headers = preset_xhr();
        let mut extra = HeaderList::new();
        extra.append_anchored(HeaderAnchor::BeforeAcceptEncoding, "x-last-chance", "v");
        apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
        let ae_idx = headers
            .iter()
            .position(|(k, _)| k == "accept-encoding")
            .unwrap();
        assert_eq!(headers[ae_idx - 1].0, "x-last-chance");
    }

    #[test]
    fn sensitive_stripped_on_cross_origin_redirect() {
        let mut headers = preset_xhr();
        let mut extra = HeaderList::new();
        extra.set("authorization", "Bearer t");
        extra.set("x-benign", "ok");
        let sensitive = |name: &str| name.eq_ignore_ascii_case("authorization");
        apply_extra_headers(&mut headers, &extra, true, &sensitive);
        assert!(!headers.iter().any(|(k, _)| k == "authorization"));
        assert!(headers.iter().any(|(k, _)| k == "x-benign"));
    }

    #[test]
    fn anchor_absent_falls_back_to_end() {
        // Navigate preset has no content-type; AfterContentType
        // anchor falls back to appending at the end.
        let mut headers = vec![
            ("sec-ch-ua".into(), "chrome".into()),
            ("user-agent".into(), "Mozilla/5.0".into()),
            ("accept".into(), "text/html".into()),
            ("accept-encoding".into(), "gzip".into()),
        ];
        let mut extra = HeaderList::new();
        extra.append_anchored(HeaderAnchor::AfterContentType, "x-d", "d");
        apply_extra_headers(&mut headers, &extra, false, &never_sensitive);
        assert_eq!(headers.last().unwrap().0, "x-d");
    }
}
