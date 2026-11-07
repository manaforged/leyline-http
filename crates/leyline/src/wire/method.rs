use http::Method;

pub(crate) fn is_safe(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

pub(crate) fn is_idempotent(method: &Method) -> bool {
    is_safe(method) || matches!(*method, Method::PUT | Method::DELETE)
}

pub(crate) fn has_body_by_default(method: &Method) -> bool {
    matches!(*method, Method::POST | Method::PUT | Method::PATCH)
}

/// Returns whether `method` is GET or HEAD, the methods that send no `Origin` header and carry SameSite=Lax cookies.
pub(crate) fn is_retrieval(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD)
}
