use std::borrow::Cow;

use crate::util::sensitive_header;

pub(crate) fn masked<'a>(
    fields: impl Iterator<Item = (&'a str, &'a [u8])>,
) -> Vec<(&'a str, Cow<'a, str>)> {
    fields
        .map(|(name, value)| {
            let shown = if sensitive_header(name) {
                Cow::Borrowed("***")
            } else {
                String::from_utf8_lossy(value)
            };
            (name, shown)
        })
        .collect()
}
