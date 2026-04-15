//! Guards public Leyline claims against absolute marketing language.

use std::path::{Path, PathBuf};

const FORBIDDEN_WORDS: &[&str] = &[
    "best",
    "undetectable",
    "indistinguishable",
    "unblockable",
    "perfect",
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

#[test]
fn readme_keeps_public_evidence_commands() {
    let readme = std::fs::read_to_string(repo_root().join("README.md")).unwrap();

    for required in [
        "cargo run -p leyline --example smoke",
        "cargo test --workspace",
        "cargo test -p leyline --test tls_peet -- --ignored",
    ] {
        assert!(
            readme.contains(required),
            "README missing public evidence command: {required}"
        );
    }
}

fn public_claim_surfaces() -> Vec<PathBuf> {
    let root = repo_root();
    [
        "README.md",
        "crates/leyline/Cargo.toml",
        "crates/leyline/src/lib.rs",
        "crates/core/src/lib.rs",
        "crates/tls/src/lib.rs",
        "crates/h2/src/lib.rs",
        "crates/quic/src/lib.rs",
        "crates/cookies/src/lib.rs",
        "crates/audit/src/lib.rs",
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
