//! Guards against SSL_CERT_DIR silently disabling trust on Debian/Ubuntu/RHEL, where all entries in `/etc/ssl/certs` are symlinks.

use super::collect_ca_dir_candidates;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Disposable per-test directory under the OS temp dir.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "leyline-ca-{}-{}-{}",
            label,
            std::process::id(),
            id
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_pem(path: &Path, label: &str) {
    let mut f = fs::File::create(path).unwrap();
    writeln!(
        f,
        "-----BEGIN CERTIFICATE-----\n{label}\n-----END CERTIFICATE-----"
    )
    .unwrap();
}

#[test]
fn missing_dir_yields_empty_candidates() {
    let tmp = TempDir::new("missing");
    let nowhere = tmp.path().join("does-not-exist");
    assert!(collect_ca_dir_candidates(&nowhere).is_empty());
}

#[test]
fn accepts_regular_pem_crt_cer_files() {
    let tmp = TempDir::new("regular");
    write_pem(&tmp.path().join("a.pem"), "a");
    write_pem(&tmp.path().join("b.crt"), "b");
    write_pem(&tmp.path().join("c.cer"), "c");
    let mut v = collect_ca_dir_candidates(tmp.path());
    v.sort();
    assert_eq!(v.len(), 3, "expected 3 candidates, got {v:?}");
}

#[test]
fn extension_match_is_case_insensitive() {
    let tmp = TempDir::new("case");
    write_pem(&tmp.path().join("a.PEM"), "a");
    write_pem(&tmp.path().join("b.Crt"), "b");
    let v = collect_ca_dir_candidates(tmp.path());
    assert_eq!(v.len(), 2);
}

#[test]
fn wrong_extensions_skipped() {
    let tmp = TempDir::new("ext");
    write_pem(&tmp.path().join("a.pem"), "a");
    write_pem(&tmp.path().join("b.hash"), "b");
    write_pem(&tmp.path().join("README"), "c");
    write_pem(&tmp.path().join("d.txt"), "d");
    write_pem(&tmp.path().join("e.pem.bak"), "e");
    let v = collect_ca_dir_candidates(tmp.path());
    assert_eq!(v.len(), 1, "only a.pem should pass: {v:?}");
}

#[test]
#[cfg(unix)]
fn symlink_to_regular_file_is_accepted() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new("sym");
    let real = tmp.path().join("real.pem");
    write_pem(&real, "real");
    let link = tmp.path().join("link.pem");
    symlink(&real, &link).unwrap();
    let v = collect_ca_dir_candidates(tmp.path());
    assert_eq!(v.len(), 2, "got {v:?}");
}

#[test]
#[cfg(unix)]
fn broken_symlink_skipped() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new("broken");
    let link = tmp.path().join("dangling.pem");
    symlink("/nonexistent/target.pem", &link).unwrap();
    let v = collect_ca_dir_candidates(tmp.path());
    assert!(v.is_empty(), "broken symlink must be skipped: {v:?}");
}

#[test]
fn directory_entry_with_cert_extension_skipped() {
    let tmp = TempDir::new("subdir");
    fs::create_dir(tmp.path().join("bogus.pem")).unwrap();
    write_pem(&tmp.path().join("real.pem"), "real");
    let v = collect_ca_dir_candidates(tmp.path());
    assert_eq!(v.len(), 1, "only real.pem expected: {v:?}");
}
