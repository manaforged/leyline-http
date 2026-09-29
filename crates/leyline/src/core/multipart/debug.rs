use std::fmt;

use super::{Form, Part};
use crate::trace::masked;

impl fmt::Debug for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Part")
            .field("name", &self.name)
            .field("filename", &self.filename)
            .field("mime", &self.mime)
            .field("body", &self.body)
            .field(
                "headers",
                &masked(
                    self.extra_headers
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_bytes())),
                ),
            )
            .finish()
    }
}

impl fmt::Debug for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Form")
            .field("boundary", &self.boundary)
            .field("parts", &self.parts)
            .finish()
    }
}
