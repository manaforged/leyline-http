//! Compile-time pins on the public response-header accessor shapes.
//!
//! The full-list accessors return `impl Iterator<Item=(&str,&str)>` and the
//! by-name accessors return `Option<&str>` — representation-independent, so
//! internal header storage can change without touching a call site. This test
//! fails to COMPILE if any pinned signature changes, surfacing the break in the
//! normal `cargo test` run rather than only when a downstream consumer rebuilds.
//!
//! `.github/workflows/public-api.yml` pins the whole public surface via
//! cargo-public-api; this file is the toolchain-free guard for the accessors
//! that gate downstream code.

#![allow(dead_code)]

use leyline::Response;

/// Each binding pins one accessor's exact public shape. Never called — it only
/// needs to type-check.
fn _response_header_api_is_pinned(r: &Response) {
    // Full-list accessors: representation-independent iterators of `&str` pairs.
    let _headers: std::option::Option<(&str, &str)> = r.headers().next();
    let _trailers: std::option::Option<(&str, &str)> = r.trailers().next();

    // By-name accessors: plain `&str`, case-insensitive.
    let _first: std::option::Option<&str> = r.header("content-type");
    let _all: std::vec::Vec<&str> = r.header_all("set-cookie");
    let _ct: std::option::Option<&str> = r.content_type();
    let _len: std::option::Option<u64> = r.content_length();

    // Cookies: the precedent the header iterators match.
    let _cookies: std::option::Option<(&str, &str)> = r.cookies().next();
    let _cookie: std::option::Option<&str> = r.cookie("sid");
}

#[test]
fn public_header_api_compiles_in_pinned_shape() {
    // The pin is the signature of `_response_header_api_is_pinned` above; if it
    // compiles, the shapes hold. This runtime body just keeps the test honest.
    fn _takes_fn(_: fn(&Response)) {}
    _takes_fn(_response_header_api_is_pinned);
}
