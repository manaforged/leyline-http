use crate::core::{Kind, ProxyConfig, ProxyUrl, Result};

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

pub(super) fn apply_env_proxy(
    config: ProxyConfig,
    on_invalid: InvalidEnvProxy,
) -> Result<ProxyConfig> {
    apply_env_proxy_from(
        config,
        on_invalid,
        |k| std::env::var(k).ok(),
        |k| std::env::var_os(k).is_some(),
    )
}

pub(super) fn apply_env_proxy_from<F, G>(
    config: ProxyConfig,
    on_invalid: InvalidEnvProxy,
    get_var: F,
    has_var: G,
) -> Result<ProxyConfig>
where
    F: Fn(&str) -> Option<String>,
    G: Fn(&str) -> bool,
{
    if !config.rules().is_empty() || !config.uses_env() {
        return Ok(config);
    }
    let Some(EnvProxy { variable, value }) = env_proxy_from(get_var, has_var) else {
        return Ok(config);
    };
    if ProxyUrl::parse(&value).is_ok() {
        return Ok(config.set_default_proxy(value).set_from_env());
    }
    let config = config.reject_env(variable);
    match (on_invalid, config.rejection(Kind::Config)) {
        (InvalidEnvProxy::FailBuild, Some(error)) => Err(error),
        _ => Ok(config),
    }
}

pub(crate) fn env_proxy_from<F, G>(get_var: F, has_var: G) -> Option<EnvProxy>
where
    F: Fn(&str) -> Option<String>,
    G: Fn(&str) -> bool,
{
    let in_cgi = CGI_SIGNAL_ENV_VARS.iter().any(|k| has_var(k));

    if in_cgi && has_var("HTTP_PROXY") {
        tracing::warn!(
            target: "leyline::env_proxy::cgi",
            "CGI environment detected — ignoring HTTP_PROXY (httpoxy mitigation)"
        );
    }

    let candidates: &[&'static str] = if in_cgi {
        &[
            "HTTPS_PROXY",
            "https_proxy",
            "http_proxy",
            "ALL_PROXY",
            "all_proxy",
        ]
    } else {
        &[
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "ALL_PROXY",
            "all_proxy",
        ]
    };

    for &variable in candidates {
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
