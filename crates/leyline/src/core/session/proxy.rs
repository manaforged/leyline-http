/// Environment variable names whose presence indicates a CGI-like request-handler context where uppercase `HTTP_PROXY` is untrusted.
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

pub(super) fn env_proxy() -> Option<String> {
    env_proxy_from(|k| std::env::var(k).ok(), |k| std::env::var_os(k).is_some())
}

/// Pure-function core of [`env_proxy`]: given a value getter and a presence getter, return the first non-empty proxy URL while applying the httpoxy CGI sniff.
pub(crate) fn env_proxy_from<F, G>(get_var: F, has_var: G) -> Option<String>
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

    let candidates: &[&str] = if in_cgi {
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

    for name in candidates {
        if let Some(val) = get_var(name) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}
