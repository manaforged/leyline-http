use std::collections::{BTreeMap, BTreeSet};

use crate::BuildResult;
use crate::browser::Row;

pub(crate) fn assign(rows: &mut [Row], ids: &BTreeMap<String, u32>) -> BuildResult<()> {
    let mut seen = BTreeSet::new();
    for row in rows.iter_mut() {
        let id = *ids.get(&row.meta.variant).ok_or_else(|| {
            format!(
                "browser_ids.toml has no id for {}; add it with the next free number",
                row.meta.variant
            )
        })?;
        if !seen.insert(id) {
            return Err(format!("browser_ids.toml gives id {id} to more than one variant").into());
        }
        row.id = id;
    }
    let expected: BTreeSet<u32> = (0..u32::try_from(rows.len())?).collect();
    if seen != expected {
        return Err("browser_ids.toml ids must run from 0 with no gaps".into());
    }
    if ids.len() != rows.len() {
        return Err("browser_ids.toml names a variant that has no profile".into());
    }
    Ok(())
}

pub(crate) fn by_id(rows: &[Row]) -> Vec<&Row> {
    let mut ordered: Vec<&Row> = rows.iter().collect();
    ordered.sort_by_key(|row| row.id);
    ordered
}
