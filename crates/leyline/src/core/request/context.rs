use super::RequestBuilder;
use crate::core::into_url::IntoUrl;

impl RequestBuilder {
    /// Sets the page that started this request, which sets the site and referrer context.
    pub fn from_page(mut self, page: impl IntoUrl) -> Self {
        match page.into_url_with_base(self.session.base_url()) {
            Ok(page) => self.initiator = Some(page),
            Err(err) => self.fail(err),
        }
        self
    }

    /// Sets a label that names this request in traces.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Makes `send` fail with the status and body when the response status is an error.
    pub fn error_for_status(mut self) -> Self {
        self.status_errors = true;
        self
    }
}
