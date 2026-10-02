mod assign;
mod build;
mod entity;
mod fieldset;
mod page;
mod scan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FormMethod {
    #[default]
    Get,
    Post,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FormEnctype {
    #[default]
    UrlEncoded,
    Multipart,
    TextPlain,
}

impl FormEnctype {
    const ALL: [FormEnctype; 3] = [Self::UrlEncoded, Self::Multipart, Self::TextPlain];

    #[must_use]
    pub fn as_str(self) -> &'static str {
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Anchor {
    href: String,
    text: String,
    rel: Option<String>,
}

#[must_use]
pub fn meta(document: &str, name: &str) -> Option<String> {
    page::meta(document, name)
}

#[must_use]
pub fn links(document: &str) -> Vec<Anchor> {
    page::links(document)
}

impl Anchor {
    #[must_use]
    pub fn href(&self) -> &str {
        &self.href
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn rel(&self) -> Option<&str> {
        self.rel.as_deref()
    }
}

#[must_use]
pub fn forms(document: &str) -> Vec<Form> {
    build::forms(document)
}

impl Form {
    #[must_use]
    pub fn find(document: &str, selector: &str) -> Option<Form> {
        forms(document).into_iter().find(|form| {
            form.id.as_deref() == Some(selector) || form.name.as_deref() == Some(selector)
        })
    }

    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    #[must_use]
    pub fn method(&self) -> FormMethod {
        self.method
    }

    #[must_use]
    pub fn enctype(&self) -> FormEnctype {
        self.enctype
    }

    #[must_use]
    pub fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }

    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }

    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

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

    #[must_use]
    pub fn buttons(&self) -> Vec<&str> {
        self.buttons.iter().map(|b| b.name.as_str()).collect()
    }

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
