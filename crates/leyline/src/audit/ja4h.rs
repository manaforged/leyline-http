use crate::audit::hash12;

pub struct Ja4hInput<'a> {
    pub method: &'a str,
    pub http_version: &'a str,
    pub headers: &'a [(String, String)],
}

pub fn compute_ja4h(input: &Ja4hInput<'_>) -> String {
    let a = section_a(input);
    let b = section_b(input);
    let c = section_c(input);
    let d = section_d(input);
    format!("{a}_{b}_{c}_{d}")
}

fn section_a(input: &Ja4hInput<'_>) -> String {
    let method = &input.method.to_lowercase();
    let method2 = if method.len() >= 2 {
        &method[..2]
    } else {
        method.as_str()
    };

    let version = match input.http_version {
        "1.0" => "10",
        "1.1" => "11",
        "2" | "2.0" => "20",
        "3" | "3.0" => "30",
        _ => "00",
    };

    let has_cookie = input.headers.iter().any(|(n, _)| n == "cookie");
    let has_referer = input.headers.iter().any(|(n, _)| n == "referer");
    let cookie_flag = if has_cookie { "c" } else { "n" };
    let referer_flag = if has_referer { "r" } else { "n" };

    let count = input
        .headers
        .iter()
        .filter(|(n, _)| n != "cookie" && n != "referer" && !n.starts_with(':'))
        .count()
        .min(99);

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
