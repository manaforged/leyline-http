use std::sync::{Arc, Mutex, PoisonError};

use http::Method;
use url::Url;

use crate::core::error::{Error, Kind, Result};
use crate::core::into_url::IntoUrl;
use crate::core::request::{IntoParamPair, RequestBuilder};
use crate::core::response::Response;
use crate::core::session::Session;
use crate::profile::Preset;

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

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn current(&self) -> Option<Url> {
        self.page
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_current(&self, page: Option<Url>) {
        *self.page.lock().unwrap_or_else(PoisonError::into_inner) = page;
    }

    pub(crate) fn request(
        &self,
        method: Method,
        url: impl IntoUrl,
        preset: Preset,
    ) -> RequestBuilder {
        let page = self.current();
        let builder = self.build(method, url, page.as_ref()).preset(preset);
        match page {
            Some(page) => builder.initiator(page),
            None => builder,
        }
    }

    pub fn fetch(&self, method: Method, url: impl IntoUrl) -> RequestBuilder {
        self.script(method, url, Preset::Xhr)
    }

    pub fn xhr(&self, url: impl IntoUrl) -> RequestBuilder {
        self.fetch(Method::GET, url)
    }

    pub fn post_json(&self, url: impl IntoUrl, body: &impl serde::Serialize) -> RequestBuilder {
        self.fetch(Method::POST, url).json(body)
    }

    pub fn subresource(&self, url: impl IntoUrl, preset: Preset) -> RequestBuilder {
        self.script(Method::GET, url, preset)
    }

    fn script(&self, method: Method, url: impl IntoUrl, preset: Preset) -> RequestBuilder {
        let mut builder = self.request(method, url, preset);
        if self.current().is_none() {
            builder.fail(
                Error::new(Kind::Request).with_message("the tab has no page; open one first"),
            );
        }
        builder
    }

    pub async fn open(&self, url: impl IntoUrl) -> Result<Response> {
        let page = self.current();
        let builder = self
            .build(Method::GET, url, page.as_ref())
            .preset(Preset::Navigate);
        self.navigate(builder).await
    }

    pub async fn follow(&self, url: impl IntoUrl) -> Result<Response> {
        self.navigate(self.trusted(self.request(Method::GET, url, Preset::Navigate)))
            .await
    }

    pub async fn submit<I, P>(&self, url: impl IntoUrl, fields: I) -> Result<Response>
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        let builder = self
            .request(Method::POST, url, Preset::FormNavigate)
            .form(fields);
        self.navigate(builder).await
    }

    #[cfg(feature = "html")]
    pub async fn submit_form(&self, form: &crate::html::Form) -> Result<Response> {
        let mut url = self.form_action(form)?;
        let fields = normalized_fields(form);
        let builder = match form.method() {
            crate::html::FormMethod::Post => encode_form(
                self.request(Method::POST, url, Preset::FormNavigate),
                form.enctype(),
                fields,
            ),
            crate::html::FormMethod::Get => {
                url.query_pairs_mut().clear().extend_pairs(&fields);
                self.request(Method::GET, url, Preset::Navigate)
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
            |body, (name, value)| body.text(name, value),
        )),
    }
}
