//! C FFI for Leyline.
//!
//! Exposes Leyline's API through opaque handles and C-compatible functions.
//! Owns a tokio runtime internally — all async operations block from the
//! caller's perspective. Language wrappers add their own async on top.
//!
//! # Memory rules
//! - Strings returned by `leyline_*` functions are heap-allocated. Free
//!   with `leyline_free_string`.
//! - Session, Response, and WebSocket handles are heap-allocated. Free
//!   with `leyline_session_free` / `leyline_response_free` /
//!   `leyline_ws_free`.
//! - All handles are internally `Send + Sync` and safe to share across
//!   threads once constructed.
//!
//! # Error handling
//! `leyline_last_error` is **thread-local**. Every `leyline_*` function
//! clears and possibly sets the error slot on the calling thread. Read
//! the error on the same thread that produced it; otherwise the read
//! returns null even when the call failed. Language wrappers that
//! schedule FFI calls onto a worker thread must marshal the error off
//! that thread immediately after each call returns.
//!
//! # Runtime failures
//! The tokio runtime is initialised lazily on first use. If construction
//! fails (extremely rare — only on a broken host), functions that need
//! the runtime set `leyline_last_error` and return a null handle (or 0
//! for numeric returns, -1 for status codes). The FFI never aborts the
//! host process.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;

use leyline::{Browser, Platform, Session};

/// Return early with a null/zero/error if a handle pointer is null.
macro_rules! check_null {
    ($ptr:expr, $ret:expr) => {
        if $ptr.is_null() {
            set_error("null handle".into());
            return $ret;
        }
    };
    ($ptr:expr) => {
        check_null!($ptr, std::ptr::null_mut())
    };
}

// ─── Runtime ────────────────────────────────────────────────────────────

static RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();

/// Return the lazily-initialised tokio runtime, or set an error and return
/// `None` if construction failed. Callers that return `*mut T` should use
/// [`rt_or_null`]; callers that return a status code should inspect the
/// return and bail out.
fn rt() -> Option<&'static tokio::runtime::Runtime> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| set_error(format!("failed to create tokio runtime: {e}")))
                .ok()
        })
        .as_ref()
}

/// Shorthand for FFI entry points that return `*mut T`: get the runtime
/// or set the last error and early-return a null pointer.
macro_rules! rt_or_null {
    () => {
        match rt() {
            Some(rt) => rt,
            None => {
                if LAST_ERROR.with(|e| e.borrow().is_none()) {
                    set_error("tokio runtime unavailable".into());
                }
                return std::ptr::null_mut();
            }
        }
    };
}

/// Shorthand for FFI entry points that return a numeric status code.
macro_rules! rt_or_status {
    ($fail:expr) => {
        match rt() {
            Some(rt) => rt,
            None => {
                if LAST_ERROR.with(|e| e.borrow().is_none()) {
                    set_error("tokio runtime unavailable".into());
                }
                return $fail;
            }
        }
    };
}

// ─── Error handling ─────────────────────────────────────────────────────

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn set_error(msg: String) {
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(msg));
}

fn clear_error() {
    LAST_ERROR.with(|e| *e.borrow_mut() = None);
}

/// Get the last error message, or null if no error.
/// Caller must free the returned string with `leyline_free_string`.
#[no_mangle]
pub extern "C" fn leyline_last_error() -> *mut c_char {
    LAST_ERROR.with(|e| match e.borrow().as_ref() {
        Some(msg) => CString::new(msg.as_str()).unwrap_or_default().into_raw(),
        None => std::ptr::null_mut(),
    })
}

/// Free a string returned by any `leyline_*` function.
#[no_mangle]
pub extern "C" fn leyline_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────

fn cstr_to_str<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(s) }.to_str().ok()
}

fn to_c_string(s: &str) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

// ─── Session ────────────────────────────────────────────────────────────

/// Opaque session handle.
pub struct LeylineSession {
    inner: Session,
}

