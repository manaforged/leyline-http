use crate::core::config::Proxy;
use crate::error::{Kind, Result};
use crate::net::ProxyUrl;

pub(crate) const CGI_SIGNAL_ENV_VARS: &[&str] = &[
    "GATEWAY_INTERFACE",
    "REQUEST_METHOD",
    "SERVER_SOFTWARE",
    "SCRIPT_NAME",
    "SCRIPT_FILENAME",
    "PATH_INFO",
    "QUERY_STRING",
    "SERVER_PROTOCOL",
    "SERVER_NAME",
    "SERVER_PORT",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InvalidEnvProxy {
    FailBuild,
    FailRequests,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvProxy {
    pub(crate) variable: &'static str,
    pub(crate) value: String,
}

impl Proxy {
    pub(super) fn apply_env_proxy(self, on_invalid: InvalidEnvProxy) -> Result<Proxy> {
        self.apply_env_proxy_from(
            on_invalid,
            |k| std::env::var(k).ok(),
            |k| std::env::var_os(k).is_some(),
        )
    }

    pub(super) fn apply_env_proxy_from<F, G>(
        mut self,
        on_invalid: InvalidEnvProxy,
        get_var: F,
        has_var: G,
    ) -> Result<Proxy>
    where
        F: Fn(&str) -> Option<String>,
        G: Fn(&str) -> bool,
    {
        if !self.rules().is_empty() || !self.uses_env() {
            return Ok(self);
        }
        let in_cgi = detect_cgi(&has_var);
        let https = read_env_group(&HTTPS_PROXY_VARS, in_cgi, &get_var);
        let http = read_env_group(&HTTP_PROXY_VARS, in_cgi, &get_var);
        let all = read_env_group(&ALL_PROXY_VARS, in_cgi, &get_var);
        if https.is_none() && http.is_none() && all.is_none() {
            return Ok(self);
        }
        for entry in [&https, &http, &all].into_iter().flatten() {
            if ProxyUrl::parse(&entry.value).is_err() {
                let config = self.reject_env(entry.variable);
                return match (on_invalid, config.rejection(Kind::Config)) {
                    (InvalidEnvProxy::FailBuild, Some(error)) => Err(error),
                    _ => Ok(config),
                };
            }
        }
        if let Some(entry) = all {
            self = self.all(entry.value);
        }
        if let Some(entry) = http {
            self = self.http(entry.value);
        }
        if let Some(entry) = https {
            self = self.https(entry.value);
        }
        let no_proxy = get_var("NO_PROXY").or_else(|| get_var("no_proxy"));
        Ok(self.set_from_env().env_no_proxy(no_proxy.as_deref()))
    }
}

const HTTPS_PROXY_VARS: [&str; 2] = ["HTTPS_PROXY", "https_proxy"];
const HTTP_PROXY_VARS: [&str; 2] = ["HTTP_PROXY", "http_proxy"];
const ALL_PROXY_VARS: [&str; 2] = ["ALL_PROXY", "all_proxy"];

#[cfg(test)]
const ENV_PROXY_VARS: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

fn detect_cgi<G: Fn(&str) -> bool>(has_var: &G) -> bool {
    let in_cgi = CGI_SIGNAL_ENV_VARS.iter().any(|k| has_var(k));
    if in_cgi && has_var("HTTP_PROXY") {
        tracing::warn!(
            target: "leyline::env_proxy::cgi",
            "CGI environment detected — ignoring HTTP_PROXY (httpoxy mitigation)"
        );
    }
    in_cgi
}

fn read_env_group<F: Fn(&str) -> Option<String>>(
    vars: &[&'static str],
    in_cgi: bool,
    get_var: &F,
) -> Option<EnvProxy> {
    for &variable in vars
        .iter()
        .filter(|&&variable| !(in_cgi && variable == "HTTP_PROXY"))
    {
        if let Some(val) = get_var(variable) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(EnvProxy {
                    variable,
                    value: trimmed.to_string(),
                });
            }
        }
    }
    None
}

#[cfg(test)]
pub(crate) fn env_proxy_from<F, G>(get_var: F, has_var: G) -> Option<EnvProxy>
where
    F: Fn(&str) -> Option<String>,
    G: Fn(&str) -> bool,
{
    let in_cgi = detect_cgi(&has_var);
    read_env_group(&ENV_PROXY_VARS, in_cgi, &get_var)
}
