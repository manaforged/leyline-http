use std::collections::{BTreeMap, BTreeSet};

use crate::BuildResult;
use crate::brand::Variant;
use crate::browser::Row;

pub(crate) fn assign(
    rows: &mut [Row],
    variants: &mut [Variant],
    ids: &BTreeMap<String, u32>,
) -> BuildResult<()> {
    let mut seen = BTreeSet::new();
    let names = rows
        .iter_mut()
        .map(|row| (row.meta.variant.clone(), &mut row.id))
        .chain(variants.iter_mut().map(|v| (v.name.clone(), &mut v.id)));
    for (name, slot) in names {
        let id = *ids.get(&name).ok_or_else(|| {
            format!("browser_ids.toml has no id for {name}; add it with the next free number")
        })?;
        if !seen.insert(id) {
            return Err(format!("browser_ids.toml gives id {id} to more than one variant").into());
        }
        *slot = id;
    }
    let total = rows.len() + variants.len();
    let expected: BTreeSet<u32> = (0..u32::try_from(total)?).collect();
    if seen != expected {
        return Err("browser_ids.toml ids must run from 0 with no gaps".into());
    }
    if ids.len() != total {
        return Err("browser_ids.toml names a variant that has no profile".into());
    }
    Ok(())
}
