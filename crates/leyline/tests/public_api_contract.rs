//! Compile-time pins on the public response-header accessor shapes.

#![allow(dead_code)]

use leyline::Response;

/// Each binding pins one accessor's exact public shape.
fn _response_header_api_is_pinned(r: &Response) {
    let _headers: std::option::Option<(&str, &str)> = r.headers().next();
    let _trailers: std::option::Option<(&str, &str)> = r.trailers().next();

    let _first: std::option::Option<&str> = r.header("content-type");
    let _all: std::option::Option<&str> = r.header_all("set-cookie").next();
    let _ct: std::option::Option<&str> = r.content_type();
    let _len: std::option::Option<u64> = r.content_length();

    let _cookies: std::option::Option<(&str, &str)> = r.cookies().next();
    let _cookie: std::option::Option<&str> = r.cookie("sid");

    let _req: std::option::Option<(&str, &str)> = r.request_headers().next();
}

#[test]
fn public_header_api_compiles_in_pinned_shape() {
    fn _takes_fn(_: fn(&Response)) {}
    _takes_fn(_response_header_api_is_pinned);
}
