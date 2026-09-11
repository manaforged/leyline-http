#![forbid(unsafe_code)]
mod jar;
pub(crate) mod parse;
mod record;

pub use jar::Jar;
pub(crate) use parse::rejected_cookie_name_value;
pub use record::{Cookie, SameSite};

pub(crate) fn is_cross_site(current: &url::Url, redirect_chain: &[String]) -> bool {
    let site_of =
        |host: &str| parse::registrable_domain(host).unwrap_or_else(|| host.to_ascii_lowercase());
    let Some(cur_host) = current.host_str() else {
        return false;
    };
    let cur_site = site_of(cur_host);
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
