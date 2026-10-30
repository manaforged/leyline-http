use std::sync::{Arc, Mutex, PoisonError};

use http::Method;
use url::Url;

use crate::core::error::{Cause, Error, Result};
use crate::core::into_url::IntoUrl;
use crate::core::request::{IntoParamPair, RequestBuilder};
use crate::core::response::Response;
use crate::core::session::Session;
use crate::profile::RequestKind;

#[doc = include_str!("../../docs/tab.md")]
#[derive(Clone)]
pub struct Tab {
    session: Session,
    page: Arc<Mutex<Option<Url>>>,
}

impl std::fmt::Debug for Tab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tab")
            .field("current", &self.current())
            .finish_non_exhaustive()
    }
}

impl Tab {
    pub(crate) fn new(session: Session) -> Self {
        Self {
            session,
            page: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the session the tab sends requests through.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Returns the URL of the page the tab has open.
    pub fn current(&self) -> Option<Url> {
        self.page
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Sets the page URL the tab treats as open, or clears it with `None`.
    pub fn set_current(&self, page: Option<Url>) {
        *self.page.lock().unwrap_or_else(PoisonError::into_inner) = page;
    }

    pub(crate) fn request(
        &self,
        method: Method,
        url: impl IntoUrl,
        preset: RequestKind,
    ) -> RequestBuilder {
        let page = self.current();
        let builder = self.build(method, url, page.as_ref()).kind(preset);
        match page {
            Some(page) => builder.from_page(page),
            None => builder,
        }
    }

    /// Returns a script request from the open page, which fails when the tab has no page.
    pub fn fetch(&self, method: Method, url: impl IntoUrl) -> RequestBuilder {
        self.script(method, url, RequestKind::Xhr)
    }

    fn script(&self, method: Method, url: impl IntoUrl, preset: RequestKind) -> RequestBuilder {
        let mut builder = self.request(method, url, preset);
        if self.current().is_none() {
            builder.fail(
                Error::new(Cause::Request).with_message("the tab has no page; open one first"),
            );
        }
        builder
    }

    /// Navigates the tab to `url` as if typed in the address bar.
    pub async fn open(&self, url: impl IntoUrl) -> Result<Response> {
        let page = self.current();
        let builder = self
            .build(Method::GET, url, page.as_ref())
            .kind(RequestKind::Navigate);
        self.navigate(builder).await
    }

    /// Navigates the tab to `url` as a link click from the open page.
    pub async fn follow(&self, url: impl IntoUrl) -> Result<Response> {
        self.navigate(self.trusted(self.request(Method::GET, url, RequestKind::Navigate)))
            .await
    }

    /// Posts `fields` as a URL-encoded form to `url` and navigates the tab to the response.
    pub async fn submit<I, P>(&self, url: impl IntoUrl, fields: I) -> Result<Response>
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        let builder = self
            .request(Method::POST, url, RequestKind::FormNavigate)
            .form(fields);
        self.navigate(builder).await
    }

    /// Submits a parsed HTML form with its method and encoding and navigates the tab to the response.
    #[cfg(feature = "html")]
    pub async fn submit_form(&self, form: &crate::html::Form) -> Result<Response> {
        let mut url = self.form_action(form)?;
        let fields = normalized_fields(form);
        let builder = match form.method() {
            Method::POST => encode_form(
                self.request(Method::POST, url, RequestKind::FormNavigate),
                form.encoding(),
                fields,
            ),
            _ => {
                url.query_pairs_mut().clear().extend_pairs(&fields);
                self.request(Method::GET, url, RequestKind::Navigate)
            }
        };
        self.navigate(self.trusted(builder)).await
    }

    #[cfg(feature = "html")]
    fn form_action(&self, form: &crate::html::Form) -> Result<Url> {
        let page = self.current().or_else(|| self.session.base_url().cloned());
        let base = form
            .base()
            .and_then(|href| href.into_url_with_base(page.as_ref()).ok())
            .or_else(|| page.clone());
        let against = if form.action().is_empty() {
            page.as_ref()
        } else {
            base.as_ref()
        };
        form.action().into_url_with_base(against)
    }

    pub(crate) async fn navigate(&self, builder: RequestBuilder) -> Result<Response> {
        let response = builder.send().await?;
        self.set_current(Some(response.url().clone()));
        Ok(response)
    }

    fn trusted(&self, mut builder: RequestBuilder) -> RequestBuilder {
        builder.trusted_origin = self.current();
        builder
    }

    fn build(&self, method: Method, url: impl IntoUrl, page: Option<&Url>) -> RequestBuilder {
        let Some(page) = page else {
            return self.session.request(method, url);
        };
        match url.into_url_with_base(Some(page)) {
            Ok(url) => self.session.request(method, url),
            Err(err) => {
                let mut builder = self.session.request(method, page.clone());
                builder.fail(err);
                builder
            }
        }
    }
}

#[cfg(feature = "html")]
fn normalized_fields(form: &crate::html::Form) -> Vec<(String, String)> {
    form.fields()
        .iter()
        .map(|(name, value)| (crlf(name), crlf(value)))
        .collect()
}

#[cfg(feature = "html")]
fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

#[cfg(feature = "html")]
fn encode_form(
    builder: RequestBuilder,
    enctype: crate::html::FormEnctype,
    fields: Vec<(String, String)>,
) -> RequestBuilder {
    use crate::html::FormEnctype;
    match enctype {
        FormEnctype::UrlEncoded => builder.form(fields),
        FormEnctype::TextPlain => {
            let body: String = fields
                .iter()
                .map(|(name, value)| format!("{name}={value}\r\n"))
                .collect();
            builder
                .header(http::header::CONTENT_TYPE, FormEnctype::TextPlain.as_str())
                .body(body)
        }
        FormEnctype::Multipart => builder.multipart(fields.into_iter().fold(
            crate::core::multipart::Form::new(),
            |body, (name, value)| body.part(name, crate::core::multipart::Part::text(value)),
        )),
    }
}
