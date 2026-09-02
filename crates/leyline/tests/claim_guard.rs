//! Guards public Leyline claims against absolute marketing language.
use std::path::{Path, PathBuf};

const FORBIDDEN_WORDS: &[&str] = &[
    "best",
    "undetectable",
    "indistinguishable",
    "unblockable",
    "perfect",
    "perfectly",
    "bypass",
];

const FORBIDDEN_PHRASES: &[&str] = &[
    "best in the world",
    "best http/tls library",
    "identical to real browsers",
    "exactly like chrome",
    "cannot be detected",
    "guaranteed to work",
    "byte-for-byte browser",
    "matches every browser",
];

#[test]
fn public_claims_stay_bounded() {
    let mut violations = Vec::new();

    for path in public_claim_surfaces() {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        scan_file(&path, &text, &mut violations);
    }

    assert!(
        violations.is_empty(),
        "public claim guard found unsupported language:\n{}",
        violations.join("\n")
    );
}

/// Every test or example name printed in README.md must exist in src/, tests/, or examples/.
#[test]
fn cited_test_names_exist() {
    let known = source_identifiers();
    let mut missing = Vec::new();

    let raw = std::fs::read_to_string(repo_root().join("README.md")).unwrap();
    let text = strip_section(&raw, "## Fuzzing");
    for name in cited_test_candidates(&text) {
        if !known.contains(&name) {
            missing.push(format!(
                "README.md cites `{name}`, which exists nowhere in src/, tests/, or examples/"
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "documentation cites test/identifier names that do not exist:\n{}",
        missing.join("\n")
    );
}

/// Doc examples must not reintroduce two known footguns: `?` on an infallible constructor (won't compile) and `resp.audit().unwrap()` (panics unless audit was enabled on the session).
#[test]
fn examples_avoid_known_footguns() {
    const FORBIDDEN_SNIPPETS: &[&str] = &[
        "chrome()?",
        "firefox()?",
        "safari()?",
        "edge()?",
        "brave()?",
        "opera()?",
        "vivaldi()?",
        ".audit().unwrap()",
    ];
    let mut hits = Vec::new();
    for path in public_claim_surfaces() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (idx, line) in text.lines().enumerate() {
            for snip in FORBIDDEN_SNIPPETS {
                if line.contains(snip) {
                    hits.push(format!(
                        "{}:{} reintroduces footgun `{}`: {}",
                        display_path(&path),
                        idx + 1,
                        snip,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        hits.is_empty(),
        "doc examples contain known-broken patterns:\n{}",
        hits.join("\n")
    );
}

/// Drop a markdown section (the `## Heading` line through the line before the next `## ` heading) so its contents are excluded from scanning.
fn strip_section(text: &str, heading: &str) -> String {
    let mut out = String::new();
    let mut in_section = false;
    for line in text.lines() {
        if line.starts_with("## ") {
            in_section = line.trim_end() == heading;
        }
        if !in_section {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Backticked tokens in a doc that look like a test/function name.
fn cited_test_candidates(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, span) in text.split('`').enumerate() {
        if i % 2 == 0 {
            continue;
        }
        if let Some((prefix, rest)) = span.split_once('{') {
            if let Some(items) = rest.strip_suffix('}')
                && is_snake_ident(prefix)
            {
                for item in items.split(',') {
                    out.push(format!("{prefix}{}", item.trim()));
                }
            }
        } else if is_snake_ident(span) {
            out.push(span.to_string());
        }
    }
    out
}

/// Lowercase snake_case with at least one underscore — the shape of test fn names and lowercase API methods, but not paths (`foo.rs`), types (`H2Config`), or `Type::method` citations.
fn is_snake_ident(s: &str) -> bool {
    s.contains('_')
        && !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Every identifier-shaped token that appears in the crate's `src/` or `tests/` Rust sources (test fn names included).
fn source_identifiers() -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for sub in ["src", "tests", "examples"] {
        collect_identifiers(&crate_dir.join(sub), &mut set);
    }
    set
}

fn collect_identifiers(dir: &Path, set: &mut std::collections::HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_identifiers(&path, set);
        } else if path.extension().is_some_and(|e| e == "rs") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                set.insert(stem.to_string());
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                for token in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                    if is_snake_ident(token) {
                        set.insert(token.to_string());
                    }
                }
            }
        }
    }
}

fn public_claim_surfaces() -> Vec<PathBuf> {
    let root = repo_root();
    [
        "README.md",
        "crates/leyline/Cargo.toml",
        "crates/leyline/src/lib.rs",
        "crates/leyline/src/audit/mod.rs",
        "crates/leyline/src/cookie/mod.rs",
        "crates/leyline/src/core/mod.rs",
        "crates/leyline/src/h2/mod.rs",
        "crates/leyline/src/pool/mod.rs",
        "crates/leyline/src/profile/mod.rs",
        "crates/leyline/src/quic/mod.rs",
        "crates/leyline/src/tcp/mod.rs",
        "crates/leyline/src/tls/mod.rs",
    ]
    .into_iter()
    .map(|path| root.join(path))
    .collect()
}

fn scan_file(path: &Path, text: &str, violations: &mut Vec<String>) {
    for (idx, line) in text.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        for word in FORBIDDEN_WORDS {
            if contains_word(&lower, word) {
                violations.push(format!(
                    "{}:{} contains banned word `{}`: {}",
                    display_path(path),
                    idx + 1,
                    word,
                    line.trim()
                ));
            }
        }

        for phrase in FORBIDDEN_PHRASES {
            if lower.contains(phrase) {
                violations.push(format!(
                    "{}:{} contains banned phrase `{}`: {}",
                    display_path(path),
                    idx + 1,
                    phrase,
                    line.trim()
                ));
            }
        }
    }
}

fn contains_word(line: &str, word: &str) -> bool {
    line.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|part| part == word)
}

fn display_path(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("leyline crate should live two levels below repo root")
        .to_path_buf()
}
