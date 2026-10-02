use url::Url;

use crate::core::error::{Error, Kind, Result};

mod sealed {
    pub trait Sealed {}
}

pub trait IntoUrl: sealed::Sealed {
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
    match (Url::parse(raw), base) {
        (Err(url::ParseError::RelativeUrlWithoutBase), Some(base))
            if !raw.chars().any(|c| c.is_ascii_control()) =>
        {
            base.join(raw).map_err(Error::from_url_parse)
        }
        _ => raw.into_url(),
    }
}

impl sealed::Sealed for &str {}
impl IntoUrl for &str {
    fn into_url(self) -> Result<Url> {
        if self.chars().any(|c| c.is_ascii_control()) {
            return Err(Error::new(Kind::Url).with_message("URL contains a control character"));
        }
        Url::parse(self).map_err(Error::from_url_parse)
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
