// leyline-quiche build script.
//
// Fork of cloudflare/quiche 0.23.7 build script, stripped to the
// `boringssl-btls-crate` path only. We do NOT build a bundled BoringSSL;
// `btls-sys` owns the BoringSSL build (it declares `links = "boringssl"` and
// emits the static link directives for ssl + crypto via its own build script).
//
// Original authors: Cloudflare, Inc. (BSD-2-Clause).

fn main() {
    // btls-sys emits `cargo:rustc-link-lib=static=ssl` and `=crypto` via its
    // own build script. We must NOT re-emit them here — duplicate link-lib
    // directives for the same symbols cause linker errors on Windows/MSVC.

    // MacOS: allow cdylib to link with undefined symbols.
    let target_os =
        std::env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS must be set by cargo");
    if target_os == "macos" {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-undefined,dynamic_lookup");
    }
}
