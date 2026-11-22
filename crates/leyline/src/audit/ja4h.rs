use std::fmt;

use crate::audit::hash12;
use crate::util::masked;

pub struct Ja4hInput<'a> {
    pub method: &'a str,
    pub version: http::Version,
    pub headers: &'a [(String, String)],
}

impl fmt::Debug for Ja4hInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ja4hInput")
            .field("method", &self.method)
            .field("version", &self.version)
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
        let method2: String = self.method.to_lowercase().chars().take(2).collect();

        let version = match self.version {
            http::Version::HTTP_10 => "10",
            http::Version::HTTP_11 => "11",
            http::Version::HTTP_2 => "20",
            http::Version::HTTP_3 => "30",
            _ => "00",
        };

        let has_cookie = self
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("cookie"));
        let has_referer = self
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("referer"));
        let cookie_flag = if has_cookie { "c" } else { "n" };
        let referer_flag = if has_referer { "r" } else { "n" };

        let count = self
            .headers
            .iter()
            .filter(|(n, _)| {
                !n.eq_ignore_ascii_case("cookie")
                    && !n.eq_ignore_ascii_case("referer")
                    && !n.starts_with(':')
            })
            .count()
            .min(99);

        let lang = self
            .headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("accept-language"))
            .map(|(_, v)| {
                let first_tag = v.split(',').next().unwrap_or("");
                let first_tag = first_tag.split(';').next().unwrap_or("").trim();
                let mut s: String = first_tag.chars().filter(|c| *c != '-').take(4).collect();
                while s.chars().count() < 4 {
                    s.push('0');
                }
                s
            })
            .unwrap_or_else(|| "0000".to_string());

        format!("{method2}{version}{cookie_flag}{referer_flag}{count:02}{lang}")
    }

    fn section_b(&self) -> String {
        let mut names: Vec<&str> = self
            .headers
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| {
                !n.eq_ignore_ascii_case("cookie")
                    && !n.eq_ignore_ascii_case("referer")
                    && !n.starts_with(':')
            })
            .collect();
        names.sort();
        let s = names.join(",");
        hash12(&s)
    }

    fn cookie_values(&self) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("cookie"))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    fn section_c(&self) -> String {
        let cookies = self.cookie_values();
        if cookies.is_empty() {
            return hash12("");
        }
        let mut names: Vec<&str> = cookies
            .into_iter()
            .flat_map(|val| val.split(';'))
            .filter_map(|pair| {
                let pair = pair.trim();
                pair.split('=').next().map(|n| n.trim())
            })
            .filter(|n| !n.is_empty())
            .collect();
        names.sort();
        hash12(&names.join(","))
    }

    fn section_d(&self) -> String {
        let cookies = self.cookie_values();
        if cookies.is_empty() {
            return hash12("");
        }
        let mut pairs: Vec<&str> = cookies
            .into_iter()
            .flat_map(|val| val.split(';'))
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();
        pairs.sort_by_key(|p| p.split('=').next().unwrap_or(""));
        hash12(&pairs.join(","))
    }
}

#[cfg(test)]
mod tests;
