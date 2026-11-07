use std::cell::RefCell;
use std::sync::Arc;

use url::Url;

use crate::error::{Error, Kind, Result};

pub(crate) mod sealed {
    use std::sync::Arc;

    use url::Url;

    use crate::error::Result;

    pub trait Sealed {
        fn shared_url(self, base: Option<&Url>) -> Result<Arc<Url>>
        where
            Self: super::IntoUrl + Sized,
        {
            self.into_url_with_base(base).map(Arc::new)
        }
    }
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

thread_local! {
    static LAST: RefCell<Option<Arc<Url>>> = const { RefCell::new(None) };
}

fn parse_with_base(raw: &str, base: Option<&Url>) -> Result<Url> {
    shared_with_base(raw, base).map(Arc::unwrap_or_clone)
}

fn shared_with_base(raw: &str, base: Option<&Url>) -> Result<Arc<Url>> {
    if let Some(url) =
        LAST.with_borrow(|last| last.as_ref().filter(|url| url.as_str() == raw).cloned())
    {
        return Ok(url);
    }
    if raw.chars().any(|c| c.is_ascii_control()) {
        return Err(Error::new(Kind::Url).with_message("URL contains a control character"));
    }
    let parsed = Url::parse(raw);
    if let Ok(url) = &parsed
        && url.as_str() == raw
    {
        let shared = Arc::new(url.clone());
        LAST.set(Some(Arc::clone(&shared)));
        return Ok(shared);
    }
    match (parsed, base) {
        (Err(url::ParseError::RelativeUrlWithoutBase), Some(base)) => {
            base.join(raw).map(Arc::new).map_err(Error::from)
        }
        (parsed, _) => parsed.map(Arc::new).map_err(Error::from),
    }
}

impl sealed::Sealed for &str {
    fn shared_url(self, base: Option<&Url>) -> Result<Arc<Url>> {
        shared_with_base(self, base)
    }
}
impl IntoUrl for &str {
    fn into_url(self) -> Result<Url> {
        parse_with_base(self, None)
    }

    fn into_url_with_base(self, base: Option<&Url>) -> Result<Url> {
        parse_with_base(self, base)
    }
}

impl sealed::Sealed for String {
    fn shared_url(self, base: Option<&Url>) -> Result<Arc<Url>> {
        shared_with_base(&self, base)
    }
}
impl IntoUrl for String {
    fn into_url(self) -> Result<Url> {
        self.as_str().into_url()
    }

    fn into_url_with_base(self, base: Option<&Url>) -> Result<Url> {
        parse_with_base(&self, base)
    }
}

impl sealed::Sealed for &String {
    fn shared_url(self, base: Option<&Url>) -> Result<Arc<Url>> {
        shared_with_base(self, base)
    }
}
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
