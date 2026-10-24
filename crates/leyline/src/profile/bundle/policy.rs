use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use http::HeaderValue;

use crate::profile::H3Profile;
use crate::profile::registry::{ProfileError, ProfileRegistry};
use crate::profile::types::{BrowserProfile, H2Profile, PlatformIdentity, TlsProfile};
use crate::tls::builder::{TlsMinVersion, profile_min_version};

use super::OFFICIAL;

type Lists<'a> = [(&'static str, Vec<&'a str>); 7];

static FLOOR: LazyLock<TlsMinVersion> = LazyLock::new(|| {
    profile_min_version(&Some(OFFICIAL.policy.min_tls_version.clone()))
        .ok()
        .flatten()
        .expect("built-in bundle policy names a supported TLS version")
});

type Known<'a> = HashMap<(&'a str, &'static str), HashSet<&'a str>>;

static BUILTIN: LazyLock<Known<'static>> = LazyLock::new(|| {
    let mut known = Known::new();
    for profile in ProfileRegistry::global().iter() {
        if tls_blocks(profile).any(|tls| below_floor(tls).unwrap_or(true)) {
            continue;
        }
        for tls in tls_blocks(profile) {
            for (field, names) in lists(tls) {
                known
                    .entry((profile.meta.browser.as_str(), field))
                    .or_default()
                    .extend(names);
            }
        }
    }
    known
});

pub(super) fn check(
    browser: &str,
    version: u32,
    profile: &BrowserProfile,
) -> Result<(), ProfileError> {
    if (browser, version) != (profile.meta.browser.as_str(), profile.meta.version) {
        return Err(violation(format!(
            "bundle entry {browser} {version} holds the profile {} {}",
            profile.meta.browser, profile.meta.version
        )));
    }
    let H2Profile {
        header_table_size: _,
        enable_push: _,
        max_concurrent_streams: _,
        initial_stream_window_size: _,
        initial_connection_window_size: _,
        max_frame_size: _,
        max_header_list_size: _,
        unknown_setting8: _,
        unknown_setting9: _,
        pseudo_order: _,
        settings_order: _,
        default_priority: _,
        fingerprint: _,
        platforms: _,
    } = &profile.h2;
    if let Some(H3Profile {
        initial_max_data: _,
        initial_max_stream_data_bidi_local: _,
        initial_max_stream_data_bidi_remote: _,
        initial_max_stream_data_uni: _,
        initial_max_streams_bidi: _,
        initial_max_streams_uni: _,
        max_idle_timeout_secs: _,
        max_udp_payload_size: _,
        initial_datagram_size: _,
        initial_crypto_split: _,
        initial_crypto_reorder: _,
        active_connection_id_limit: _,
        dcid_length: _,
        scid_length: _,
        qpack_max_table_capacity: _,
        qpack_blocked_streams: _,
        max_field_section_size: _,
        race: _,
        max_ack_delay_ms: _,
        transport_parameters: _,
        transport_order: _,
        settings: _,
        control_grease_frame: _,
        pseudo_order: _,
        priority_update: _,
        tls: _,
    }) = &profile.h3
    {}
    for tls in tls_blocks(profile) {
        check_tls(&profile.meta.browser, tls)?;
    }
    for (platform, identity) in &profile.identity {
        let PlatformIdentity {
            user_agent,
            sec_ch_ua,
            accept_language,
        } = identity;
        let values = [
            ("user_agent", Some(user_agent)),
            ("sec_ch_ua", Some(sec_ch_ua)),
            ("accept_language", accept_language.as_ref()),
        ];
        for (field, value) in values {
            if value.is_some_and(|value| HeaderValue::from_str(value).is_err()) {
                return Err(violation(format!(
                    "identity.{platform}.{field} is not a valid header value"
                )));
            }
        }
    }
    Ok(())
}

fn check_tls(browser: &str, tls: &TlsProfile) -> Result<(), ProfileError> {
    if below_floor(tls).map_err(violation)? {
        return Err(violation(format!(
            "min_tls_version {:?} is below the bundle floor {:?}",
            tls.min_tls_version.as_deref().unwrap_or_default(),
            *FLOOR
        )));
    }
    let builtin: &Known<'_> = &BUILTIN;
    let empty = HashSet::new();
    for (field, names) in lists(tls) {
        let known = builtin.get(&(browser, field)).unwrap_or(&empty);
        let names: HashSet<&str> = names.into_iter().collect();
        if let Some(name) = names.difference(known).next() {
            return Err(violation(format!(
                "{field} entry {name:?} is not in any built-in {browser} profile at or above the floor"
            )));
        }
    }
    Ok(())
}

fn below_floor(tls: &TlsProfile) -> Result<bool, String> {
    let declared = profile_min_version(&tls.min_tls_version).map_err(|error| error.to_string())?;
    Ok(declared.is_some_and(|min| min < *FLOOR))
}

fn tls_blocks(profile: &BrowserProfile) -> impl Iterator<Item = &TlsProfile> {
    std::iter::once(&profile.tls).chain(profile.h3.as_ref().and_then(|h3| h3.tls.as_ref()))
}

fn lists(tls: &TlsProfile) -> Lists<'_> {
    let TlsProfile {
        ciphers,
        curves,
        key_shares,
        sigalgs,
        delegated_credentials,
        record_size_limit: _,
        alps,
        alps_new_codepoint: _,
        ocsp_stapling: _,
        signed_cert_timestamps: _,
        cert_compression,
        permute_extensions: _,
        extension_permutation: _,
        extension_tail: _,
        grease: _,
        sigalg_grease: _,
        ech_grease: _,
        pre_shared_key: _,
        session_tickets: _,
        request_trust_anchors: _,
        fingerprint: _,
        padding: _,
        min_tls_version: _,
        tls12_extensions: _,
    } = tls;
    [
        ("ciphers", strings(ciphers)),
        ("curves", strings(curves)),
        (
            "key_shares",
            strings(key_shares.as_deref().unwrap_or_default()),
        ),
        ("sigalgs", strings(sigalgs)),
        (
            "delegated_credentials",
            delegated_credentials
                .iter()
                .flat_map(|algorithms| algorithms.split(':'))
                .collect(),
        ),
        ("cert_compression", strings(cert_compression)),
        ("alps", alps.iter().map(String::as_str).collect()),
    ]
}

fn strings(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

fn violation(reason: String) -> ProfileError {
    ProfileError::Policy { reason }
}
