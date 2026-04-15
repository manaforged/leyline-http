//! clap argument tree.
//!
//! One file on purpose — the full CLI surface is less than 250 lines
//! and it's easier to reason about the flag groupings when they're
//! all in the same scope. If this file ever grows past 400 lines,
//! split it by subcommand.

use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use leyline::{Browser, Platform, ALL_BROWSERS};

/// `leyline` — fingerprint-aware HTTP client and profile inspector.
///
/// Run `leyline <URL>` for an instant GET with the default Chrome
/// profile. Run `leyline --help <subcommand>` for per-subcommand
/// flags. Full docs: https://github.com/manaforged/leyline-http
#[derive(Debug, Parser)]
#[command(
    name = "leyline",
    bin_name = "leyline",
    version,
    about = "Fingerprint-aware HTTP client built on the leyline TLS library.",
    long_about = None,
    arg_required_else_help = false,
    // Surface "you passed flags at the root but then also gave a
    // subcommand" as a usage error, instead of silently ignoring
    // the root flags. Applies to the whole flattened RequestArgs,
    // not just the positional URL.
    args_conflicts_with_subcommands = true,
    after_help = "Exit codes:\n  0   success (2xx)\n  2   usage error\n  3   config error\n  4   network error\n  5   protocol error\n  6   request timeout\n  22  HTTP 4xx\n  23  HTTP 5xx",
)]
pub struct Cli {
    /// Positional URL for the implicit `GET` form (`leyline <url>`).
    /// Ignored when a subcommand is given.
    #[arg(value_name = "URL", global = false)]
    pub url: Option<String>,

    /// Flags that apply to the implicit-GET form go here so they
    /// share a parser with the explicit verbs.
    #[command(flatten)]
    pub request: RequestArgs,

    /// Subcommand. If omitted and a URL is given, defaults to `get`.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// GET a URL. Equivalent to the bare `leyline <URL>` form.
    Get(VerbArgs),
    /// POST to a URL. Supply a body with `-d`, `--json`, or `--form`.
    Post(VerbArgs),
    /// PUT to a URL.
    Put(VerbArgs),
    /// PATCH a URL.
    Patch(VerbArgs),
    /// DELETE a URL.
    Delete(VerbArgs),
    /// HEAD a URL — no body is printed.
    Head(VerbArgs),
    /// Alias for `get -v` — shows the full audit block by default.
    Fetch(VerbArgs),
    /// HEAD with verbose fingerprint / cert / wire output — the
    /// fingerprint debugging workflow in a single command.
    Inspect(VerbArgs),

    /// Browser profile introspection (all offline, no network).
    #[command(subcommand)]
    Profile(ProfileCmd),

    /// Emit shell completions (bash, zsh, fish, elvish, powershell).
    Completions {
        /// Target shell.
        #[arg(value_enum)]
        shell: CompletionShell,
    },

    /// Print the CLI version, the linked `leyline` library version,
    /// and the full profile catalog with versions.
    Version,
}

#[derive(Debug, Subcommand)]
pub enum ProfileCmd {
    /// List every builtin profile with its version and platform.
    List,
    /// Dump the full profile — TLS ciphers, curves, sigalgs, H2
    /// SETTINGS, header preset.
    Show {
        /// Profile name (e.g. `chrome147`, `firefox148`, `safari18`).
        name: String,
    },
    /// Structured diff between two profiles.
    Diff {
        /// First profile.
        a: String,
        /// Second profile.
        b: String,
    },
    /// Compute JA3/JA4 for a profile **without issuing a request**.
    Audit {
        /// Profile name.
        name: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    Powershell,
}

/// Flags wrapped on every verb subcommand. Kept in a separate struct
/// so the implicit-GET form at the root can `#[command(flatten)]` it.
#[derive(Debug, Args, Clone)]
pub struct VerbArgs {
    /// Target URL.
    #[arg(value_name = "URL")]
    pub url: String,

