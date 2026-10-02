mod build;
mod entity;
mod page;
mod scan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FormMethod {
    #[default]
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Form {
    action: String,
    method: FormMethod,
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
        match self.fields.iter_mut().find(|(key, _)| *key == name) {
            Some(field) => field.1 = value,
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
