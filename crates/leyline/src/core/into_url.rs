use url::Url;

use crate::core::error::{Error, Result};

mod sealed {
    pub trait Sealed {}
}

pub trait IntoUrl: sealed::Sealed {
    fn into_url(self) -> Result<Url>;
}

impl sealed::Sealed for &str {}
impl IntoUrl for &str {
    fn into_url(self) -> Result<Url> {
        Url::parse(self).map_err(Error::from_url_parse)
    }
}

impl sealed::Sealed for String {}
impl IntoUrl for String {
    fn into_url(self) -> Result<Url> {
        self.as_str().into_url()
    }
}

impl sealed::Sealed for &String {}
impl IntoUrl for &String {
    fn into_url(self) -> Result<Url> {
        self.as_str().into_url()
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
