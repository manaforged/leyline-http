//! Fingerprint trust map — where each self-reported fingerprint dimension sits relative to truth, offline and deterministic.

use leyline::Platform;
use leyline::audit::{
    Ja3Input, Ja4Input, chrome_extension_ids, compute_ja3, compute_ja4, compute_ja4t,
};
use leyline::h2::H2Config;
use leyline::profile::{ALL_BROWSERS, ProfileRegistry};

#[derive(PartialEq, Clone, Copy)]
enum Status {
    /// Wire-faithful + golden match.
    Gated,
    /// Wire-faithful golden MISMATCH → regression, hard fail.
    GatedFail,
    /// Reconstruction happens to match the wire golden.
    ReconAccurate,
    /// Reconstruction diverges from the wire golden (audit() not wire-exact here).
    ReconDiverges,
    /// No golden / not offline-wire-verifiable.
    Unanchored,
}

impl Status {
    fn tag(self) -> &'static str {
        match self {
            Status::Gated => "GATED ok",
            Status::GatedFail => "GATED FAIL (regression)",
            Status::ReconAccurate => "recon: matches wire",
            Status::ReconDiverges => "recon: diverges from wire",
            Status::Unanchored => "unanchored (no golden)",
        }
    }
}

struct Row {
    profile: String,
    dimension: String,
    status: Status,
    detail: String,
}

impl Row {
    fn new(profile: &str, dimension: &str, status: Status, detail: String) -> Self {
        Self {
            profile: profile.to_string(),
            dimension: dimension.to_string(),
            status,
            detail,
        }
    }
}

#[test]
fn fingerprint_conformance() {
    let reg = ProfileRegistry::builtin();
    let mut rows: Vec<Row> = Vec::new();

    for browser in ALL_BROWSERS {
        let p = reg.get_browser(browser).expect("built-in profile");
        let name = browser.to_string();

        let h2 = H2Config::from_profile(&p.h2).expect("valid built-in h2");
        rows.push(gated(
            &name,
            "Akamai-H2",
            &h2.akamai_fingerprint(),
            p.expected_h2_fingerprint(),
        ));
        for plat in ALL_PLATFORMS {
            if p.h2.platforms.contains_key(plat.identity_key()) {
                let resolved = p.h2.resolve_for_platform(plat).expect("resolve");
                let fp = H2Config::from_profile(&resolved)
                    .expect("valid override")
                    .akamai_fingerprint();
                rows.push(gated(
                    &name,
                    &format!("Akamai-H2 [{}]", plat.identity_key()),
                    &fp,
                    p.expected_h2_fingerprint_for(plat),
                ));
            }
        }

        let ext = chrome_extension_ids(&p.tls);
        let ja4 = compute_ja4(&Ja4Input {
            ciphers: &p.tls.ciphers,
            sigalgs: &p.tls.sigalgs,
            curves: &p.tls.curves,
            extension_ids: &ext,
            tls_version: "1.3",
            has_sni: true,
            alpn: "h2",
        });
        rows.push(recon(&name, "JA4 (audit)", &ja4, p.expected_ja4()));

        let ja3 = compute_ja3(&Ja3Input {
            ciphers: &p.tls.ciphers,
            curves: &p.tls.curves,
            extension_ids: &ext,
            tls_record_version: 771,
        });
        rows.push(Row::new(
            &name,
            "JA3 (audit)",
            Status::Unanchored,
            format!("reconstruction {ja3}; no JA3 golden in any TOML"),
        ));

        let tcp = Platform::Windows.tcp_profile();
        let ja4t = compute_ja4t(
            tcp.window_size,
            tcp.mss as u16,
            tcp.window_scale as u8,
            true,
        );
        rows.push(Row::new(
            &name,
            "JA4T (TCP)",
            Status::Unanchored,
            format!("computed {ja4t}; no JA4T golden, not offline-wire-verifiable"),
        ));

        let cc = &p.tls.cert_compression;
        let applied = cc.iter().filter(|a| a.as_str() == "brotli").count();
        rows.push(Row::new(
            &name,
            "cert-compression",
            Status::Unanchored,
            format!(
                "advertises {cc:?}; apply_profile applies brotli only ({applied}/{})",
                cc.len()
            ),
        ));

        #[cfg(feature = "http3")]
        {
            let fam = &p.meta.family;
            let detail = match leyline::H3Config::for_family(fam) {
                Ok(_) => format!("family {fam:?} → H3Config; no QUIC-capture golden"),
                Err(_) => format!("family {fam:?} → no H3 config (HTTP/3 unsupported)"),
            };
            rows.push(Row::new(&name, "H3 transport", Status::Unanchored, detail));
        }
    }

    let report = render(&rows);
    println!("{report}");
    let path = format!("{}/fingerprint-conformance.md", env!("CARGO_TARGET_TMPDIR"));
    let _ = std::fs::write(&path, &report);
    println!("(full map also written to {path})");

    let regressions = rows
        .iter()
        .filter(|r| r.status == Status::GatedFail)
        .count();
    assert_eq!(
        regressions, 0,
        "{regressions} GATED (wire-faithful) fingerprint regression(s) — see map above"
    );
}

