use super::*;

pub(super) fn validate(method: &str, headers: &[(String, String)]) -> Result<(), H1PooledError> {
    if !is_valid_token(method) {
        return Err(H1PooledError::Config(format!(
            "invalid HTTP method `{method}`: non-token bytes not allowed"
        )));
    }
    for (name, value) in headers {
        if !is_valid_token(name) {
            return Err(H1PooledError::Config(format!(
                "invalid header name `{name}`: non-token bytes not allowed"
            )));
        }
        if !is_valid_header_value(value) {
            return Err(H1PooledError::Config(format!(
                "invalid value for header `{name}`: control characters not allowed"
            )));
        }
    }
    Ok(())
}

pub(super) fn target(url: &url::Url, target: H1Target) -> Result<(String, String), H1PooledError> {
    let host = url
        .host_str()
        .ok_or_else(|| H1PooledError::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        H1PooledError::Config(format!("no default port for scheme {}", url.scheme()))
    })?;
    let authority = authority_for(url, host, port);
    let path = path_and_query(url);
    let request_target = match target {
        H1Target::OriginForm => path,
        H1Target::AbsoluteForm => {
            format!("{}://{}{}", url.scheme(), authority, path)
        }
    };
    if !is_valid_request_target(&request_target) {
        return Err(H1PooledError::Config(
            "invalid request target: control characters not allowed".to_string(),
        ));
    }
    Ok((request_target, authority))
}

pub(super) fn frame(method: &str, headers: &mut Vec<(String, String)>, body: H1Body) -> Framing {
    let has_cl = contains_header(headers, "content-length");
    let has_te = contains_header(headers, "transfer-encoding");
    match body {
        H1Body::Empty => {
            if method_typically_has_body(method) && !has_cl && !has_te {
                headers.push(("Content-Length".into(), "0".into()));
            }
            Framing::None
        }
        H1Body::Buffered(b) => {
            if !has_cl && !has_te {
                headers.push(("Content-Length".into(), b.len().to_string()));
            }
            Framing::Buffered(b)
        }
        H1Body::FixedStream { stream, length } => {
            if !has_cl && !has_te {
                headers.push(("Content-Length".into(), length.to_string()));
            }
            Framing::Fixed { stream, length }
        }
        H1Body::ChunkedStream { stream } => {
            if !has_te {
                headers.push(("Transfer-Encoding".into(), "chunked".into()));
            }
            Framing::Chunked { stream }
        }
    }
}

pub(super) fn head(method: &str, request_target: &str, headers: &[(String, String)]) -> Vec<u8> {
    let mut req = Vec::new();
    req.extend_from_slice(format!("{method} {request_target} HTTP/1.1\r\n").as_bytes());
    for (name, value) in headers {
        let name = h1_header_name(name);
        req.extend_from_slice(name.as_bytes());
        req.extend_from_slice(b": ");
        req.extend_from_slice(value.as_bytes());
        req.extend_from_slice(b"\r\n");
    }
    req.extend_from_slice(b"\r\n");
    req
}
