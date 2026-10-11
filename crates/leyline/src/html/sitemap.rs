use super::entity::decode;
use super::scan::Scanner;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Loc,
    Index,
}

const FIELDS: [(&str, Field); 2] = [("loc", Field::Loc), ("sitemapindex", Field::Index)];

impl Field {
    fn of(name: &str) -> Option<Field> {
        FIELDS
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, field)| *field)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Sitemap {
    pub(crate) index: bool,
    pub(crate) locs: Vec<String>,
}

pub(super) fn read(document: &str) -> Sitemap {
    let mut sitemap = Sitemap::default();
    for tag in Scanner::new(document).filter(|tag| !tag.closing) {
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
