//! Chrome-profiled cookie jar.
//!
//! Models the Chrome 147 cookie rules Leyline currently tests:
//! - Cookie header sorted by path length desc, then creation time asc
//! - SameSite=Lax default, SameSite=None requires Secure
//! - 180 cookies per domain, 3300 global, LRU eviction
//! - 400-day max lifetime cap
//! - Cookie prefixes (__Secure-, __Host-)
//! - Public suffix awareness via the Mozilla PSL (the `psl` crate)
//! - SameSite enforcement on cross-site requests (Strict/Lax withheld)
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

#![forbid(unsafe_code)]
// This module must stay free of `unsafe`; memory-unsafe code is confined to
// leyline-bssl* (FFI) and leyline's tcp/tls platform bridges.
mod jar;
mod parse;
mod record;

pub use jar::Jar;
pub(crate) use parse::rejected_cookie_name_value;
pub use record::{Cookie, SameSite};

/// Whether a request is cross-site relative to the navigation that started it,
/// for SameSite cookie enforcement. A navigation turns cross-site once any hop
/// — the current URL or any earlier redirect — leaves the registrable domain
/// (eTLD+1) of the original request. The first request of a chain is always
/// same-site. Returns `false` (same-site) when a host cannot be resolved, so
/// an unparseable URL never wrongly withholds cookies.
pub(crate) fn is_cross_site(current: &url::Url, redirect_chain: &[String]) -> bool {
    let site_of =
        |host: &str| parse::registrable_domain(host).unwrap_or_else(|| host.to_ascii_lowercase());
    let Some(cur_host) = current.host_str() else {
        return false;
    };
    let cur_site = site_of(cur_host);
    // The navigation's site is the registrable domain of the original request:
    // the first redirect-chain entry, or the current URL when none has occurred.
    let nav_site = match redirect_chain.first().and_then(|f| url::Url::parse(f).ok()) {
        Some(orig) => orig.host_str().map(site_of),
        None => Some(cur_site.clone()),
    };
    let Some(nav_site) = nav_site else {
        return false;
    };
    if cur_site != nav_site {
        return true;
    }
    redirect_chain.iter().any(|u| {
        url::Url::parse(u)
            .ok()
            .and_then(|u| u.host_str().map(site_of))
            .map(|s| s != nav_site)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests;
