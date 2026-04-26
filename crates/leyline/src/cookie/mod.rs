//! Chrome-profiled cookie jar.
//!
//! Models the Chrome 147 cookie rules Leyline currently tests:
//! - Cookie header sorted by path length desc, then creation time asc
//! - SameSite=Lax default, SameSite=None requires Secure
//! - 180 cookies per domain, 3300 global, LRU eviction
//! - 400-day max lifetime cap
//! - Cookie prefixes (__Secure-, __Host-)
//! - Public suffix awareness via hardcoded TLD list
//!
//! ## Persistence
//!
//! [`Jar`] is `Serialize` + `Deserialize`. The wire format is a flat list of
//! [`Cookie`] records; on load, cookies are re-bucketed into the in-memory
//! domain map without going through the `Set-Cookie` parse path. This means
//! a jar that holds a host-only cookie on `api.example.com` and a
//! `Domain=example.com` cookie on `example.com` round-trips losslessly —
//! every domain attribution is preserved, which the legacy `String`-shaped
//! `Cookie:` header export silently dropped.

mod jar;
mod parse;
mod record;

pub use jar::Jar;
pub use record::{Cookie, SameSite};
