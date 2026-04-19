//! Chrome-profiled cookie jar.
//!
//! Models the Chrome 147 cookie rules Leyline currently tests:
//! - Cookie header sorted by path length desc, then creation time asc
//! - SameSite=Lax default, SameSite=None requires Secure
//! - 180 cookies per domain, 3300 global, LRU eviction
//! - 400-day max lifetime cap
//! - Cookie prefixes (__Secure-, __Host-)
//! - Public suffix awareness via hardcoded TLD list

mod cookie;
mod jar;
mod parse;

pub use jar::CookieJar;
