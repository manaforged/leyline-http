use super::assign::{Entry, Item, Owner, assign};
use super::entity::decode;
use super::fieldset::Fieldsets;
use super::scan::{Scanner, Tag};
use super::{Button, Form, FormEnctype, FormMethod};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Value,
    Checkbox,
    Radio,
    Submit,
    Image,
    Ignored,
}

const INPUT_TYPES: [(&str, Control); 7] = [
    ("checkbox", Control::Checkbox),
    ("radio", Control::Radio),
    ("submit", Control::Submit),
    ("image", Control::Image),
    ("button", Control::Ignored),
    ("reset", Control::Ignored),
    ("file", Control::Ignored),
];

const BUTTON_TYPES: [(&str, Control); 2] =
    [("button", Control::Ignored), ("reset", Control::Ignored)];

const TOGGLE_DEFAULT: &str = "on";

struct Select {
    field: Option<(Owner, String)>,
    multiple: bool,
    first: Option<String>,
    chosen: Vec<String>,
}

#[derive(Default)]
struct Builder {
    forms: Vec<Form>,
    open: Option<usize>,
    entries: Vec<Entry>,
    select: Option<Select>,
    fieldsets: Fieldsets,
    base: Option<String>,
}

pub(super) fn forms(document: &str) -> Vec<Form> {
    let mut builder = Builder::default();
    for tag in Scanner::new(document) {
        builder.tag(&tag);
    }
    builder.finish()
}

impl Builder {
    fn tag(&mut self, tag: &Tag<'_>) {
        match (tag.closing, tag.name.as_str()) {
            (false, "form") if self.open.is_none() => {
                self.close_select();
                self.forms.push(open(tag));
                self.open = Some(self.forms.len() - 1);
            }
            (true, "form") => self.close_form(),
            (false, "base") if self.base.is_none() => {
                self.base = tag.attr("href").map(|href| href.trim().to_owned());
            }
            _ => {
                self.fieldsets.tag(tag);
                self.control(tag);
            }
        }
    }

    fn control(&mut self, tag: &Tag<'_>) {
        match (tag.closing, tag.name.as_str()) {
            (false, "input") => self.input(tag),
            (false, "button") => self.button(tag),
            (false, "textarea") => self.textarea(tag),
            (false, "select") => {
                self.close_select();
                self.select = Some(Select {
                    field: self.field(tag),
                    multiple: tag.has("multiple"),
                    first: None,
                    chosen: Vec::new(),
                });
            }
            (false, "option") => self.option(tag),
            (true, "select") => self.close_select(),
            _ => {}
        }
    }

    fn field(&self, tag: &Tag<'_>) -> Option<(Owner, String)> {
        if tag.has("disabled") || self.fieldsets.disabled() {
            return None;
        }
        let name = tag.attr("name").filter(|name| !name.is_empty())?;
        let owner = match tag.attr("form") {
            Some(id) => Owner::Id(id.to_owned()),
            None => Owner::Open(self.open?),
        };
        Some((owner, name.to_owned()))
    }

    fn push(&mut self, owner: Owner, name: String, value: String, radio: bool) {
        self.entries.push(Entry {
            owner,
            item: Item::Field { name, value, radio },
        });
    }

    fn input(&mut self, tag: &Tag<'_>) {
        let Some((owner, name)) = self.field(tag) else {
            return;
        };
        let value = tag.attr("value");
        let toggled = value.unwrap_or(TOGGLE_DEFAULT).to_owned();
        match kind(tag, &INPUT_TYPES, Control::Value) {
            Control::Value => self.push(owner, name, value.unwrap_or_default().to_owned(), false),
            Control::Checkbox if tag.has("checked") => self.push(owner, name, toggled, false),
            Control::Radio if tag.has("checked") => self.push(owner, name, toggled, true),
            Control::Submit => self.add_button(owner, name, value, false),
            Control::Image => self.add_button(owner, name, value, true),
            Control::Checkbox | Control::Radio | Control::Ignored => {}
        }
    }

    fn button(&mut self, tag: &Tag<'_>) {
        let Some((owner, name)) = self.field(tag) else {
            return;
        };
        if kind(tag, &BUTTON_TYPES, Control::Submit) == Control::Submit {
            self.add_button(owner, name, tag.attr("value"), false);
        }
    }

    fn add_button(&mut self, owner: Owner, name: String, value: Option<&str>, image: bool) {
        self.entries.push(Entry {
            owner,
            item: Item::Button(Button {
                name,
                value: value.unwrap_or_default().to_owned(),
                image,
            }),
        });
    }

    fn textarea(&mut self, tag: &Tag<'_>) {
        let Some((owner, name)) = self.field(tag) else {
            return;
        };
        let raw = tag.raw.unwrap_or_default();
        let raw = raw
            .strip_prefix("\r\n")
            .or_else(|| raw.strip_prefix('\n'))
            .unwrap_or(raw);
        self.push(owner, name, decode(raw), false);
    }

    fn option(&mut self, tag: &Tag<'_>) {
        let Some(select) = self.select.as_mut() else {
            return;
        };
        if tag.has("disabled") {
            return;
        }
        let value = match tag.attr("value") {
            Some(value) => value.to_owned(),
            None => collapse(&decode(tag.text)),
        };
        if select.first.is_none() {
            select.first = Some(value.clone());
        }
        if tag.has("selected") {
            select.chosen.push(value);
        }
    }

    fn close_select(&mut self) {
        let Some(select) = self.select.take() else {
            return;
        };
        let Some((owner, name)) = select.field else {
            return;
        };
        let values = match (select.multiple, select.chosen.is_empty()) {
            (true, _) => select.chosen,
            (false, false) => select.chosen.last().cloned().into_iter().collect(),
            (false, true) => select.first.into_iter().collect(),
        };
        for value in values {
            self.push(owner.clone(), name.clone(), value, false);
        }
    }

    fn close_form(&mut self) {
        self.close_select();
        self.open = None;
    }

    fn finish(mut self) -> Vec<Form> {
        self.close_form();
        assign(&mut self.forms, self.entries);
        for form in &mut self.forms {
            form.base.clone_from(&self.base);
        }
        self.forms
    }
}

fn open(tag: &Tag<'_>) -> Form {
    let method = match tag.attr("method") {
        Some(method) if method.trim().eq_ignore_ascii_case("post") => FormMethod::Post,
        _ => FormMethod::Get,
    };
    Form {
        action: tag.attr("action").unwrap_or_default().trim().to_owned(),
        method,
        enctype: tag
            .attr("enctype")
            .map(FormEnctype::parse)
            .unwrap_or_default(),
        base: None,
        id: tag.attr("id").map(str::to_owned),
        name: tag.attr("name").map(str::to_owned),
        fields: Vec::new(),
        buttons: Vec::new(),
    }
}

fn kind(tag: &Tag<'_>, table: &[(&str, Control)], default: Control) -> Control {
    let Some(kind) = tag.attr("type").map(str::trim) else {
        return default;
    };
    table
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(kind))
        .map_or(default, |(_, control)| *control)
}

pub(super) fn collapse(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}
