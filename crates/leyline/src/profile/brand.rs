//! Chromium-family identity overlays (Edge, Opera, Vivaldi).
//!
//! Edge, Opera, and Vivaldi ship the same Chromium codebase as Chrome —
//! same TLS ClientHello, same H2 SETTINGS. They differ only in HTTP
//! identity headers (`User-Agent` suffix, `sec-ch-ua` brand, a
//! privacy header or two). A [`ChromiumBrand`] is overlaid on an
//! existing Chrome profile at `Session` build time; JA4 and Akamai
//! fingerprint stay bit-for-bit Chromium.
//!
//! Brave is the exception: it diverges from Chrome on H2 SETTINGS
//! (drops `unknown_setting8`), `sec-ch-ua` slot order, GREASE form,
//! request-header order, and `accept-language` q-factor — far enough
//! that the overlay model can't represent it accurately. Brave ships
//! as the first-class [`Browser::Brave146`](super::Browser::Brave146)
//! profile instead.

use crate::profile::Platform;

/// Chromium-family browser skin applied on top of a Chrome profile.
/// Affects only HTTP identity headers — never TLS or HTTP/2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ChromiumBrand {
    /// Stock Chrome. No overlay.
    #[default]
    Chrome,
    /// Microsoft Edge. Adds `Edg/N` to UA, swaps `sec-ch-ua` brand,
    /// ships `dnt: 1`.
    Edge,
    /// Opera. Adds `OPR/N` to UA, swaps `sec-ch-ua` brand.
    Opera,
    /// Vivaldi. Adds `Vivaldi/N.M.B.P` to UA, drops `"Google Chrome"`
    /// from `sec-ch-ua` so only Chromium + GREASE remain — Vivaldi
    /// deliberately omits its own brand from `sec-ch-ua` by default
    /// (per vivaldi.com/blog/technology/client-hints-or-client-lies).
    Vivaldi,
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
    /// The brand's OWN major version on this Chromium anchor — the value the
    /// overlay encodes — for callers that need the version outside the HTTP
    /// overlay (e.g. the browser's `userAgentData`). Chrome/Edge ship in lockstep
    /// so they share the Chromium major; Opera versions separately (the
    /// `OPERA_PER_CHROMIUM` registry); Vivaldi is versioned by build string, not a
    /// bare major, so it has no answer here. `None` = no verified version for this
    /// anchor (same gate as `overlay`). This is THE source of truth — callers must
    /// not hardcode a parallel value.
    pub fn version_for(self, chromium_major: u32) -> Option<u32> {
        match self {
            Self::Chrome | Self::Edge => Some(chromium_major),
            Self::Opera => OPERA_PER_CHROMIUM
                .iter()
                .find_map(|(chromium, opera)| (*chromium == chromium_major).then_some(*opera)),
            Self::Vivaldi => None,
        }
    }

    /// Human-readable label, e.g. for logs.
    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Opera => "Opera",
            Self::Vivaldi => "Vivaldi",
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
            Self::Opera => opera_overlay(chromium_major, platform, profile_user_agent).map(Some),
            Self::Vivaldi => vivaldi_overlay(
                chromium_major,
                platform,
                profile_user_agent,
                profile_sec_ch_ua,
            )
            .map(Some),
        }
    }
}

/// Concrete header overrides applied on top of the active Chromium
/// profile. All fields are already computed against the active
/// profile — the caller copies them in without further transformation.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
        // Resolve to the concrete host OS; brand overlays only run for an
        // explicit Chromium browser, where the platform is already resolved,
        // so this is a belt-and-suspenders arm rather than a live path.
        Platform::Host => desktop_only(Platform::detect_host()),
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
    // UA-reduced `Edg/{major}.0.0.0` form. Verified against tls.peet.ws
    // 2026-04-25 with Microsoft Edge 147 on macOS — Microsoft ships the
    // same reduced format Chrome adopted in 2022. Seeding a fake build
    // version from a hardcoded table would be a real fingerprint mismatch.
    Ok(BrandOverlay {
        user_agent: format!("{profile_user_agent} Edg/{chromium_major}.0.0.0"),
        sec_ch_ua: swap_brand(profile_sec_ch_ua, "Microsoft Edge"),
        extra_headers: vec![("dnt".into(), "1".into())],
        navigate_accept: None,
    })
}

