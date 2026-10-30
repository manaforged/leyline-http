mod assign;
mod build;
mod directives;
mod entity;
mod fieldset;
mod page;
mod scan;
mod sitemap;

pub(crate) use sitemap::Sitemap;

use crate::core::lookup;

pub(crate) const WEB_SCHEMES: [&str; 2] = ["http", "https"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub(crate) enum FormMethod {
    #[default]
    Get,
    Post,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub(crate) enum FormEnctype {
    #[default]
    UrlEncoded,
    Multipart,
    TextPlain,
}

impl FormEnctype {
    const ALL: [FormEnctype; 3] = [Self::UrlEncoded, Self::Multipart, Self::TextPlain];

    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::UrlEncoded => "application/x-www-form-urlencoded",
            Self::Multipart => "multipart/form-data",
            Self::TextPlain => "text/plain",
        }
    }

    fn parse(value: &str) -> Self {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|enctype| enctype.as_str().eq_ignore_ascii_case(value))
            .unwrap_or_default()
    }
}

/// An HTML form read from a page, with its action, method, encoding, fields, and submit buttons.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Form {
    action: String,
    method: FormMethod,
    enctype: FormEnctype,
    base: Option<String>,
    id: Option<String>,
    name: Option<String>,
    fields: Vec<(String, String)>,
    buttons: Vec<Button>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Button {
    name: String,
    value: String,
    image: bool,
}

/// A link read from an `<a>` element, with its href, text, and `rel` value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Anchor {
    href: String,
    text: String,
    rel: Option<String>,
}

/// Returns the content of the `<meta>` tag named `name`, or `None` when the page has none.
#[must_use]
pub fn meta(document: &str, name: &str) -> Option<String> {
    page::meta(document, name)
}

/// Returns every link in the document.
#[must_use]
pub fn links(document: &str) -> Vec<Anchor> {
    page::links(document)
}

/// Returns the readable text of the document.
#[must_use]
pub fn text(document: &str) -> String {
    page::text(document)
}

pub(crate) fn base(document: &str) -> Option<String> {
    page::base(document)
}

/// Returns the page's canonical URL, or `None` when the page declares none.
#[must_use]
pub fn canonical(document: &str) -> Option<String> {
    page::canonical(document)
}

pub(crate) fn sitemap(document: &str) -> Sitemap {
    sitemap::read(document)
}

/// The robots directives for a page, read from its meta tags and `X-Robots-Tag` headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Directives {
    index: bool,
    follow: bool,
    ai: bool,
}

impl Directives {
    /// Reads the directives that apply to `agent` from the document and response headers.
    #[must_use]
    pub fn read(document: &str, headers: &http::HeaderMap, agent: &str) -> Directives {
        directives::read(document, headers, agent)
    }

    /// Returns whether the page may be indexed.
    #[must_use]
    pub fn index(&self) -> bool {
        self.index
    }

    /// Returns whether the page's links may be followed.
    #[must_use]
    pub fn follow(&self) -> bool {
        self.follow
    }

    /// Returns whether the page may be used by AI systems.
    #[must_use]
    pub fn ai(&self) -> bool {
        self.ai
    }
}

impl Anchor {
    /// Returns the link's `href` value as written in the page.
    #[must_use]
    pub fn href(&self) -> &str {
        &self.href
    }

    /// Returns the link's text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the link's `rel` attribute, or `None` when it has none.
    #[must_use]
    pub fn rel(&self) -> Option<&str> {
        self.rel.as_deref()
    }
}

/// Returns every form in the document.
#[must_use]
pub fn forms(document: &str) -> Vec<Form> {
    build::forms(document)
}

impl Form {
    /// Returns the first form whose `id` or `name` equals `name`, or `None` when no form matches.
    #[must_use]
    pub fn find(document: &str, name: &str) -> Option<Form> {
        forms(document)
            .into_iter()
            .find(|form| form.id.as_deref() == Some(name) || form.name.as_deref() == Some(name))
    }

    /// Returns the form action URL.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Returns the form method, `GET` or `POST`.
    #[must_use]
    pub fn method(&self) -> http::Method {
        match self.method {
            FormMethod::Get => http::Method::GET,
            FormMethod::Post => http::Method::POST,
        }
    }

    /// Returns the form encoding as a MIME type, such as `multipart/form-data`.
    #[must_use]
    pub fn enctype(&self) -> &'static str {
        self.enctype.as_str()
    }

    pub(crate) fn encoding(&self) -> FormEnctype {
        self.enctype
    }

    /// Returns the page's base URL for resolving the action, or `None` when the page has none.
    #[must_use]
    pub fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }

    /// Returns the form's `id` attribute, or `None` when it has none.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Returns the form's `name` attribute, or `None` when it has none.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the form's fields as name and value pairs, in document order.
    #[must_use]
    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }

    /// Returns the value of the first field named `name`, or `None` when there is none.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Sets field `name` to `value`, replacing the first match and removing later duplicates, or appends it.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) -> &mut Self {
        let name = name.into();
        let value = value.into();
        match self.fields.iter().position(|(key, _)| *key == name) {
            Some(first) => {
                self.fields[first].1 = value;
                let mut index = 0;
                self.fields.retain(|(key, _)| {
                    let keep = index <= first || *key != name;
                    index += 1;
                    keep
                });
            }
            None => self.fields.push((name, value)),
        }
        self
    }

    /// Returns the names of the form's submit buttons.
    #[must_use]
    pub fn buttons(&self) -> Vec<&str> {
        self.buttons.iter().map(|b| b.name.as_str()).collect()
    }

    /// Adds the named submit button's value to the fields, or `.x` and `.y` for an image button; returns `false` if none matches.
    pub fn press(&mut self, button: &str) -> bool {
        let Some(found) = self.buttons.iter().find(|b| b.name == button).cloned() else {
            return false;
        };
        if found.image {
            self.fields
                .push((format!("{}.x", found.name), "0".to_owned()));
            self.fields
                .push((format!("{}.y", found.name), "0".to_owned()));
        } else {
            self.fields.push((found.name, found.value));
        }
        true
    }
}