/// Create a session using the latest bundled Chrome profile. Returns null
/// on error (check `leyline_last_error`). The resolved browser version
/// drifts as profiles are added — pass a pinned browser string to
/// [`leyline_session_new`] when you need version stability.
#[no_mangle]
pub extern "C" fn leyline_session_chrome() -> *mut LeylineSession {
    clear_error();
    match Session::chrome_latest() {
        Ok(s) => Box::into_raw(Box::new(LeylineSession { inner: s })),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Create a session using the latest bundled Firefox profile.
#[no_mangle]
pub extern "C" fn leyline_session_firefox() -> *mut LeylineSession {
    clear_error();
    match Session::firefox_latest() {
        Ok(s) => Box::into_raw(Box::new(LeylineSession { inner: s })),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Create a session using the latest bundled Safari profile.
#[no_mangle]
pub extern "C" fn leyline_session_safari() -> *mut LeylineSession {
    clear_error();
    match Session::safari_latest() {
        Ok(s) => Box::into_raw(Box::new(LeylineSession { inner: s })),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Create a session with full configuration.
/// `browser`: "chrome147", "firefox148", "safari18", etc.
/// `platform`: "windows", "macos", "linux", "ios", "android"
/// `proxy`: proxy URL or null for no proxy
/// `timeout_secs`: request timeout in seconds (0 = default 30s)
#[no_mangle]
pub extern "C" fn leyline_session_new(
    browser: *const c_char,
    platform: *const c_char,
    proxy: *const c_char,
    timeout_secs: u32,
) -> *mut LeylineSession {
    clear_error();

    let browser_str = match cstr_to_str(browser) {
        Some(s) => s,
        None => {
            set_error("browser is null".into());
            return std::ptr::null_mut();
        }
    };
    let platform_str = cstr_to_str(platform).unwrap_or("windows");
    let proxy_str = cstr_to_str(proxy);

    let browser_enum = match parse_browser(browser_str) {
        Some(b) => b,
        None => {
            set_error(format!("unknown browser: {browser_str}"));
            return std::ptr::null_mut();
        }
    };
    let platform_enum = parse_platform(platform_str);

    let mut builder = Session::builder()
        .browser(browser_enum)
        .platform(platform_enum);

    if let Some(p) = proxy_str {
        builder = builder.proxy(p);
    }
    if timeout_secs > 0 {
        builder = builder.timeout(std::time::Duration::from_secs(timeout_secs as u64));
    }

    match builder.build() {
        Ok(s) => Box::into_raw(Box::new(LeylineSession { inner: s })),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Free a session.
#[no_mangle]
pub extern "C" fn leyline_session_free(session: *mut LeylineSession) {
    if !session.is_null() {
        unsafe {
            drop(Box::from_raw(session));
        }
    }
}

// ─── Response ───────────────────────────────────────────────────────────

/// Opaque response handle.
pub struct LeylineResponse {
    inner: leyline_core::Response,
}

/// GET a URL. Returns null on error.
#[no_mangle]
pub extern "C" fn leyline_session_get(
    session: *const LeylineSession,
    url: *const c_char,
) -> *mut LeylineResponse {
    clear_error();
    check_null!(session);
    let rt = rt_or_null!();
    let session = unsafe { &(*session).inner };
    let url = match cstr_to_str(url) {
        Some(s) => s,
        None => {
            set_error("url is null".into());
            return std::ptr::null_mut();
        }
    };

    match rt.block_on(session.navigate(url)) {
        Ok(r) => Box::into_raw(Box::new(LeylineResponse { inner: r })),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// POST JSON. `body` is a JSON string.
#[no_mangle]
pub extern "C" fn leyline_session_post_json(
    session: *const LeylineSession,
    url: *const c_char,
    body: *const c_char,
) -> *mut LeylineResponse {
    clear_error();
    check_null!(session);
    let rt = rt_or_null!();
    let session = unsafe { &(*session).inner };
    let url = match cstr_to_str(url) {
        Some(s) => s,
        None => {
            set_error("url is null".into());
            return std::ptr::null_mut();
        }
    };
    let body_str = match cstr_to_str(body) {
        Some(s) => s,
        None => {
            set_error("body is null".into());
            return std::ptr::null_mut();
        }
    };

    let value: serde_json::Value = match serde_json::from_str(body_str) {
        Ok(v) => v,
        Err(e) => {
            set_error(format!("invalid JSON: {e}"));
            return std::ptr::null_mut();
        }
    };

    match rt.block_on(session.post_json(url, &value)) {
        Ok(r) => Box::into_raw(Box::new(LeylineResponse { inner: r })),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// POST form data. `data` is "key=value&key2=value2".
#[no_mangle]
pub extern "C" fn leyline_session_post_form(
    session: *const LeylineSession,
    url: *const c_char,
    data: *const c_char,
) -> *mut LeylineResponse {
    clear_error();
    check_null!(session);
    let rt = rt_or_null!();
    let session = unsafe { &(*session).inner };
    let url = match cstr_to_str(url) {
        Some(s) => s,
        None => {
            set_error("url is null".into());
            return std::ptr::null_mut();
        }
    };
    let data_str = match cstr_to_str(data) {
        Some(s) => s,
        None => {
            set_error("data is null".into());
            return std::ptr::null_mut();
        }
    };

    match rt.block_on(session.post_form_str(url, data_str)) {
        Ok(r) => Box::into_raw(Box::new(LeylineResponse { inner: r })),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Free a response.
#[no_mangle]
pub extern "C" fn leyline_response_free(response: *mut LeylineResponse) {
    if !response.is_null() {
        unsafe {
            drop(Box::from_raw(response));
        }
    }
}

/// Get the HTTP status code.
#[no_mangle]
pub extern "C" fn leyline_response_status(response: *const LeylineResponse) -> u16 {
    check_null!(response, 0);
    unsafe { (*response).inner.status() }
}

/// Get the HTTP version string (`HTTP/1.1`, `HTTP/2`, or `HTTP/3`). Caller must free.
#[no_mangle]
pub extern "C" fn leyline_response_version(response: *const LeylineResponse) -> *mut c_char {
    check_null!(response);
    to_c_string(unsafe { &*response }.inner.version().as_str())
}

/// Get the negotiated TLS ALPN string, if available. Caller must free.
#[no_mangle]
pub extern "C" fn leyline_response_tls_alpn(response: *const LeylineResponse) -> *mut c_char {
    check_null!(response);
    match unsafe { &*response }.inner.tls_alpn() {
        Some(alpn) => to_c_string(alpn),
        None => std::ptr::null_mut(),
    }
}

/// Get the response body as a string. Caller must free with `leyline_free_string`.
#[no_mangle]
pub extern "C" fn leyline_response_text(response: *const LeylineResponse) -> *mut c_char {
    check_null!(response);
    to_c_string(&unsafe { &*response }.inner.text())
}

/// Get the response body length in bytes.
#[no_mangle]
pub extern "C" fn leyline_response_body_len(response: *const LeylineResponse) -> usize {
    check_null!(response, 0);
    unsafe { (*response).inner.bytes().len() }
}

/// Copy up to `buf_len` bytes of the response body into the caller-owned
/// buffer `buf`, starting at byte `offset` in the body. Returns the number
/// of bytes copied (0 if `offset` is at or past the body end, or on null
/// handle). Use [`leyline_response_body_len`] first to size the buffer.
///
/// # Safety
/// `buf` must point to writeable memory of at least `buf_len` bytes. The
/// FFI does not retain the buffer pointer.
#[no_mangle]
pub unsafe extern "C" fn leyline_response_body_copy(
    response: *const LeylineResponse,
    offset: usize,
    buf: *mut u8,
    buf_len: usize,
) -> usize {
    if response.is_null() || buf.is_null() || buf_len == 0 {
        return 0;
    }
    let body = (*response).inner.bytes();
    if offset >= body.len() {
        return 0;
    }
    let available = body.len() - offset;
    let n = available.min(buf_len);
    std::ptr::copy_nonoverlapping(body.as_ptr().add(offset), buf, n);
    n
}

/// Get the final URL (after redirects). Caller must free.
#[no_mangle]
pub extern "C" fn leyline_response_url(response: *const LeylineResponse) -> *mut c_char {
    check_null!(response);
    to_c_string(unsafe { &*response }.inner.url())
}

/// Get a response header value by name. Returns null if not found. Caller must free.
#[no_mangle]
pub extern "C" fn leyline_response_header(
    response: *const LeylineResponse,
    name: *const c_char,
) -> *mut c_char {
    check_null!(response);
    let name = match cstr_to_str(name) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    match unsafe { &*response }.inner.header(name) {
        Some(val) => to_c_string(val),
        None => std::ptr::null_mut(),
    }
}

/// Get all response headers as a JSON object string. Duplicate names become arrays.
/// Caller must free. Prefer `leyline_response_headers_array_json` for wire order.
#[no_mangle]
pub extern "C" fn leyline_response_headers_json(response: *const LeylineResponse) -> *mut c_char {
    check_null!(response);
    let resp = unsafe { &*response };
    let mut map = serde_json::Map::new();
    for (k, v) in resp.inner.headers() {
        let key = k.to_ascii_lowercase();
        match map.get_mut(&key) {
            Some(serde_json::Value::Array(values)) => {
                values.push(serde_json::Value::String(v.clone()));
            }
            Some(existing) => {
                let first = existing.take();
                *existing =
                    serde_json::Value::Array(vec![first, serde_json::Value::String(v.clone())]);
            }
            None => {
                map.insert(key, serde_json::Value::String(v.clone()));
            }
        }
    }
    let json = serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string());
    to_c_string(&json)
}

/// Get all response headers as an ordered JSON array of `[name, value]` pairs.
/// Caller must free.
#[no_mangle]
pub extern "C" fn leyline_response_headers_array_json(
    response: *const LeylineResponse,
) -> *mut c_char {
    check_null!(response);
    let resp = unsafe { &*response };
    let json = serde_json::to_string(resp.inner.headers()).unwrap_or_else(|_| "[]".to_string());
    to_c_string(&json)
}

/// Get response trailers as an ordered JSON array of `[name, value]` pairs.
/// Caller must free.
#[no_mangle]
pub extern "C" fn leyline_response_trailers_json(response: *const LeylineResponse) -> *mut c_char {
    check_null!(response);
    let resp = unsafe { &*response };
    let json = serde_json::to_string(resp.inner.trailers()).unwrap_or_else(|_| "[]".to_string());
    to_c_string(&json)
}

/// Get audit data as JSON. Returns null if not available. Caller must free.
#[no_mangle]
pub extern "C" fn leyline_response_audit_json(response: *const LeylineResponse) -> *mut c_char {
    check_null!(response);
    let resp = unsafe { &*response };
    match resp.inner.audit() {
        Some(audit) => {
            let json = serde_json::json!({
                "ja4": audit.ja4,
                "ja3": audit.ja3,
                "h2_fingerprint": audit.h2_fingerprint,
                "ja4t": audit.ja4t,
                "ja4h": audit.ja4h,
            });
            to_c_string(&json.to_string())
        }
        None => std::ptr::null_mut(),
    }
}

// ─── One-liner API ──────────────────────────────────────────────────────

/// Quick GET with default Chrome session. Returns null on error.
#[no_mangle]
pub extern "C" fn leyline_get(url: *const c_char) -> *mut LeylineResponse {
    clear_error();
    let rt = rt_or_null!();
    let url = match cstr_to_str(url) {
        Some(s) => s,
        None => {
            set_error("url is null".into());
            return std::ptr::null_mut();
        }
    };

    match rt.block_on(leyline::get(url)) {
        Ok(r) => Box::into_raw(Box::new(LeylineResponse { inner: r })),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

// ─── WebSocket ──────────────────────────────────────────────────────────

/// Opaque WebSocket handle.
pub struct LeylineWebSocket {
    inner: leyline_core::WsConnection,
}

/// Connect to a WebSocket URL. Returns null on error.
#[no_mangle]
pub extern "C" fn leyline_session_websocket(
    session: *const LeylineSession,
    url: *const c_char,
) -> *mut LeylineWebSocket {
    clear_error();
    check_null!(session);
    let rt = rt_or_null!();
    let session = unsafe { &(*session).inner };
    let url = match cstr_to_str(url) {
        Some(s) => s,
        None => {
            set_error("url is null".into());
            return std::ptr::null_mut();
        }
    };
    match rt.block_on(session.websocket(url)) {
        Ok(ws) => Box::into_raw(Box::new(LeylineWebSocket { inner: ws })),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Send a text message. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn leyline_ws_send(ws: *mut LeylineWebSocket, msg: *const c_char) -> i32 {
    clear_error();
    check_null!(ws, -1);
    let rt = rt_or_status!(-1);
    let ws = unsafe { &mut (*ws).inner };
    let msg = match cstr_to_str(msg) {
        Some(s) => s,
        None => {
            set_error("msg is null".into());
            return -1;
        }
    };
    match rt.block_on(ws.send(msg)) {
        Ok(()) => 0,
        Err(e) => {
            set_error(e.to_string());
            -1
        }
    }
}

/// Receive a message. Returns null on close/error. Caller must free.
#[no_mangle]
pub extern "C" fn leyline_ws_recv(ws: *mut LeylineWebSocket) -> *mut c_char {
    clear_error();
    check_null!(ws);
    let rt = rt_or_null!();
    let ws = unsafe { &mut (*ws).inner };
    match rt.block_on(ws.recv()) {
        Ok(Some(msg)) => to_c_string(&msg.to_string()),
        Ok(None) => std::ptr::null_mut(),
        Err(e) => {
            set_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Close a WebSocket.
#[no_mangle]
pub extern "C" fn leyline_ws_close(ws: *mut LeylineWebSocket) -> i32 {
    clear_error();
    check_null!(ws, -1);
    let rt = rt_or_status!(-1);
    let ws = unsafe { &mut (*ws).inner };
    match rt.block_on(ws.close()) {
        Ok(()) => 0,
        Err(e) => {
            set_error(e.to_string());
            -1
        }
    }
}

/// Free a WebSocket handle.
#[no_mangle]
pub extern "C" fn leyline_ws_free(ws: *mut LeylineWebSocket) {
    if !ws.is_null() {
        unsafe {
            drop(Box::from_raw(ws));
        }
    }
}

// ─── Internal helpers ───────────────────────────────────────────────────

fn parse_browser(s: &str) -> Option<Browser> {
    Some(match s.to_lowercase().as_str() {
        "chrome147" | "chrome" => Browser::Chrome147,
        "chrome146" => Browser::Chrome146,
        "chrome145" => Browser::Chrome145,
        "firefox148" | "firefox" => Browser::Firefox148,
        "safari18" | "safari" => Browser::Safari18,
        "okhttp_android10" | "okhttp" => Browser::OkHttpAndroid10,
        "okhttp_android7" => Browser::OkHttpAndroid7,
        "safari_ios15" => Browser::SafariiOS15,
        "safari_ios17" => Browser::SafariiOS17,
        "safari_ios18" => Browser::SafariiOS18,
        _ => return None,
    })
}

fn parse_platform(s: &str) -> Platform {
    match s.to_lowercase().as_str() {
        "windows" | "win" => Platform::Windows,
        "macos" | "mac" | "osx" => Platform::MacOS,
        "linux" => Platform::Linux,
        "android" => Platform::Android,
        "ios" => Platform::IOS,
        _ => Platform::Windows,
    }
}
