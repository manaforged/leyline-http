use std::fmt::Write as _;

use crate::BuildResult;

pub(crate) fn render(source: &str) -> BuildResult<String> {
    let mut rows = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let (name, codes) = line
            .split_once('\t')
            .ok_or_else(|| format!("entities.tsv:{}: expected name<TAB>codepoints", index + 1))?;
        let value = codes
            .split(' ')
            .map(|code| {
                u32::from_str_radix(code, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| format!("entities.tsv:{}: bad codepoint {code:?}", index + 1))
            })
            .collect::<Result<String, String>>()?;
        rows.push((name.to_owned(), value));
    }
    if !rows.windows(2).all(|pair| pair[0].0 < pair[1].0) {
        return Err("entities.tsv must be sorted by name with no duplicates".into());
    }
    let mut out = format!("static ENTITIES: [(&str, &str); {}] = [\n", rows.len());
    for (name, value) in &rows {
        writeln!(out, "    ({name:?}, {value:?}),")?;
    }
    out.push_str("];\n");
    Ok(out)
}
