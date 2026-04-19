//! Chromium-family identity overlays (Edge, Brave, Opera).
//!
//! Edge, Brave, and Opera ship the same Chromium codebase as Chrome —
//! same TLS ClientHello, same H2 SETTINGS. They differ only in HTTP
//! identity headers (`User-Agent` suffix, `sec-ch-ua` brand, a
//! privacy header or two). A [`ChromiumBrand`] is overlaid on an
//! existing Chrome profile at `Session` build time; JA4 and Akamai
//! fingerprint stay bit-for-bit Chromium.

use crate::profile::Platform;

/// Chromium-family browser skin applied on top of a Chrome profile.
/// Affects only HTTP identity headers — never TLS or HTTP/2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ChromiumBrand {
    /// Stock Chrome. No overlay.
    #[default]
    Chrome,
    /// Microsoft Edge. Adds `Edg/N` to UA, swaps `sec-ch-ua` brand,
    /// ships `dnt: 1`.
    Edge,
    /// Brave. Keeps Chrome's UA, swaps `sec-ch-ua` brand, ships
    /// `sec-gpc: 1`, drops `signed-exchange` from the Navigate Accept.
    Brave,
    /// Opera. Adds `OPR/N` to UA, swaps `sec-ch-ua` brand.
    Opera,
}

/// Reason an overlay could not be produced — surfaced to the caller
/// rather than emitting a header set we can't verify.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BrandOverlayError {
    /// Requested brand has no verified overlay for this
    /// Chromium/platform combination.
    Unverified {
        /// Brand that was requested.
        brand: ChromiumBrand,
        /// Chromium major the active profile anchors to.
        chromium_major: u32,
        /// Platform for the request.
        platform: Platform,
    },
}

impl std::fmt::Display for BrandOverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unverified {
                brand,
                chromium_major,
                platform,
            } => write!(
                f,
                "{} overlay on Chromium {chromium_major} / {platform} is not verified \
                 against a live capture; drop the brand or pick an anchor that has one",
                brand.label(),
            ),
        }
    }
}

impl std::error::Error for BrandOverlayError {}

impl ChromiumBrand {
    /// Human-readable label, e.g. for logs.
    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Brave => "Brave",
            Self::Opera => "Opera",
        }
    }

    /// Return the brand-specific identity overlay for the given
    /// Chromium anchor + platform. Uses the active profile's UA and
    /// `sec-ch-ua` as input so the overlay inherits version-
    /// dependent GREASE tokens and slot ordering verbatim.
    ///
    /// `Ok(None)` means Chrome — no overlay. `Err` means we don't
    /// have a capture that justifies the overlay for this
    /// combination; the caller MUST surface the error rather than
    /// guess.
    pub fn overlay(
        self,
        chromium_major: u32,
        platform: Platform,
        profile_user_agent: &str,
        profile_sec_ch_ua: &str,
    ) -> Result<Option<BrandOverlay>, BrandOverlayError> {
        match self {
            Self::Chrome => Ok(None),
            Self::Edge => edge_overlay(
                chromium_major,
                platform,
                profile_user_agent,
                profile_sec_ch_ua,
            )
            .map(Some),
            Self::Brave => brave_overlay(
                chromium_major,
                platform,
                profile_user_agent,
                profile_sec_ch_ua,
            )
            .map(Some),
            Self::Opera => opera_overlay(chromium_major, platform, profile_user_agent).map(Some),
        }
    }
}

/// Concrete header overrides applied on top of the active Chromium
/// profile. All fields are already computed against the active
/// profile — the caller copies them in without further transformation.
#[derive(Debug, Clone)]
pub struct BrandOverlay {
    /// Final `User-Agent` string (base profile UA plus any
    /// brand-specific suffix). For Brave this equals the input UA.
    pub user_agent: String,
    /// Final `sec-ch-ua` brand list with the sibling brand name
    /// spliced into the profile's slot structure.
    pub sec_ch_ua: String,
    /// Additional headers this brand ships by default (e.g. `dnt`,
    /// `sec-gpc`). Applied after the preset's identity block; the
    /// caller is responsible for deduping against user-supplied
    /// headers and the preset's own output.
    pub extra_headers: Vec<(String, String)>,
    /// `Accept` header override for the Navigate preset. Most
    /// Chromium siblings match Chrome here; Brave drops the
    /// `application/signed-exchange` suffix.
    pub navigate_accept: Option<String>,
}

