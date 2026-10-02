use std::collections::{HashMap, HashSet};

use super::{Button, Form};

#[derive(Clone)]
pub(super) enum Owner {
    Open(usize),
    Id(String),
}

pub(super) enum Item {
    Field {
        name: String,
        value: String,
        radio: bool,
    },
    Button(Button),
}

pub(super) struct Entry {
    pub(super) owner: Owner,
    pub(super) item: Item,
}

pub(super) fn assign(
    forms: &mut [Form],
    entries: Vec<Entry>,
    ids: &HashMap<String, Option<usize>>,
) {
    let mut fields: Vec<Vec<(String, String, bool)>> = vec![Vec::new(); forms.len()];
    for entry in entries {
        let index = match &entry.owner {
            Owner::Open(index) => Some(*index),
            Owner::Id(id) => ids.get(id).copied().flatten(),
        };
        let Some(index) = index.filter(|index| *index < forms.len()) else {
            continue;
        };
        match entry.item {
            Item::Field { name, value, radio } => fields[index].push((name, value, radio)),
            Item::Button(button) => forms[index].buttons.push(button),
        }
    }
    for (form, fields) in forms.iter_mut().zip(fields) {
        form.fields = last_radio(fields);
    }
}

fn last_radio(fields: Vec<(String, String, bool)>) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut kept: Vec<(String, String)> = fields
        .into_iter()
        .rev()
        .filter(|(name, _, radio)| !radio || seen.insert(name.clone()))
        .map(|(name, value, _)| (name, value))
        .collect();
    kept.reverse();
    kept
}
