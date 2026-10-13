use super::entity::decode;
use super::scan::Scanner;
use super::{WEB_SCHEMES, lookup};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Loc,
    Index,
}

const FIELDS: [(&str, Field); 2] = [("loc", Field::Loc), ("sitemapindex", Field::Index)];

impl Field {
    fn of(name: &str) -> Option<Field> {
        let local = name.rsplit(':').next().unwrap_or(name);
        lookup(&FIELDS, local)
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
    for tag in Scanner::xml(document).filter(|tag| !tag.closing) {
        match Field::of(&tag.name) {
            Some(Field::Index) => sitemap.index = true,
            Some(Field::Loc) => {
                let loc = decode(tag.text.trim());
                if !loc.is_empty() {
                    sitemap.locs.push(loc);
                }
            }
            None => {}
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
