use leyline_bssl::ssl::{SslContextBuilder, SslOptions, SslVersion};

use crate::iana::{boring_curve_name, sigalg_id};
use crate::profile::{CertCompression, TlsMinVersion, TlsProfile};
use crate::tls::error::TlsError;

#[cfg(feature = "compression-brotli")]
use super::cert::Brotli;
#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
use super::cert::Zlib;
#[cfg(feature = "compression-zstd")]
use super::cert::Zstd;
#[cfg(leyline_any_codec)]
use super::cert::add_decompressor;
#[cfg(not(all(
    feature = "compression-brotli",
    feature = "compression-zstd",
    any(feature = "compression-gzip", feature = "compression-deflate")
)))]
use super::missing;
use super::{Transport, partition_ciphers};

pub(super) fn ciphers(builder: &mut SslContextBuilder, tls: &TlsProfile) -> Result<(), TlsError> {
    let (tls13, tls12) = partition_ciphers(&tls.ciphers)?;
    let list = tls12.join(":");
    if !list.is_empty() {
        builder
            .set_cipher_list(&list)
            .map_err(TlsError::from_stack)?;
    }
    exact::tls13_cipher_order(builder, tls, &tls13)
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
    builder
        .set_sigalgs(&exact::sigalg_list(ids))
        .map_err(TlsError::from_stack)
}

pub(super) fn compression(
    builder: &mut SslContextBuilder,
    tls: &TlsProfile,
) -> Result<(), TlsError> {
    for algo in tls.cert_compressions() {
        match algo.map_err(TlsError::Profile)? {
            CertCompression::Brotli => add_brotli(builder)?,
            CertCompression::Zlib => add_zlib(builder)?,
            CertCompression::Zstd => add_zstd(builder)?,
        }
    }
    Ok(())
}

#[cfg(feature = "compression-brotli")]
fn add_brotli(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    add_decompressor(builder, Brotli)
}

#[cfg(not(feature = "compression-brotli"))]
fn add_brotli(_builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    Err(missing("brotli", crate::profile::Encoding::Brotli))
}

#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
fn add_zlib(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    add_decompressor(builder, Zlib)
}

#[cfg(not(any(feature = "compression-gzip", feature = "compression-deflate")))]
fn add_zlib(_builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    Err(missing("zlib", crate::profile::Encoding::Deflate))
}

#[cfg(feature = "compression-zstd")]
fn add_zstd(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    add_decompressor(builder, Zstd)
}

#[cfg(not(feature = "compression-zstd"))]
fn add_zstd(_builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    Err(missing("zstd", crate::profile::Encoding::Zstd))
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

    exact::extension_order(builder, tls)?;

    builder.set_grease_enabled(tls.grease);
    builder.set_grease_sigalgs_enabled(tls.grease && tls.sigalg_grease);

    if !tls.session_tickets {
        builder.set_options(SslOptions::NO_TICKET);
    }
    Ok(())
}

#[cfg(any(
    target_os = "linux",
    target_os = "windows",
    all(target_arch = "aarch64", target_os = "macos")
))]
mod exact {
    use leyline_bssl::ssl::SslContextBuilder;

    use crate::profile::TlsProfile;
    use crate::tls::error::TlsError;

    pub(super) fn tls13_cipher_order(
        builder: &mut SslContextBuilder,
        _tls: &TlsProfile,
        tls13: &[u16],
    ) -> Result<(), TlsError> {
        if !tls13.is_empty() {
            builder
                .set_tls13_cipher_order(tls13)
                .map_err(TlsError::from_stack)?;
        }
        Ok(())
    }

    pub(super) fn sigalg_list(ids: Vec<u16>) -> Vec<u16> {
        ids
    }

    pub(super) fn extension_order(
        builder: &mut SslContextBuilder,
        tls: &TlsProfile,
    ) -> Result<(), TlsError> {
        if let Some(order) = &tls.extension_permutation {
            builder
                .set_extension_order(order)
                .map_err(TlsError::from_stack)?;
        }
        Ok(())
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "windows",
    all(target_arch = "aarch64", target_os = "macos")
)))]
mod exact {
    use leyline_bssl::ssl::SslContextBuilder;

    use crate::profile::TlsProfile;
    use crate::tls::error::TlsError;

    pub(super) fn tls13_cipher_order(
        _builder: &mut SslContextBuilder,
        tls: &TlsProfile,
        tls13: &[u16],
    ) -> Result<(), TlsError> {
        if !tls13.is_empty() && tls.extension_permutation.is_some() {
            return Err(TlsError::Profile(
                "exact TLS order requires a rebuilt BoringSSL bundle for this target".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn sigalg_list(ids: Vec<u16>) -> Vec<u16> {
        let mut unique = Vec::with_capacity(ids.len());
        for id in ids {
            if !unique.contains(&id) {
                unique.push(id);
            }
        }
        unique
    }

    pub(super) fn extension_order(
        _builder: &mut SslContextBuilder,
        tls: &TlsProfile,
    ) -> Result<(), TlsError> {
        if tls.extension_permutation.is_some() {
            return Err(TlsError::Profile(
                "exact TLS extension order requires a rebuilt BoringSSL bundle for this target"
                    .into(),
            ));
        }
        Ok(())
    }
}

pub(super) fn versions(
    builder: &mut SslContextBuilder,
    tls: &TlsProfile,
    transport: Transport,
    floor: TlsMinVersion,
) -> Result<(), TlsError> {
    let base = match transport {
        Transport::Quic => TlsMinVersion::Tls13,
        Transport::Tcp(default_min) => tls
            .min_version()
            .map_err(TlsError::Profile)?
            .unwrap_or(default_min),
    };
    let min = match base.max(floor) {
        TlsMinVersion::Tls10 => SslVersion::TLS1,
        TlsMinVersion::Tls12 => SslVersion::TLS1_2,
        TlsMinVersion::Tls13 => SslVersion::TLS1_3,
    };
    builder
        .set_min_proto_version(Some(min))
        .map_err(TlsError::from_stack)?;
    Ok(())
}
