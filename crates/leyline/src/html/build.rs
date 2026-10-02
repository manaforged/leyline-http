use super::entity::decode;
use super::scan::{Scanner, Tag};
use super::{Button, Form, FormMethod};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Value,
    Toggle,
    Submit,
    Image,
    Ignored,
}

const INPUT_TYPES: [(&str, Control); 7] = [
    ("checkbox", Control::Toggle),
    ("radio", Control::Toggle),
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
    name: Option<String>,
    multiple: bool,
    first: Option<String>,
    chosen: Vec<String>,
}

#[derive(Default)]
struct Builder {
    done: Vec<Form>,
    form: Option<Form>,
    select: Option<Select>,
}

pub(super) fn forms(document: &str) -> Vec<Form> {
    let mut builder = Builder::default();
    for tag in Scanner::new(document) {
        builder.tag(&tag);
    }
    builder.close_form();
    builder.done
}

impl Builder {
    fn tag(&mut self, tag: &Tag<'_>) {
        match (tag.closing, tag.name.as_str()) {
            (false, "form") if self.form.is_none() => self.form = Some(open(tag)),
            (true, "form") => self.close_form(),
            _ if self.form.is_some() => self.control(tag),
            _ => {}
        }
    }

    fn control(&mut self, tag: &Tag<'_>) {
        match (tag.closing, tag.name.as_str()) {
            (false, "input") => self.input(tag),
            (false, "button") => self.button(tag),
            (false, "textarea") => self.textarea(tag),
            (false, "select") => {
                self.close_select();
                self.select = Some(select(tag));
            }
            (false, "option") => self.option(tag),
            (true, "select") => self.close_select(),
            _ => {}
        }
    }

    fn push(&mut self, name: String, value: String) {
        if let Some(form) = self.form.as_mut() {
            form.fields.push((name, value));
        }
    }

    fn input(&mut self, tag: &Tag<'_>) {
        let Some(name) = field_name(tag) else {
            return;
        };
        let value = tag.attr("value");
        match kind(tag, &INPUT_TYPES, Control::Value) {
            Control::Value => self.push(name, value.unwrap_or_default().to_owned()),
            Control::Toggle if tag.has("checked") => {
                self.push(name, value.unwrap_or(TOGGLE_DEFAULT).to_owned());
            }
            Control::Submit => self.add_button(name, value, false),
            Control::Image => self.add_button(name, value, true),
            Control::Toggle | Control::Ignored => {}
        }
    }

    fn button(&mut self, tag: &Tag<'_>) {
        let Some(name) = field_name(tag) else {
            return;
        };
        if kind(tag, &BUTTON_TYPES, Control::Submit) == Control::Submit {
            self.add_button(name, tag.attr("value"), false);
        }
    }

    fn add_button(&mut self, name: String, value: Option<&str>, image: bool) {
        if let Some(form) = self.form.as_mut() {
            form.buttons.push(Button {
                name,
                value: value.unwrap_or_default().to_owned(),
                image,
            });
        }
    }

    fn textarea(&mut self, tag: &Tag<'_>) {
        let Some(name) = field_name(tag) else {
            return;
        };
        let raw = tag.raw.unwrap_or_default();
        let raw = raw
            .strip_prefix("\r\n")
            .or_else(|| raw.strip_prefix('\n'))
            .unwrap_or(raw);
        self.push(name, decode(raw));
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
        let Some(name) = select.name else {
            return;
        };
        let values = match (select.multiple, select.chosen.is_empty()) {
            (true, _) => select.chosen,
            (false, false) => select.chosen.last().cloned().into_iter().collect(),
            (false, true) => select.first.into_iter().collect(),
        };
        for value in values {
            self.push(name.clone(), value);
        }
    }

    fn close_form(&mut self) {
        self.close_select();
        if let Some(form) = self.form.take() {
            self.done.push(form);
        }
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
        id: tag.attr("id").map(str::to_owned),
        name: tag.attr("name").map(str::to_owned),
        fields: Vec::new(),
        buttons: Vec::new(),
    }
}

fn select(tag: &Tag<'_>) -> Select {
    Select {
        name: field_name(tag),
        multiple: tag.has("multiple"),
        first: None,
        chosen: Vec::new(),
    }
}

fn field_name(tag: &Tag<'_>) -> Option<String> {
    if tag.has("disabled") {
        return None;
    }
    tag.attr("name")
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
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
