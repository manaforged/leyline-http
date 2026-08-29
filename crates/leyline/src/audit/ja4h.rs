//! JA4H HTTP request fingerprint (FoxIO specification).
//!
//! Format: `{section_a}_{section_b}_{section_c}_{section_d}`
//! Computed from the HTTP request headers.

use crate::audit::hash12;

/// Input data for JA4H computation.
pub struct Ja4hInput<'a> {
    /// HTTP method (e.g. "GET", "POST").
    pub method: &'a str,
    /// HTTP version: "1.0", "1.1", "2", "3".
    pub http_version: &'a str,
    /// Request headers as (name, value) pairs in order.
    /// Names should be lowercase.
    pub headers: &'a [(String, String)],
}

/// Compute JA4H fingerprint.
pub fn compute_ja4h(input: &Ja4hInput<'_>) -> String {
    let a = section_a(input);
    let b = section_b(input);
    let c = section_c(input);
    let d = section_d(input);
    format!("{a}_{b}_{c}_{d}")
}

/// Section A: request metadata.
/// Format: {method:2}{version:2}{cookie}{referer}{header_count:02}{lang:4}
fn section_a(input: &Ja4hInput<'_>) -> String {
    // Method: first 2 chars, lowercase.
    let method = &input.method.to_lowercase();
    let method2 = if method.len() >= 2 {
        &method[..2]
    } else {
        method.as_str()
    };

    // Version.
    let version = match input.http_version {
        "1.0" => "10",
        "1.1" => "11",
        "2" | "2.0" => "20",
        "3" | "3.0" => "30",
        _ => "00",
    };

    // Cookie/Referer presence.
    let has_cookie = input.headers.iter().any(|(n, _)| n == "cookie");
    let has_referer = input.headers.iter().any(|(n, _)| n == "referer");
    let cookie_flag = if has_cookie { "c" } else { "n" };
    let referer_flag = if has_referer { "r" } else { "n" };

    // Header count excluding cookie, referer, and pseudo-headers.
    let count = input
        .headers
        .iter()
        .filter(|(n, _)| n != "cookie" && n != "referer" && !n.starts_with(':'))
        .count()
        .min(99);

    // Accept-Language: first tag, remove dashes, pad/truncate to 4 chars.
    let lang = input
        .headers
        .iter()
        .find(|(n, _)| n == "accept-language")
        .map(|(_, v)| {
            let first_tag = v.split(',').next().unwrap_or("");
            let first_tag = first_tag.split(';').next().unwrap_or("").trim();
            let cleaned: String = first_tag.chars().filter(|c| *c != '-').collect();
            let mut s = cleaned;
            while s.len() < 4 {
                s.push('0');
            }
            s[..4].to_string()
        })
        .unwrap_or_else(|| "0000".to_string());

    format!("{method2}{version}{cookie_flag}{referer_flag}{count:02}{lang}")
}

/// Section B: sorted header names hash.
fn section_b(input: &Ja4hInput<'_>) -> String {
    let mut names: Vec<&str> = input
        .headers
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| *n != "cookie" && *n != "referer" && !n.starts_with(':'))
        .collect();
    names.sort();
    let s = names.join(",");
    hash12(&s)
}

/// Section C: sorted cookie names hash.
fn section_c(input: &Ja4hInput<'_>) -> String {
    let cookie_val = input
        .headers
        .iter()
        .find(|(n, _)| n == "cookie")
        .map(|(_, v)| v.as_str());

    match cookie_val {
        Some(val) => {
            let mut names: Vec<&str> = val
                .split(';')
                .filter_map(|pair| {
                    let pair = pair.trim();
                    pair.split('=').next().map(|n| n.trim())
                })
                .filter(|n| !n.is_empty())
                .collect();
            names.sort();
            hash12(&names.join(","))
        }
        None => "000000000000".to_string(),
    }
}

/// Section D: sorted cookie name=value pairs hash.
fn section_d(input: &Ja4hInput<'_>) -> String {
    let cookie_val = input
        .headers
        .iter()
        .find(|(n, _)| n == "cookie")
        .map(|(_, v)| v.as_str());

    match cookie_val {
        Some(val) => {
            let mut pairs: Vec<&str> = val
                .split(';')
                .map(|p| p.trim())
                .filter(|p| !p.is_empty())
                .collect();
            pairs.sort_by_key(|p| p.split('=').next().unwrap_or(""));
            hash12(&pairs.join(","))
        }
        None => "000000000000".to_string(),
    }
}

#[cfg(test)]
mod tests;
