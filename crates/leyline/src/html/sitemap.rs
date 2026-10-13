use super::entity::decode;
use super::scan::Scanner;
use super::{WEB_SCHEMES, lookup};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Loc,
    Set,
    Index,
}

const FIELDS: [(&str, Field); 3] = [
    ("loc", Field::Loc),
    ("urlset", Field::Set),
    ("sitemapindex", Field::Index),
];

impl Field {
    fn of(name: &str) -> Option<(&str, Field)> {
        let (prefix, local) = name.rsplit_once(':').unwrap_or(("", name));
        Some((prefix, lookup(&FIELDS, local)?))
    }
}

#[derive(Debug, Default)]
pub(crate) struct Sitemap {
    pub(crate) index: bool,
    pub(crate) locs: Vec<String>,
}

pub(super) fn read(document: &str) -> Sitemap {
    if !document.contains('<') {
        return plain(document);
    }
    let mut sitemap = Sitemap::default();
    let mut root: Option<String> = None;
    for tag in Scanner::xml(document).filter(|tag| !tag.closing) {
        let Some((prefix, field)) = Field::of(&tag.name) else {
            continue;
        };
        match field {
            Field::Set | Field::Index => {
                sitemap.index |= field == Field::Index;
                root.get_or_insert_with(|| prefix.to_owned());
            }
            Field::Loc if root.as_deref().unwrap_or_default() == prefix => {
                let loc = decode(tag.text.trim());
                if !loc.is_empty() {
                    sitemap.locs.push(loc);
                }
            }
            Field::Loc => {}
        }
    }
    sitemap
}

fn plain(document: &str) -> Sitemap {
    let locs = document
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.split_once("://")
                .is_some_and(|(scheme, _)| WEB_SCHEMES.contains(&scheme))
        })
        .map(str::to_owned)
        .collect();
    Sitemap { index: false, locs }
}