/// Desktop-only platforms for which the UA suffixes match real
/// captures. Mobile variants use different suffixes (`EdgA/`,
/// `EdgiOS/`, `OPT/`, `OPX/`) that we haven't captured, so they
/// return `BrandOverlayError::Unverified`.
///
/// Written as an exhaustive match so adding a new `Platform`
/// variant (the enum is `#[non_exhaustive]`) produces a compile
/// error here rather than a silent mis-classification.
fn desktop_only(platform: Platform) -> bool {
    match platform {
        Platform::Windows | Platform::MacOS | Platform::Linux => true,
        Platform::Android | Platform::IOS => false,
    }
}

fn edge_overlay(
    chromium_major: u32,
    platform: Platform,
    profile_user_agent: &str,
    profile_sec_ch_ua: &str,
) -> Result<BrandOverlay, BrandOverlayError> {
    if !desktop_only(platform) {
        return Err(BrandOverlayError::Unverified {
            brand: ChromiumBrand::Edge,
            chromium_major,
            platform,
        });
    }
    Ok(BrandOverlay {
        user_agent: format!("{profile_user_agent} Edg/{chromium_major}.0.0.0"),
        sec_ch_ua: swap_brand(profile_sec_ch_ua, "Microsoft Edge"),
        extra_headers: vec![("dnt".into(), "1".into())],
        navigate_accept: None,
    })
}