const ALL_PLATFORMS: [Platform; 5] = [
    Platform::Windows,
    Platform::MacOS,
    Platform::Linux,
    Platform::Android,
    Platform::IOS,
];

/// A wire-faithful dimension compared to its golden (gated).
fn gated(profile: &str, dim: &str, got: &str, golden: Option<&str>) -> Row {
    match golden {
        Some(g) if g == got => Row::new(profile, dim, Status::Gated, g.to_string()),
        Some(g) => Row::new(
            profile,
            dim,
            Status::GatedFail,
            format!("golden={g}  got={got}"),
        ),
        None => Row::new(
            profile,
            dim,
            Status::Unanchored,
            format!("emitted {got}; no golden"),
        ),
    }
}

/// A reconstruction compared to the wire golden (reported, never gated).
fn recon(profile: &str, dim: &str, got: &str, golden: Option<&str>) -> Row {
    match golden {
        Some(g) if g == got => Row::new(profile, dim, Status::ReconAccurate, g.to_string()),
        Some(g) => Row::new(
            profile,
            dim,
            Status::ReconDiverges,
            format!("wire golden={g}  audit got={got}"),
        ),
        None => Row::new(
            profile,
            dim,
            Status::Unanchored,
            format!("reconstruction {got}; no golden"),
        ),
    }
}

/// Render the map as markdown.
fn render(rows: &[Row]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let mut cur = "";
    let _ = writeln!(s, "\n# Fingerprint trust map (offline)\n");
    let _ = writeln!(
        s,
        "Wire truth = the live `tls_peet` suite. This shows trust status, not wire correctness.\n"
    );
    for r in rows {
        if r.profile != cur {
            let _ = writeln!(s, "\n## {}", r.profile);
            cur = &r.profile;
        }
        let _ = writeln!(
            s,
            "  {:<22} {:<28} {}",
            r.dimension,
            r.status.tag(),
            r.detail
        );
    }

    let count = |st: Status| rows.iter().filter(|r| r.status == st).count();
    let _ = writeln!(s, "\n# Summary");
    let _ = writeln!(s, "  GATED ok            : {}", count(Status::Gated));
    let _ = writeln!(s, "  GATED FAIL          : {}", count(Status::GatedFail));
    let _ = writeln!(
        s,
        "  recon matches wire  : {}",
        count(Status::ReconAccurate)
    );
    let _ = writeln!(
        s,
        "  recon diverges      : {}",
        count(Status::ReconDiverges)
    );
    let _ = writeln!(s, "  unanchored          : {}", count(Status::Unanchored));

    let _ = writeln!(
        s,
        "\n# audit() reconstructions that diverge from the wire golden (not bugs — use live tls_peet for wire truth)"
    );
    for r in rows.iter().filter(|r| r.status == Status::ReconDiverges) {
        let _ = writeln!(s, "  [{}] {} — {}", r.profile, r.dimension, r.detail);
    }

    let _ = writeln!(
        s,
        "\n# Unanchored (no offline golden — wire truth needs a live capture)"
    );
    for r in rows.iter().filter(|r| r.status == Status::Unanchored) {
        let _ = writeln!(s, "  [{}] {} — {}", r.profile, r.dimension, r.detail);
    }
    s
}
