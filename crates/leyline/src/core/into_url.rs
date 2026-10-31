use url::Url;

use crate::core::error::{Cause, Error, Result};

mod sealed {
    pub trait Sealed {}
}

/// Converts a value into a parsed `Url`.
pub trait IntoUrl: sealed::Sealed {
    /// Parses the value as a URL and fails when it is not a valid absolute URL.
    fn into_url(self) -> Result<Url>;

    #[doc(hidden)]
    fn into_url_with_base(self, base: Option<&Url>) -> Result<Url>
    where
        Self: Sized,
    {
        let _ = base;
        self.into_url()
    }
}

fn parse_with_base(raw: &str, base: Option<&Url>) -> Result<Url> {
    if raw.chars().any(|c| c.is_ascii_control()) {
        return Err(Error::new(Cause::Url).with_message("URL contains a control character"));
    }
    match (Url::parse(raw), base) {
        (Err(url::ParseError::RelativeUrlWithoutBase), Some(base)) => {
            base.join(raw).map_err(Error::from_url_parse)
        }
        (parsed, _) => parsed.map_err(Error::from_url_parse),
    }
}

impl sealed::Sealed for &str {}
impl IntoUrl for &str {
    fn into_url(self) -> Result<Url> {
        parse_with_base(self, None)
    }

    fn into_url_with_base(self, base: Option<&Url>) -> Result<Url> {
        parse_with_base(self, base)
    }
}

impl sealed::Sealed for String {}
impl IntoUrl for String {
    fn into_url(self) -> Result<Url> {
        self.as_str().into_url()
    }

    fn into_url_with_base(self, base: Option<&Url>) -> Result<Url> {
        parse_with_base(&self, base)
    }
}

impl sealed::Sealed for &String {}
impl IntoUrl for &String {
    fn into_url(self) -> Result<Url> {
        self.as_str().into_url()
    }

    fn into_url_with_base(self, base: Option<&Url>) -> Result<Url> {
        parse_with_base(self, base)
    }
}

impl sealed::Sealed for Url {}
impl IntoUrl for Url {
    fn into_url(self) -> Result<Url> {
        Ok(self)
    }
}

impl sealed::Sealed for &Url {}
impl IntoUrl for &Url {
    fn into_url(self) -> Result<Url> {
        Ok(self.clone())
    }
}