fn brave_overlay(
    chromium_major: u32,
    platform: Platform,
    profile_user_agent: &str,
    profile_sec_ch_ua: &str,
) -> Result<BrandOverlay, BrandOverlayError> {
    if !desktop_only(platform) {
        return Err(BrandOverlayError::Unverified {
            brand: ChromiumBrand::Brave,
            chromium_major,
            platform,
        });
    }
    Ok(BrandOverlay {
        user_agent: profile_user_agent.to_string(),
        sec_ch_ua: swap_brand(profile_sec_ch_ua, "Brave"),
        extra_headers: vec![("sec-gpc".into(), "1".into())],
        navigate_accept: Some(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,\
             image/webp,image/apng,*/*;q=0.8"
                .into(),
        ),
    })
}

fn opera_overlay(
    chromium_major: u32,
    platform: Platform,
    profile_user_agent: &str,
) -> Result<BrandOverlay, BrandOverlayError> {
    // Opera's sec-ch-ua uses a different GREASE placeholder and slot
    // order than Chrome's, so we can't derive it from the profile by
    // a simple swap. We only emit an overlay for pairs we've
    // verified by live capture.
    let opera_version = match (chromium_major, desktop_only(platform)) {
        (145, true) => 129, // Live capture.
        _ => {
            return Err(BrandOverlayError::Unverified {
                brand: ChromiumBrand::Opera,
                chromium_major,
                platform,
            });
        }
    };
    Ok(BrandOverlay {
        user_agent: format!("{profile_user_agent} OPR/{opera_version}.0.0.0"),
        sec_ch_ua: format!(
            r#""Not:A-Brand";v="99", "Opera";v="{opera_version}", "Chromium";v="{chromium_major}""#
        ),
        extra_headers: Vec::new(),
        navigate_accept: None,
    })
}

/// Replace the `"Google Chrome"` entry in a `sec-ch-ua` brand list
/// with the named sibling brand, preserving every other entry's
/// exact text — GREASE placeholder token (`"Not.A/Brand";v="8"` vs
/// `"Not_A Brand";v="24"` etc.), slot ordering, and spacing.
///
/// Debug-asserts that the input actually contained `"Google Chrome"`;
/// a silent no-op is a category of bug we'd rather surface in tests
/// than ship to production as stock-Chrome headers wearing a sibling
/// `Session`.
pub(crate) fn swap_brand(chrome_sec_ch_ua: &str, new_brand: &str) -> String {
    const CHROME_NAME: &str = r#""Google Chrome""#;
    let mut swapped = false;
    let result = chrome_sec_ch_ua
        .split(',')
        .map(|entry| {
            // ASCII whitespace only — HTTP header values are ASCII.
            let ws_len = entry
                .bytes()
                .take_while(|b| b.is_ascii_whitespace())
                .count();
            let (lead, rest) = entry.split_at(ws_len);
            if let Some(tail) = rest.strip_prefix(CHROME_NAME) {
                swapped = true;
                format!("{lead}\"{new_brand}\"{tail}")
            } else {
                entry.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    debug_assert!(
        swapped,
        "swap_brand called with sec-ch-ua that does not contain `\"Google Chrome\"`: \
         {chrome_sec_ch_ua:?}",
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- swap_brand format snapshots across Chromium versions ----
    // These assertions live in the same file as the source, so they
    // catch template-edit typos, not wire-level truth. The
    // wire-level truth is in session.rs integration tests.

    #[test]
    fn swap_brand_preserves_chrome147_grease_form() {
        let chrome = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
        assert_eq!(
            swap_brand(chrome, "Microsoft Edge"),
            r#""Microsoft Edge";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#
        );
    }

    #[test]
    fn swap_brand_preserves_chrome145_grease_form() {
        let chrome = r#""Google Chrome";v="145", "Not_A Brand";v="24", "Chromium";v="145""#;
        assert_eq!(
            swap_brand(chrome, "Microsoft Edge"),
            r#""Microsoft Edge";v="145", "Not_A Brand";v="24", "Chromium";v="145""#
        );
    }

    #[test]
    fn swap_brand_preserves_chrome146_slot_order() {
        let chrome = r#""Google Chrome";v="146", "Chromium";v="146", "Not_A Brand";v="24""#;
        assert_eq!(
            swap_brand(chrome, "Brave"),
            r#""Brave";v="146", "Chromium";v="146", "Not_A Brand";v="24""#
        );
    }

    // ---- Overlay gating ----

    #[test]
    fn chrome_brand_has_no_overlay() {
        let ua = "Mozilla/5.0 ...";
        let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
        assert!(ChromiumBrand::Chrome
            .overlay(147, Platform::Windows, ua, sch)
            .unwrap()
            .is_none());
    }

    #[test]
    fn edge_overlay_populates_ua_and_headers() {
        let ua = "Mozilla/5.0 ... Chrome/147.0.0.0 Safari/537.36";
        let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
        let o = ChromiumBrand::Edge
            .overlay(147, Platform::Windows, ua, sch)
            .unwrap()
            .unwrap();
        assert!(o.user_agent.ends_with(" Edg/147.0.0.0"));
        assert!(o.sec_ch_ua.contains(r#""Microsoft Edge";v="147""#));
        assert_eq!(o.extra_headers, vec![("dnt".into(), "1".into())]);
        assert!(o.navigate_accept.is_none());
    }

    #[test]
    fn brave_overlay_matches_chrome_ua() {
        let ua = "Mozilla/5.0 ... Chrome/147.0.0.0 Safari/537.36";
        let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
        let o = ChromiumBrand::Brave
            .overlay(147, Platform::Windows, ua, sch)
            .unwrap()
            .unwrap();
        assert_eq!(o.user_agent, ua);
        assert!(o.sec_ch_ua.contains(r#""Brave";v="147""#));
        assert_eq!(o.extra_headers, vec![("sec-gpc".into(), "1".into())]);
        assert!(o
            .navigate_accept
            .as_deref()
            .map(|v| !v.contains("signed-exchange"))
            .unwrap_or(false));
    }

    #[test]
    fn opera_overlay_matches_live_capture() {
        let ua = "Mozilla/5.0 ... Chrome/145.0.0.0 Safari/537.36";
        let o = ChromiumBrand::Opera
            .overlay(145, Platform::Windows, ua, "")
            .unwrap()
            .unwrap();
        assert!(o.user_agent.ends_with(" OPR/129.0.0.0"));
        assert_eq!(
            o.sec_ch_ua,
            r#""Not:A-Brand";v="99", "Opera";v="129", "Chromium";v="145""#
        );
    }

    #[test]
    fn edge_overlay_on_desktop_ok_for_all_three_platforms() {
        let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
        for p in [Platform::Windows, Platform::MacOS, Platform::Linux] {
            assert!(ChromiumBrand::Edge.overlay(147, p, "ua", sch).is_ok());
        }
    }

    #[test]
    fn edge_overlay_on_mobile_errors() {
        let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
        for p in [Platform::Android, Platform::IOS] {
            let err = ChromiumBrand::Edge.overlay(147, p, "ua", sch).unwrap_err();
            assert!(matches!(err, BrandOverlayError::Unverified { .. }));
        }
    }

    #[test]
    fn opera_overlay_only_accepts_chrome_145_desktop() {
        assert!(ChromiumBrand::Opera
            .overlay(145, Platform::Windows, "ua", "")
            .is_ok());
        for bad in [146u32, 147, 144] {
            assert!(ChromiumBrand::Opera
                .overlay(bad, Platform::Windows, "ua", "")
                .is_err());
        }
        for p in [Platform::Android, Platform::IOS] {
            assert!(ChromiumBrand::Opera.overlay(145, p, "ua", "").is_err());
        }
    }

    #[test]
    fn error_display_uses_platform_display_not_debug() {
        let err = BrandOverlayError::Unverified {
            brand: ChromiumBrand::Opera,
            chromium_major: 147,
            platform: Platform::MacOS,
        };
        let msg = format!("{err}");
        // `Platform::Display` is `"macOS"` (the sec-ch-ua form);
        // `Debug` would be `"MacOS"`. We want the former.
        assert!(
            msg.contains("/ macOS /") || msg.contains("/ macOS"),
            "{msg}"
        );
        assert!(!msg.contains("MacOS"), "{msg}");
    }
}
