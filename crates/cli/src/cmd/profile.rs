//! `leyline profile` subcommand group — offline introspection of the
//! builtin TLS profiles.

use std::io::Write;

use anyhow::{anyhow, Result};
use owo_colors::OwoColorize;

use leyline::{profile, Browser, ALL_BROWSERS};

use crate::args::{browser_cli_name, ProfileCmd};
use crate::exit::ExitCode;

pub fn run(cmd: ProfileCmd) -> Result<ExitCode> {
    match cmd {
        ProfileCmd::List => list(),
        ProfileCmd::Show { name } => show(&name),
        ProfileCmd::Diff { a, b } => diff(&a, &b),
        ProfileCmd::Audit { name } => audit(&name),
    }
}

fn list() -> Result<ExitCode> {
    let mut out = anstream::stdout().lock();
    writeln!(
        out,
        "{}",
        format!("{:<18} {:<12} {}", "profile", "family", "version").bold()
    )?;
    writeln!(out, "{}", "─".repeat(48).dimmed())?;
    for b in ALL_BROWSERS {
        let (family, version) = b.profile_key();
        let cli = browser_cli_name(b);
        writeln!(
            out,
            "{:<18} {:<12} {}",
            cli.cyan(),
            family,
            version.to_string().magenta()
        )?;
    }
    Ok(ExitCode::Ok)
}

fn show(name: &str) -> Result<ExitCode> {
    let browser = resolve(name)?;
    let p = profile(browser);
    let mut out = anstream::stdout().lock();

    writeln!(
        out,
        "{} {}",
        "profile:".dimmed(),
        browser.to_string().bold()
    )?;
    writeln!(out)?;

    writeln!(out, "{}", "── tls ──".dimmed())?;
    writeln!(out, "  ciphers ({}):", p.tls.ciphers.len())?;
    for c in &p.tls.ciphers {
        writeln!(out, "    {c}")?;
    }
    writeln!(out, "  curves ({}):", p.tls.curves.len())?;
    for c in &p.tls.curves {
        writeln!(out, "    {c}")?;
    }
    writeln!(out, "  sigalgs ({}):", p.tls.sigalgs.len())?;
    for s in &p.tls.sigalgs {
        writeln!(out, "    {s}")?;
    }
    if let Some(alps) = &p.tls.alps {
        writeln!(
            out,
            "  alps: {alps} (new-codepoint: {})",
            p.tls.alps_new_codepoint
        )?;
    }
    writeln!(out, "  ech_grease: {}", p.tls.ech_grease)?;
    if let Some(ext) = &p.tls.extension_permutation {
        writeln!(out, "  extension_permutation: {ext:?}")?;
    }
    writeln!(out)?;

    writeln!(out, "{}", "── http/2 ──".dimmed())?;
    for (name, value) in h2_settings_pairs(p) {
        writeln!(out, "  setting {name:<32} = {value}")?;
    }
    writeln!(out, "  pseudo_order ({}):", p.h2.pseudo_order.len())?;
    for h in &p.h2.pseudo_order {
        writeln!(out, "    {h}")?;
    }
    writeln!(out, "  settings_order ({}):", p.h2.settings_order.len())?;
    for h in &p.h2.settings_order {
        writeln!(out, "    {h}")?;
    }
    writeln!(out)?;

    writeln!(out, "{}", "── identities ──".dimmed())?;
    for name in p.identity.keys() {
        writeln!(out, "  {name}")?;
    }

    Ok(ExitCode::Ok)
}

fn diff(a: &str, b: &str) -> Result<ExitCode> {
    let ba = resolve(a)?;
    let bb = resolve(b)?;
    let pa = profile(ba);
    let pb = profile(bb);
    let mut out = anstream::stdout().lock();

    writeln!(
        out,
        "{} {} {} {}",
        "diff:".dimmed(),
        ba.to_string().bold(),
        "→".dimmed(),
        bb.to_string().bold()
    )?;
    writeln!(out)?;

    diff_list(&mut out, "ciphers", &pa.tls.ciphers, &pb.tls.ciphers)?;
    diff_list(&mut out, "curves", &pa.tls.curves, &pb.tls.curves)?;
    diff_list(&mut out, "sigalgs", &pa.tls.sigalgs, &pb.tls.sigalgs)?;
    diff_list(
        &mut out,
        "h2 pseudo_order",
        &pa.h2.pseudo_order,
        &pb.h2.pseudo_order,
    )?;
    diff_list(
        &mut out,
        "h2 settings_order",
        &pa.h2.settings_order,
        &pb.h2.settings_order,
    )?;

    // H2 SETTINGS — structured pair diff.
    let setting_diff = h2_setting_diff(pa, pb);
    if !setting_diff.is_empty() {
        writeln!(out, "{}", "h2 settings".bold())?;
        for line in setting_diff {
            writeln!(out, "  {line}")?;
        }
        writeln!(out)?;
    }

    Ok(ExitCode::Ok)
}