/// Opera Stable major version per Chromium anchor. Opera Stable numbers its
/// major a fixed 16 below the Chromium it rebases on (149→133, 148→132, 147→131,
/// 146→130, 145→129) and ships on a contemporaneous cadence; entries here pair
/// each supported Chromium anchor with the Opera Stable major that shipped on
/// it.
///
/// The 149/133 and 148/132 pairings are sourced from Opera's official desktop
/// release blog (Opera 133 Stable on Chromium 149.0.7827.201; Opera 132 Stable
/// on Chromium 148.0.7778.97); their sec-ch-ua wire shape reuses the verified
/// 129 template. Anchors marked EXTRAPOLATED are derived from vendor release notes,
/// structurally identical to theÌ5/129 live
/// capture.
const OPERA_PER_CHROMIUM: &[(u32, u32)] = &[
    (149, 133), // Opera 133 Stable = Chromium 149.0.7827.201 — blogs.opera.com/desktop, 2026.
    (148, 132), // Opera 132 Stable = Chromium 148.0.7778.97 — blogs.opera.com/desktop, 2026.
    (147, 131), // EXTRAPOLATED from vendor release notes.
    (146, 130), // EXTRAPOLATED from vendor release notes.
    (145, 129), // Live capture.
];

fn opera_overlay(
    chromium_major: u32,
    platform: Platform,
    profile_user_agent: &str,
) -> Result<BrandOverlay, BrandOverlayError> {
    // Opera's sec-ch-ua uses a different GREASE placeholder and slot
    // order than Chrome's, so we can't derive it from the profile by
    // a simple swap. We only emit an overlay for pairs we've verified
    // (live capture or vendor-doc extrapolation from a verified shape).
    if !desktop_only(platform) {
        return Err(BrandOverlayError::Unverified {
            brand: ChromiumBrand::Opera,
            chromium_major,
            platform,
        });
    }
    let opera_version = OPERA_PER_CHROMIUM
        .iter()
        .find_map(|(chromium, opera)| (*chromium == chromium_major).then_some(*opera))
        .ok_or(BrandOverlayError::Unverified {
            brand: ChromiumBrand::Opera,
            chromium_major,
            platform,
        })?;
    Ok(BrandOverlay {
        user_agent: format!("{profile_user_agent} OPR/{opera_version}.0.0.0"),
        sec_ch_ua: format!(
            r#""Not:A-Brand";v="99", "Opera";v="{opera_version}", "Chromium";v="{chromium_major}""#
        ),
        extra_headers: Vec::new(),
        navigate_accept: None,
    })
}

/// Real Vivaldi build versions per Chromium major. Sourced from
/// vendor release notes. Vivaldi tracks the Chrome major closely on Stable.
const VIVALDI_BUILDS_PER_MAJOR: &[(u32, &[&str])] = &[
    // EXTRAPOLATED from vendor release notes.
    (147, &["7.9.3970.59"]),
];

fn vivaldi_build_for(chromium_major: u32, fallback_seed: u64) -> Option<&'static str> {
    for (major, builds) in VIVALDI_BUILDS_PER_MAJOR {
        if *major == chromium_major {
            return Some(builds[(fallback_seed as usize) % builds.len()]);
        }
    }
    None
}