    #[command(flatten)]
    pub request: RequestArgs,
}

impl Default for RequestArgs {
    fn default() -> Self {
        Self {
            profile: "chrome147".into(),
            platform: "windows".into(),
            http3: false,
            http2: false,
            http1: false,
            proxy: None,
            timeout: None,
            redirect: 10,
            insecure: false,
            header: Vec::new(),
            data: None,
            json: None,
            form: Vec::new(),
            query: Vec::new(),
            user: None,
            bearer: None,
            format: None,
            output: None,
            no_color: false,
            show_fingerprint: false,
            show_cert: None,
            show_wire: false,
            audit_only: false,
            verbose: false,
        }
    }
}

/// All request-shaping flags live here so the root `Cli` and every
/// `VerbArgs` share the same definition.
#[derive(Debug, Args, Clone)]
pub struct RequestArgs {
    // ─── session shaping ─────────────────────────────────────────
    /// Browser profile (`chrome147`, `firefox148`, `safari18`, ...).
    #[arg(
        short = 'p',
        long = "profile",
        value_name = "NAME",
        default_value = "chrome147"
    )]
    pub profile: String,

    /// Platform (`windows`, `macos`, `linux`, `android`, `ios`).
    #[arg(long = "platform", value_name = "OS", default_value = "windows")]
    pub platform: String,

    /// Force HTTP/3 over QUIC.
    #[arg(long = "http3", conflicts_with_all = ["http2", "http1"])]
    pub http3: bool,

    /// Force HTTP/2.
    #[arg(long = "http2", conflicts_with_all = ["http3", "http1"])]
    pub http2: bool,

    /// Force HTTP/1.1.
    #[arg(long = "http1", conflicts_with_all = ["http3", "http2"])]
    pub http1: bool,

    /// Proxy URL (`http://`, `https://`, `socks5://`, with or without auth).
    #[arg(short = 'x', long = "proxy", value_name = "URL")]
    pub proxy: Option<String>,

    /// Request timeout (`5s`, `500ms`, `2m`). Default: 30s.
    #[arg(long = "timeout", value_name = "DUR", value_parser = parse_duration)]
    pub timeout: Option<Duration>,

    /// Max redirects to follow. `0` disables.
    #[arg(long = "redirect", value_name = "N", default_value = "10")]
    pub redirect: usize,

    /// **Dangerous.** Skip TLS peer certificate verification.
    #[arg(short = 'k', long = "insecure")]
    pub insecure: bool,

    // ─── request body / headers ─────────────────────────────────
    /// Extra request header (`Name: value`). Repeatable.
    #[arg(short = 'H', long = "header", value_name = "H", action = ArgAction::Append)]
    pub header: Vec<String>,

    /// Request body. Literal string, `@path/to/file`, or `-` for stdin.
    #[arg(short = 'd', long = "data", value_name = "BODY")]
    pub data: Option<String>,

    /// Serialize the given JSON literal as the request body and set
    /// `Content-Type: application/json`.
    #[arg(long = "json", value_name = "JSON", conflicts_with_all = ["data", "form"])]
    pub json: Option<String>,

    /// URL-encoded form field (`k=v`). Repeatable.
    #[arg(long = "form", value_name = "K=V", action = ArgAction::Append)]
    pub form: Vec<String>,

    /// Query string parameter (`k=v`). Repeatable. Added to the URL.
    #[arg(short = 'q', long = "query", value_name = "K=V", action = ArgAction::Append)]
    pub query: Vec<String>,

    /// HTTP basic auth (`user:pass`).
    #[arg(short = 'u', long = "user", value_name = "USER:PASS")]
    pub user: Option<String>,

    /// Bearer token. Sent as `Authorization: Bearer <token>`.
    #[arg(long = "bearer", value_name = "TOKEN")]
    pub bearer: Option<String>,

    // ─── output ────────────────────────────────────────────────
    /// Output format. Defaults to `pretty` on a TTY, `raw` when piped.
    #[arg(long = "format", value_enum, value_name = "FMT")]
    pub format: Option<OutputFormat>,

    /// Write the body to a file instead of stdout.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Disable ANSI color output for this invocation. `NO_COLOR=1`
    /// in the environment is honored automatically by `anstream` and
    /// has the same effect.
    #[arg(long = "no-color")]
    pub no_color: bool,

    // ─── differentiated flags (Leyline-only) ────────────────────
    /// Print the JA4/JA3/JA4H/JA4T/H2 fingerprint block after the response.
    #[arg(short = 'f', long = "show-fingerprint")]
    pub show_fingerprint: bool,

    /// Print the TLS peer certificate (`summary`, `pem`, or `hex`).
    #[arg(long = "show-cert", value_name = "FMT", value_enum)]
    pub show_cert: Option<CertFormat>,

    /// Print the wire-final request headers (after session merge).
    #[arg(long = "show-wire")]
    pub show_wire: bool,

    /// Suppress the response body entirely; print only the audit block.
    #[arg(long = "audit-only")]
    pub audit_only: bool,

    /// Shorthand for `--show-fingerprint --show-cert summary --show-wire`.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum OutputFormat {
    /// Colored, httpie-style output (default on TTY).
    Pretty,
    /// Structured JSON document with request/response/audit/tls keys.
    Json,
    /// Response body only, nothing else (default when stdout is piped).
    Raw,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum CertFormat {
    /// Short summary: subject CN, SAN list, issuer, validity.
    Summary,
    /// Full PEM-encoded certificate.
    Pem,
    /// Hex dump of the DER bytes.
    Hex,
}

