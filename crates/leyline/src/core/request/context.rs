use super::RequestBuilder;
use crate::cookie::Jar;
use crate::core::Result;
use crate::core::into_url::IntoUrl;

impl RequestBuilder {
    pub fn cookie_jar(mut self, jar: Jar) -> Self {
        self.session = self.session.with_cookie_jar(jar);
        self
    }

    pub fn initiator(mut self, page: impl IntoUrl) -> Self {
        match page.into_url_with_base(self.session.base_url()) {
            Ok(page) => self.initiator = Some(page),
            Err(err) => self.fail(err),
        }
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    pub fn error_for_status(mut self) -> Self {
        self.status_errors = true;
        self
    }

    pub async fn download(
        self,
        path: impl AsRef<std::path::Path>,
        limit: Option<u64>,
    ) -> Result<u64> {
        let deadline = self.session.deadline(self.timeouts.as_ref());
        self.stream()
            .send()
            .await?
            .error_for_status_with_body(&deadline)
            .await?
            .download_to(path, limit)
            .await
    }
}
