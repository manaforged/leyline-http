use std::fmt;

use crate::audit::hash12;
use crate::trace::masked;

pub struct Ja4hInput<'a> {
    pub method: &'a str,
    pub http_version: &'a str,
    pub headers: &'a [(String, String)],
}

impl fmt::Debug for Ja4hInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ja4hInput")
            .field("method", &self.method)
            .field("http_version", &self.http_version)
            .field(
                "headers",
                &masked(self.headers.iter().map(|(k, v)| (k.as_str(), v.as_bytes()))),
            )
            .finish()
    }
}

pub fn compute_ja4h(input: &Ja4hInput<'_>) -> String {
    let a = input.section_a();
    let b = input.section_b();
    let c = input.section_c();
    let d = input.section_d();
    format!("{a}_{b}_{c}_{d}")
}

impl Ja4hInput<'_> {
    fn section_a(&self) -> String {
        let method = &self.method.to_lowercase();
        let method2 = if method.len() >= 2 {
            &method[..2]
        } else {
            method.as_str()
        };

        let version = match self.http_version {
            "1.0" => "10",
            "1.1" => "11",
            "2" | "2.0" => "20",
            "3" | "3.0" => "30",
            _ => "00",
        };

        let has_cookie = self.headers.iter().any(|(n, _)| n == "cookie");
        let has_referer = self.headers.iter().any(|(n, _)| n == "referer");
        let cookie_flag = if has_cookie { "c" } else { "n" };
        let referer_flag = if has_referer { "r" } else { "n" };

        let count = self
            .headers
            .iter()
            .filter(|(n, _)| n != "cookie" && n != "referer" && !n.starts_with(':'))
            .count()
            .min(99);

        let lang = self
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

    fn section_b(&self) -> String {
        let mut names: Vec<&str> = self
            .headers
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| *n != "cookie" && *n != "referer" && !n.starts_with(':'))
            .collect();
        names.sort();
        let s = names.join(",");
        hash12(&s)
    }

    fn section_c(&self) -> String {
        let cookie_val = self
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

    fn section_d(&self) -> String {
        let cookie_val = self
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
}

#[cfg(test)]
mod tests;
