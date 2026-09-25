use leyline_bssl::ssl::{SslContextBuilder, SslOptions, SslVersion};

use crate::iana::{boring_curve_name, sigalg_id};
use crate::profile::TlsProfile;
use crate::tls::error::TlsError;

#[cfg(feature = "compression-brotli")]
use super::BrotliDecompressor;
#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
use super::ZlibDecompressor;
#[cfg(feature = "compression-zstd")]
use super::ZstdDecompressor;
#[cfg(not(all(
    feature = "compression-brotli",
    feature = "compression-zstd",
    any(feature = "compression-gzip", feature = "compression-deflate")
)))]
use super::missing;
use super::{TlsMinVersion, profile_min_version, tls13_cipher_ids};

pub(super) fn ciphers(builder: &mut SslContextBuilder, tls: &TlsProfile) -> Result<(), TlsError> {
    let tls13 = tls13_cipher_ids(&tls.ciphers)?;
    let list = tls
        .ciphers
        .iter()
        .filter(|name| {
            crate::iana::cipher_id(name).is_none_or(|id| !crate::iana::is_tls13_cipher(id))
        })
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(":");
    if !list.is_empty() {
        builder
            .set_cipher_list(&list)
            .map_err(TlsError::from_stack)?;
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "windows",
        all(target_arch = "aarch64", target_os = "macos")
    ))]
    if !tls13.is_empty() {
        builder
            .set_tls13_cipher_order(&tls13)
            .map_err(TlsError::from_stack)?;
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "windows",
        all(target_arch = "aarch64", target_os = "macos")
    )))]
    if !tls13.is_empty() && tls.extension_permutation.is_some() {
        return Err(TlsError::Profile(
            "exact TLS order requires a rebuilt BoringSSL bundle for this target".into(),
        ));
    }
    Ok(())
}

pub(super) fn curves(builder: &mut SslContextBuilder, tls: &TlsProfile) -> Result<(), TlsError> {
    let list = tls
        .curves
        .iter()
        .map(|c| boring_curve_name(c))
        .collect::<Vec<_>>()
        .join(":");
    builder
        .set_curves_list(&list)
        .map_err(TlsError::from_stack)?;
    Ok(())
}

pub(super) fn sigalgs(builder: &mut SslContextBuilder, tls: &TlsProfile) -> Result<(), TlsError> {
    let ids = tls
        .sigalgs
        .iter()
        .map(|name| {
            sigalg_id(name)
                .ok_or_else(|| TlsError::Profile(format!("unknown signature algorithm: {name}")))
        })
        .collect::<Result<Vec<u16>, _>>()?;
    if let Err(e) = builder.set_sigalgs(&ids) {
        if !e.to_string().contains("DUPLICATE_SIGNATURE_ALGORITHM") {
            return Err(TlsError::SslConfig(e.to_string()));
        }
        let mut deduped = ids.clone();
        deduped.dedup();
        builder
            .set_sigalgs(&deduped)
            .map_err(TlsError::from_stack)?;
    }
    Ok(())
}

pub(super) fn compression(
    builder: &mut SslContextBuilder,
    tls: &TlsProfile,
) -> Result<(), TlsError> {
    for algo in &tls.cert_compression {
        match algo.as_str() {
            "brotli" => add_brotli(builder)?,
            "zlib" => add_zlib(builder)?,
            "zstd" => add_zstd(builder)?,
            other => {
                return Err(TlsError::Profile(format!(
                    "unknown cert compression algorithm: {other:?}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(feature = "compression-brotli")]
fn add_brotli(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    builder
        .add_certificate_compression_algorithm(BrotliDecompressor)
        .map_err(TlsError::from_stack)
}

#[cfg(not(feature = "compression-brotli"))]
fn add_brotli(_builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    Err(missing("brotli", "compression-brotli"))
}

#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
fn add_zlib(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    builder
        .add_certificate_compression_algorithm(ZlibDecompressor)
        .map_err(TlsError::from_stack)
}

#[cfg(not(any(feature = "compression-gzip", feature = "compression-deflate")))]
fn add_zlib(_builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    Err(missing("zlib", "compression-deflate"))
}

#[cfg(feature = "compression-zstd")]
fn add_zstd(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    builder
        .add_certificate_compression_algorithm(ZstdDecompressor)
        .map_err(TlsError::from_stack)
}

#[cfg(not(feature = "compression-zstd"))]
fn add_zstd(_builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    Err(missing("zstd", "compression-zstd"))
}

pub(super) fn extensions(
    builder: &mut SslContextBuilder,
    tls: &TlsProfile,
) -> Result<(), TlsError> {
    if tls.ocsp_stapling {
        builder.enable_ocsp_stapling();
    }

    if tls.signed_cert_timestamps {
        builder.enable_signed_cert_timestamps();
    }

    compression(builder, tls)?;

    if let Some(ref dc) = tls.delegated_credentials {
        builder
            .set_delegated_credentials(dc)
            .map_err(TlsError::from_stack)?;
    }

    if let Some(limit) = tls.record_size_limit {
        builder.set_record_size_limit(limit);
    }

    if tls.permute_extensions {
        builder.set_permute_extensions(true);
    }
    if !tls.extension_tail.is_empty() {
        builder
            .set_extension_tail(&tls.extension_tail)
            .map_err(TlsError::from_stack)?;
    }

    order(builder, tls)?;

    builder.set_grease_enabled(tls.grease);
    builder.set_grease_signature_algorithms(tls.grease && tls.sigalg_grease);

    if !tls.session_tickets {
        builder.set_options(SslOptions::NO_TICKET);
    }
    Ok(())
}

fn order(builder: &mut SslContextBuilder, tls: &TlsProfile) -> Result<(), TlsError> {
    #[cfg(any(
        target_os = "linux",
        target_os = "windows",
        all(target_arch = "aarch64", target_os = "macos")
    ))]
    if let Some(order) = &tls.extension_permutation {
        builder
            .set_extension_order(order)
            .map_err(TlsError::from_stack)?;
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "windows",
        all(target_arch = "aarch64", target_os = "macos")
    )))]
    if tls.extension_permutation.is_some() {
        return Err(TlsError::Profile(
            "exact TLS extension order requires a rebuilt BoringSSL bundle for this target".into(),
        ));
    }
    Ok(())
}

pub(super) fn versions(
    builder: &mut SslContextBuilder,
    tls: &TlsProfile,
    floor: TlsMinVersion,
) -> Result<(), TlsError> {
    let min = if floor == TlsMinVersion::Tls13 {
        TlsMinVersion::Tls13
    } else {
        match profile_min_version(&tls.min_tls_version)? {
            Some(declared) => declared,
            None => floor,
        }
    };
    let min = match min {
        TlsMinVersion::Tls10 => SslVersion::TLS1,
        TlsMinVersion::Tls12 => SslVersion::TLS1_2,
        TlsMinVersion::Tls13 => SslVersion::TLS1_3,
    };
    builder
        .set_min_proto_version(Some(min))
        .map_err(TlsError::from_stack)?;
    Ok(())
}