// ─── parsers / helpers ───────────────────────────────────────────

fn parse_duration(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s).map_err(|e| format!("invalid duration `{s}`: {e}"))
}

impl RequestArgs {
    /// Resolve `--profile` into a `leyline::Browser`. Accepts the
    /// canonical names from `ALL_BROWSERS` in a case-insensitive,
    /// punctuation-insensitive form — `chrome147`, `Chrome147`,
    /// `chrome-147`, `chrome_147` all work.
    pub fn resolve_browser(&self) -> Result<Browser, String> {
        let norm = normalize_name(&self.profile);
        for b in ALL_BROWSERS {
            if normalize_name(&browser_cli_name(b)) == norm
                || normalize_name(&b.to_string()) == norm
            {
                return Ok(b);
            }
        }
        let known = ALL_BROWSERS
            .iter()
            .map(|b| browser_cli_name(*b))
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "unknown profile `{}` (known: {known})",
            self.profile
        ))
    }

    pub fn resolve_platform(&self) -> Result<Platform, String> {
        match normalize_name(&self.platform).as_str() {
            "windows" | "win" => Ok(Platform::Windows),
            "macos" | "mac" | "osx" | "darwin" => Ok(Platform::MacOS),
            "linux" => Ok(Platform::Linux),
            "android" => Ok(Platform::Android),
            "ios" | "iphone" | "ipad" => Ok(Platform::IOS),
            other => Err(format!("unknown platform `{other}`")),
        }
    }

    /// Apply `-v / --verbose` shorthand: toggle on fingerprint + cert
    /// summary + wire. Idempotent.
    pub fn apply_verbose(&mut self) {
        if self.verbose {
            self.show_fingerprint = true;
            self.show_wire = true;
            if self.show_cert.is_none() {
                self.show_cert = Some(CertFormat::Summary);
            }
        }
    }
}

/// Canonical CLI name for a browser (`chrome147`, `firefox148`, ...).
pub fn browser_cli_name(b: Browser) -> String {
    let (name, version) = b.profile_key();
    let name = name.replace('-', "");
    format!("{name}{version}")
}

fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_canonical_browser_name() {
        let args = RequestArgs {
            profile: "chrome147".into(),
            ..Default::default()
        };
        assert_eq!(args.resolve_browser().unwrap(), Browser::Chrome147);
    }

    #[test]
    fn resolves_case_and_punct_insensitive() {
        for variant in ["Chrome147", "chrome-147", "chrome_147", "CHROME147"] {
            let args = RequestArgs {
                profile: variant.into(),
                ..Default::default()
            };
            assert_eq!(args.resolve_browser().unwrap(), Browser::Chrome147);
        }
    }

    #[test]
    fn resolves_safari_ios_profile() {
        let args = RequestArgs {
            profile: "safariios18".into(),
            ..Default::default()
        };
        assert_eq!(args.resolve_browser().unwrap(), Browser::SafariiOS18);
    }

    #[test]
    fn unknown_profile_lists_alternatives() {
        let args = RequestArgs {
            profile: "netscape1".into(),
            ..Default::default()
        };
        let err = args.resolve_browser().unwrap_err();
        assert!(err.contains("chrome147"));
    }

    #[test]
    fn verbose_sets_all_show_flags() {
        let mut args = RequestArgs {
            verbose: true,
            ..Default::default()
        };
        args.apply_verbose();
        assert!(args.show_fingerprint);
        assert!(args.show_wire);
        assert_eq!(args.show_cert, Some(CertFormat::Summary));
    }
}