fn vivaldi_overlay(
    chromium_major: u32,
    platform: Platform,
    profile_user_agent: &str,
    profile_sec_ch_ua: &str,
) -> Result<BrandOverlay, BrandOverlayError> {
    if !desktop_only(platform) {
        return Err(BrandOverlayError::Unverified {
            brand: ChromiumBrand::Vivaldi,
            chromium_major,
            platform,
        });
    }
    let seed = profile_user_agent.bytes().fold(0u64, |acc, b| {
        acc.wrapping_mul(1099511628211).wrapping_add(b as u64)
    });
    let build = vivaldi_build_for(chromium_major, seed).ok_or(BrandOverlayError::Unverified {
        brand: ChromiumBrand::Vivaldi,
        chromium_major,
        platform,
    })?;
    Ok(BrandOverlay {
        user_agent: format!("{profile_user_agent} Vivaldi/{build}"),
        sec_ch_ua: drop_brand(profile_sec_ch_ua, "Google Chrome"),
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

/// Remove the named brand entry from a `sec-ch-ua` brand list,
/// preserving every other entry's exact text and slot ordering.
/// Used by the Vivaldi overlay, which strips `"Google Chrome"` and
/// leaves only Chromium + the GREASE placeholder — Vivaldi's
/// documented default.
///
/// Debug-asserts that the named brand was actually present so a
/// silent no-op (which would ship as stock Chrome's brand list)
/// surfaces in tests.
pub(crate) fn drop_brand(chrome_sec_ch_ua: &str, brand_name: &str) -> String {
    let needle = format!("\"{brand_name}\"");
    let mut dropped = false;
    let result = chrome_sec_ch_ua
        .split(',')
        .filter(|entry| {
            let trimmed = entry.trim_start();
            if trimmed.starts_with(&needle) {
                dropped = true;
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    debug_assert!(
        dropped,
        "drop_brand called with sec-ch-ua that does not contain {needle}: {chrome_sec_ch_ua:?}",
    );
    // After filtering, the first surviving entry may carry a leading
    // space inherited from the comma-and-space separator. Re-normalise
    // to the canonical `, ` join shape.
    result.trim_start().to_string()
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
    fn drop_brand_removes_chrome_entry_only() {
        let chrome = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
        assert_eq!(
            drop_brand(chrome, "Google Chrome"),
            r#""Not.A/Brand";v="8", "Chromium";v="147""#
        );
    }

    #[test]
    fn drop_brand_preserves_remaining_slot_order() {
        // Different chrome major shuffles slot order; dropping must
        // not re-sort the survivors.
        let chrome = r#""Not_A Brand";v="24", "Chromium";v="146", "Google Chrome";v="146""#;
        assert_eq!(
            drop_brand(chrome, "Google Chrome"),
            r#""Not_A Brand";v="24", "Chromium";v="146""#
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
        assert!(
            ChromiumBrand::Chrome
                .overlay(147, Platform::Windows, ua, sch)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn edge_overlay_populates_ua_and_headers() {
        let ua = "Mozilla/5.0 ... Chrome/147.0.0.0 Safari/537.36";
        let sch = r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#;
        let o = ChromiumBrand::Edge
            .overlay(147, Platform::Windows, ua, sch)
            .unwrap()
            .unwrap();
        // Real Edge ships the UA-reduced `Edg/{major}.0.0.0` form on
        // current versions. Verified against tls.peet.ws on 2026-04-25
        // with Edge 147 on macOS.
        assert!(
            o.user_agent.ends_with(" Edg/147.0.0.0"),
            "Edge UA must end with reduced Edg/{{major}}.0.0.0: {}",
            o.user_agent
        );
        assert!(o.sec_ch_ua.contains(r#""Microsoft Edge";v="147""#));
        assert_eq!(o.extra_headers, vec![("dnt".into(), "1".into())]);
        assert!(o.navigate_accept.is_none());
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
    fn opera_overlay_only_accepts_verified_anchors() {
        // OPERA_PER_CHROMIUM is the source of truth — 145/146/147/148/149 each
        // yield a distinct Opera version, anything outside the table
        // errors. Mobile platforms always error regardless of anchor.
        for (chromium, expected_opera) in [
            (145u32, 129u32),
            (146, 130),
            (147, 131),
            (148, 132),
            (149, 133),
        ] {
            let o = ChromiumBrand::Opera
                .overlay(chromium, Platform::Windows, "ua", "")
                .unwrap()
                .unwrap();
            assert!(
                o.user_agent
                    .ends_with(&format!(" OPR/{expected_opera}.0.0.0")),
                "Chromium {chromium} should map to OPR/{expected_opera}: {}",
                o.user_agent
            );
        }
        for bad in [144u32, 150] {
            assert!(
                ChromiumBrand::Opera
                    .overlay(bad, Platform::Windows, "ua", "")
                    .is_err()
            );
        }
        for p in [Platform::Android, Platform::IOS] {
            assert!(ChromiumBrand::Opera.overlay(147, p, "ua", "").is_err());
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
