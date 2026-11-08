use std::sync::Arc;

use http::header::USER_AGENT;

use crate::audit::{AuditTlsCache, ClientHello};
use crate::core::{Identity, Protocol};
use crate::error::Result;
use crate::h2::H2Config;
use crate::profile::TcpProfile;
use crate::profile::{Browser, BrowserProfile, ChromiumBrand, HeaderStyle, Platform};

pub(in crate::core::session) enum IdentitySource {
    Bare,
    Profile(Arc<BrowserProfile>),
    Browser { tls: Browser, http: Browser },
}

pub(in crate::core::session) struct IdentityInputs<'a> {
    pub source: IdentitySource,
    pub platform: Platform,
    pub brand: ChromiumBrand,
    pub protocol: Protocol,
    pub tcp: &'a TcpProfile,
    pub audit: bool,
    pub default_headers: &'a [(String, String)],
    pub languages: Option<&'a [String]>,
}

pub(in crate::core::session) struct DerivedIdentity {
    pub browser: Option<Browser>,
    pub identity: Option<Identity>,
    pub platform: Platform,
    pub profile: Arc<BrowserProfile>,
    pub header_style: HeaderStyle,
    pub header_order: Option<Vec<String>>,
    pub user_agent: String,
    pub sec_ch_ua: String,
    pub accept_language: String,
    pub h2_config: H2Config,
    #[cfg(feature = "http3")]
    pub h3_config: Option<crate::quic::H3Config>,
    pub audit_tls: Option<Arc<AuditTlsCache>>,
}

struct Selected {
    browser: Option<Browser>,
    identity: Option<Identity>,
    http_profile: Option<&'static BrowserProfile>,
    base: Arc<BrowserProfile>,
}

impl Selected {
    fn new(source: IdentitySource, platform: Platform) -> Result<Self> {
        match source {
            IdentitySource::Bare => Ok(Self::without_browser(BrowserProfile::bare_shared())),
            IdentitySource::Profile(profile) => Ok(Self::without_browser(profile)),
            IdentitySource::Browser { tls, http } => Self::for_browsers(tls, http, platform),
        }
    }

    fn without_browser(base: Arc<BrowserProfile>) -> Self {
        Self {
            browser: None,
            identity: None,
            http_profile: None,
            base,
        }
    }

    fn for_browsers(tls: Browser, http: Browser, platform: Platform) -> Result<Self> {
        let tls = tls.for_platform(platform);
        let http = http.for_platform(platform);
        Ok(Self {
            browser: Some(tls),
            identity: Some(Identity::new(http, platform).rotate_tls(tls)?),
            http_profile: Some(http.profile()),
            base: tls.shared_profile(),
        })
    }
}

fn session_user_agent(default_headers: &[(String, String)]) -> Option<&str> {
    default_headers
        .iter()
        .rev()
        .find(|(name, _)| name.eq_ignore_ascii_case(USER_AGENT.as_str()))
        .map(|(_, value)| value.as_str())
}

fn session_accept_language(
    languages: Option<&[String]>,
    style: HeaderStyle,
    bare: bool,
    profile_value: Option<String>,
) -> String {
    match languages {
        Some(tags) => crate::profile::languages::accept_language(tags, style),
        None if bare => String::new(),
        None => profile_value.unwrap_or_default(),
    }
}

pub(in crate::core::session) fn derive_identity(
    input: IdentityInputs<'_>,
) -> Result<DerivedIdentity> {
    let platform = input.platform.resolve();
    let brand = input.brand;
    let bare = matches!(input.source, IdentitySource::Bare);
    let selected = Selected::new(input.source, platform)?;
    let profile = brand.tls_profile(selected.base);
    let http_profile = selected.http_profile.unwrap_or(&*profile);
    let header_style = brand
        .header_style()
        .unwrap_or(http_profile.meta.header_style);
    let header_order = http_profile.meta.header_order.clone();
    let resolved = http_profile.resolve_identity(platform, brand)?;
    let h2_config = H2Config::from_profile(&profile.h2.resolve_for_platform(platform)?)?;
    #[cfg(feature = "http3")]
    let h3_config = if profile.h3.is_none() && !input.protocol.requires_h3() {
        None
    } else {
        Some(crate::quic::H3Config::from_profile(&profile)?)
    };
    #[cfg(feature = "http3")]
    let quic_tls = h3_config.as_ref().map(|config| &config.tls);
    #[cfg(not(feature = "http3"))]
    let quic_tls = None;
    let hello = if input.protocol.tcp_offers_h2() {
        ClientHello::TcpH2
    } else {
        ClientHello::TcpHttp11
    };
    let audit_tls = input.audit.then(|| {
        Arc::new(AuditTlsCache::new(
            &profile, &h2_config, input.tcp, hello, quic_tls,
        ))
    });

    Ok(DerivedIdentity {
        browser: selected.browser,
        identity: selected.identity,
        platform,
        profile,
        header_style,
        header_order,
        sec_ch_ua: match session_user_agent(input.default_headers) {
            Some(_) => String::new(),
            None => resolved.sec_ch_ua,
        },
        user_agent: session_user_agent(input.default_headers)
            .map_or(resolved.user_agent, str::to_owned),
        accept_language: session_accept_language(
            input.languages,
            header_style,
            bare,
            resolved.accept_language,
        ),
        h2_config,
        #[cfg(feature = "http3")]
        h3_config,
        audit_tls,
    })
}
