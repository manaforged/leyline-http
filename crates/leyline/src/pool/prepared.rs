use http::Method;

use crate::error::BodyLimit;
use crate::net::ProxyUrl;
use crate::wire::{Body, RequestHeaders, ResponseMode};

pub(crate) struct Prepared<'a> {
    pub(crate) method: &'a Method,
    pub(crate) url: &'a url::Url,
    pub(crate) headers: RequestHeaders,
    pub(crate) body: Body,
    pub(crate) proxy: Option<&'a ProxyUrl>,
    pub(crate) response: ResponseMode,
    pub(crate) body_limit: BodyLimit,
    pub(crate) stale_resend: bool,
}

impl<'a> Prepared<'a> {
    pub(crate) fn again(&self, body: Body) -> Self {
        Prepared {
            method: self.method,
            url: self.url,
            headers: self.headers.clone(),
            body,
            proxy: self.proxy,
            response: self.response,
            body_limit: self.body_limit,
            stale_resend: self.stale_resend,
        }
    }
}