fn audit(name: &str) -> Result<ExitCode> {
    let browser = resolve(name)?;
    let p = profile(browser);
    let mut out = anstream::stdout().lock();

    let extension_ids = leyline::audit::chrome_extension_ids(&p.tls);
    let ja4 = leyline::audit::compute_ja4(&leyline::audit::Ja4Input {
        ciphers: &p.tls.ciphers,
        sigalgs: &p.tls.sigalgs,
        curves: &p.tls.curves,
        extension_ids: &extension_ids,
        tls_version: "1.3",
        has_sni: true,
        alpn: "h2",
    });
    let ja3 = leyline::audit::compute_ja3(&leyline::audit::Ja3Input {
        ciphers: &p.tls.ciphers,
        curves: &p.tls.curves,
        extension_ids: &extension_ids,
        tls_record_version: 0x0303,
    });

    writeln!(
        out,
        "{} {}",
        "profile:".dimmed(),
        browser.to_string().bold()
    )?;
    writeln!(out, "{}    {}", "JA4".dimmed(), ja4.magenta().bold())?;
    writeln!(out, "{}    {}", "JA3".dimmed(), ja3.dimmed())?;
    writeln!(
        out,
        "{}",
        "  (JA4H is request-dependent and can't be computed offline)".dimmed()
    )?;
    Ok(ExitCode::Ok)
}

// ─── helpers ─────────────────────────────────────────────────────

fn resolve(name: &str) -> Result<Browser> {
    let norm = normalize(name);
    for b in ALL_BROWSERS {
        if normalize(&browser_cli_name(b)) == norm || normalize(&b.to_string()) == norm {
            return Ok(b);
        }
    }
    Err(anyhow!(
        "unknown profile `{name}`; run `leyline profile list` for the full set"
    ))
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn diff_list<W: Write>(
    out: &mut W,
    label: &str,
    a: &[String],
    b: &[String],
) -> std::io::Result<()> {
    if a == b {
        return Ok(());
    }
    writeln!(out, "{}", label.bold())?;
    let a_set: std::collections::BTreeSet<_> = a.iter().collect();
    let b_set: std::collections::BTreeSet<_> = b.iter().collect();
    for added in b_set.difference(&a_set) {
        writeln!(out, "  {} {added}", "+".green())?;
    }
    for removed in a_set.difference(&b_set) {
        writeln!(out, "  {} {removed}", "-".red())?;
    }
    if a.len() == b.len() && a_set == b_set {
        writeln!(out, "  {} order changed", "~".yellow())?;
    }
    writeln!(out)
}

/// Pull every H2 SETTINGS frame field off a profile as
/// `(name, Option<u32>)`, preserving declaration order. Lets both
/// `show` and `diff` share the same view.
macro_rules! h2_fields {
    ($p:expr) => {
        [
            ("header_table_size", $p.header_table_size),
            ("enable_push", $p.enable_push.map(|v| v as u32)),
            ("max_concurrent_streams", $p.max_concurrent_streams),
            ("initial_stream_window_size", $p.initial_stream_window_size),
            (
                "initial_connection_window_size",
                $p.initial_connection_window_size,
            ),
            ("max_frame_size", $p.max_frame_size),
            ("max_header_list_size", $p.max_header_list_size),
            ("unknown_setting8", $p.unknown_setting8),
            ("unknown_setting9", $p.unknown_setting9),
        ]
    };
}

fn h2_settings_pairs(p: &leyline::BrowserProfile) -> Vec<(&'static str, u32)> {
    h2_fields!(&p.h2)
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect()
}

fn h2_setting_diff(a: &leyline::BrowserProfile, b: &leyline::BrowserProfile) -> Vec<String> {
    let mut lines = Vec::new();
    let av = h2_fields!(&a.h2);
    let bv = h2_fields!(&b.h2);
    for ((name, va), (_, vb)) in av.iter().zip(bv.iter()) {
        match (va, vb) {
            (Some(x), Some(y)) if x != y => lines.push(format!("~ {name}: {x} → {y}")),
            (None, Some(y)) => lines.push(format!("+ {name} = {y}")),
            (Some(x), None) => lines.push(format!("- {name} = {x}")),
            _ => {}
        }
    }
    lines
}
