#!/usr/bin/env bash
# Produce + check in a prebuilt BoringSSL bundle.
#
# Leyline's leyline-bssl-sys crate links checked-in prebuilt BoringSSL so most
# developers never need CMake/Perl/libclang/Go. Adding a new target means
# running the in-repo source build once on a machine of that target and copying
# the resulting static libs + bindgen output into the repo. This script
# automates the build-and-strip steps, and works for any host (including the
# targets we don't ship yet: x86_64-apple-darwin, aarch64-unknown-linux-gnu).
#
# Prereqs on the host: a Rust toolchain plus the source-build deps
#   (cmake, perl, go, and libclang — `LIBCLANG_PATH` may need setting on macOS).
# Cross-packaging x86_64-pc-windows-msvc from a non-Windows host additionally
# requires cargo-xwin, clang-cl, lld-link, llvm-lib, ninja, and nasm. cargo-xwin
# acquires the Microsoft SDK/UCRT sysroot; it deliberately does not use MinGW.
# On a Windows host the same target builds natively: run this script under Git
# Bash inside a Visual Studio developer environment, with cmake, ninja, nasm,
# perl, go, and LLVM (libclang) on PATH.
#
# Usage:
#   ./scripts/package-bssl.sh            # build + install bundle for the host target
#   ./scripts/package-bssl.sh --check    # only report what's missing, build nothing
#   ./scripts/package-bssl.sh --verify   # only verify committed artifacts against
#                                        #   native/CHECKSUMS (no build)
#   ./scripts/package-bssl.sh --target x86_64-pc-windows-msvc
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
export PATH="$HOME/.cargo/bin:$PATH"

# Cargo and CMake on Windows are native executables; hand them a Windows path
# even when this script runs under Git Bash/MSYS. Elsewhere this is identity.
winpath() {
    if command -v cygpath >/dev/null 2>&1; then
        cygpath -w "$1"
    else
        printf '%s\n' "$1"
    fi
}

step() { printf '\n\033[1;34m== %s ==\033[0m\n' "$*"; }
ok()   { printf '\033[1;32mOK %s\033[0m\n'  "$*"; }
warn() { printf '\033[1;33mWARN %s\033[0m\n' "$*"; }
die()  { printf '\033[1;31mERROR %s\033[0m\n' "$*" >&2; exit 1; }
usage() {
    cat <<'EOF'
Usage: ./scripts/package-bssl.sh [--check] [--verify] [--target <triple>]

Without --target, package the host target. The only supported cross target is
x86_64-pc-windows-msvc; it uses cargo-xwin, clang-cl, lld-link, llvm-lib, and
the Microsoft SDK/UCRT sysroot acquired by cargo-xwin.
EOF
}

check_only=0
verify_only=0
target=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --check) check_only=1 ;;
        --verify) verify_only=1 ;;
        --target)
            [[ $# -ge 2 ]] || die "--target requires a Rust target triple"
            target="$2"
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *) die "unknown argument: $1 (expected --check, --verify, or --target <triple>)" ;;
    esac
    shift
done

sys_dir="crates/leyline-bssl-sys"

if [[ $verify_only -eq 1 ]]; then
    step "verify committed artifacts against native/CHECKSUMS"
    (cd "$sys_dir" && sha256sum -c native/CHECKSUMS) || die "checksum drift: committed artifacts differ from native/CHECKSUMS"
    ok "all artifacts match native/CHECKSUMS"
    exit 0
fi

# --- target triple ---------------------------------------------------------
if [[ -z "$target" ]]; then
    target="$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')"
    [[ -n "$target" ]] || die "could not determine host target from 'rustc -vV' — is Rust installed?"
fi
triple="$target"
cross_windows=0
if [[ "$triple" == "x86_64-pc-windows-msvc" ]]; then
    host="$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')"
    [[ -n "$host" ]] || die "could not determine host target from 'rustc -vV' — is Rust installed?"
    [[ "$host" != "$triple" ]] && cross_windows=1
elif [[ "$target" != "$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')" ]]; then
    die "cross-packaging is supported only for x86_64-pc-windows-msvc; run without --target for a host bundle"
fi

case "$triple" in
    *-msvc)  is_msvc=1; crypto_lib="crypto.lib"; ssl_lib="ssl.lib" ;;
    *)       is_msvc=0; crypto_lib="libcrypto.a"; ssl_lib="libssl.a" ;;
esac

native_dir="$sys_dir/native/$triple/lib"
bindings_file="$sys_dir/src/bindings/$triple.rs"

step "target: $triple"
have_libs=0
[[ -f "$native_dir/$crypto_lib" && -f "$native_dir/$ssl_lib" ]] && have_libs=1
have_bindings=0
[[ -f "$bindings_file" ]] && have_bindings=1
registered_build=0
grep -q "\"$triple\"" "$sys_dir/build.rs" && registered_build=1
registered_lib=0
grep -q "$triple" "$sys_dir/src/lib.rs" && registered_lib=1

printf '  native libs (%s, %s): %s\n' "$crypto_lib" "$ssl_lib" \
    "$([[ $have_libs == 1 ]] && echo present || echo MISSING)"
printf '  bindings (%s): %s\n' "$bindings_file" \
    "$([[ $have_bindings == 1 ]] && echo present || echo MISSING)"
printf '  registered in build.rs SUPPORTED_TARGETS: %s\n' \
    "$([[ $registered_build == 1 ]] && echo yes || echo NO)"
printf '  registered in src/lib.rs cfg arms: %s\n' \
    "$([[ $registered_lib == 1 ]] && echo yes || echo NO)"

if [[ $check_only == 1 ]]; then
    exit 0
fi

if [[ $have_libs == 1 && $have_bindings == 1 ]]; then
    warn "a bundle for $triple already exists; rebuilding will overwrite it"
fi

# --- source-build leyline-bssl-sys in-repo ---------------------------------
# The crate links the prebuilt libs by default (build = "build.rs"); the source
# build (cmake + bindgen) lives behind build/main.rs + the source-build feature.
# Toggle to it, build, collect the static libs + bindings.rs, then toggle back.
step "source-building $sys_dir from BoringSSL (this can take 5-15 min)"
for tool in cmake perl go; do
    command -v "$tool" >/dev/null 2>&1 || warn "$tool not found on PATH — the source build will likely fail without it"
done

if [[ $cross_windows == 1 ]]; then
    for tool in clang-cl lld-link llvm-lib ninja nasm; do
        command -v "$tool" >/dev/null 2>&1 || die "$tool is required for x86_64-pc-windows-msvc packaging; install LLVM and retry"
    done
    cargo xwin --version >/dev/null 2>&1 || die "cargo-xwin is required for x86_64-pc-windows-msvc packaging; install it with: cargo install --locked cargo-xwin"
    [[ "${XWIN_CROSS_COMPILER:-clang-cl}" == "clang-cl" ]] || die "x86_64-pc-windows-msvc packaging requires XWIN_CROSS_COMPILER=clang-cl, never MinGW or clang"
fi

git submodule update --init "$sys_dir/deps/boringssl"

restore_build() { perl -0pi -e 's/^build = "build\/main\.rs"/build = "build.rs"/m' "$sys_dir/Cargo.toml"; }
build_root="$(mktemp -d)"
trap 'restore_build; rm -rf "$build_root"' EXIT
target_root="${CARGO_TARGET_DIR:-$build_root/target}"
build_dir_root="${CARGO_BUILD_BUILD_DIR:-$target_root}"
perl -0pi -e 's/^build = "build\.rs"/build = "build\/main.rs"/m' "$sys_dir/Cargo.toml"

if [[ $cross_windows == 1 ]]; then
    (
        cd "$sys_dir"
        XWIN_CROSS_COMPILER=clang-cl \
        CARGO_TARGET_DIR="$(winpath "$target_root")" \
        cargo xwin build --release --target "$triple" --features source-build
    ) || die "Windows/MSVC source build failed — cargo-xwin must be able to acquire its Microsoft SDK/UCRT sysroot and use clang-cl/lld-link/llvm-lib"
else
    ( cd "$sys_dir" && CARGO_TARGET_DIR="$(winpath "$target_root")" cargo build --release --features source-build ) \
        || die "leyline-bssl-sys source build failed — install cmake/perl/go/libclang and retry"
fi
restore_build

artifact_dir="$build_dir_root/release"
[[ $cross_windows == 1 ]] && artifact_dir="$build_dir_root/$triple/release"
bindings_src="$(find "$artifact_dir" -name bindings.rs -print -quit 2>/dev/null)"
[[ -n "$bindings_src" ]] || die "could not locate leyline-bssl-sys OUT_DIR after build"
src_out="$(dirname "$bindings_src")"

# --- install libs -----------------------------------------------------------
step "installing artifacts into the repo"
mkdir -p "$native_dir"
find_lib() {
    local name="$1"
    if [[ -f "$src_out/build/$name" ]]; then
        printf '%s\n' "$src_out/build/$name"
        return
    fi
    find "$src_out/build" -type f -name "$name" -print -quit
}
crypto_src="$(find_lib "$crypto_lib")"
ssl_src="$(find_lib "$ssl_lib")"
[[ -n "$crypto_src" ]] || die "could not locate $crypto_lib in the BoringSSL build output"
[[ -n "$ssl_src" ]] || die "could not locate $ssl_lib in the BoringSSL build output"
cp "$crypto_src" "$native_dir/$crypto_lib"
cp "$ssl_src"    "$native_dir/$ssl_lib"
if [[ $is_msvc == 0 ]]; then
    case "$triple" in
        *-apple-*) strip -S "$native_dir/$crypto_lib" "$native_dir/$ssl_lib" ;;
        *)         strip --strip-debug "$native_dir/$crypto_lib" "$native_dir/$ssl_lib" ;;
    esac
fi
ok "native libs -> $native_dir/"

cp "$src_out/bindings.rs" "$bindings_file"
ok "bindings -> $bindings_file"

# --- prove the exports carry the LEYLINE_ prefix ---------------------------
# BoringSSL is built with -DBORINGSSL_PREFIX=LEYLINE so a downstream crate can
# link openssl-sys or boring-sys in the same binary. Fail the package if an
# unprefixed OpenSSL-style export survived.
step "verifying LEYLINE_ symbol prefixing"
# llvm-nm reads both ELF/Mach-O archives and MSVC .lib; dumpbin is the fallback
# on a Windows host that has the MSVC tools but no LLVM.
nm_tool=""
for candidate in llvm-nm nm; do
    command -v "$candidate" >/dev/null 2>&1 && { nm_tool="$candidate"; break; }
done
dumpbin_tool=""
command -v dumpbin >/dev/null 2>&1 && dumpbin_tool="dumpbin"
if [[ $is_msvc == 1 && "$nm_tool" == "nm" ]]; then
    nm_tool=""
fi
if [[ -z "$nm_tool" && -z "$dumpbin_tool" ]]; then
    warn "no llvm-nm, nm, or dumpbin on PATH — cannot verify symbol prefixing for $triple"
else
    if [[ -n "$nm_tool" ]]; then
        exports="$("$nm_tool" --defined-only --extern-only "$native_dir/$crypto_lib" "$native_dir/$ssl_lib" 2>/dev/null \
            | awk '{print $NF}' | sed 's/^_//')"
    else
        # `//symbols` survives MSYS argument mangling as `/symbols`.
        exports="$(dumpbin //symbols "$(winpath "$native_dir/$crypto_lib")" "$(winpath "$native_dir/$ssl_lib")" 2>/dev/null \
            | awk '/External/ && !/UNDEF/ {print $NF}' | sed 's/^_//')"
    fi
    prefixed="$(printf '%s\n' "$exports" | grep -c '^LEYLINE_' || true)"
    leaked="$(printf '%s\n' "$exports" \
        | grep -E '^(SSL|TLS|CRYPTO|EVP|X509|BIO|ASN1|RSA|EC|BN|ERR|OPENSSL)_' || true)"
    [[ "$prefixed" -gt 0 ]] || die "no LEYLINE_-prefixed exports in $native_dir — the prefixed build did not run"
    [[ -z "$leaked" ]] || die "unprefixed BoringSSL exports remain in $native_dir:
$leaked"
    ok "$prefixed prefixed exports, no unprefixed BoringSSL exports"
fi

# --- tell the maintainer what code still needs touching ---------------------
if [[ $registered_build == 0 || $registered_lib == 0 ]]; then
    step "MANUAL STEP: register the target"
    cat <<EOF
The bundle is in place, but $triple still needs to be taught to the crate:

  1. $sys_dir/build.rs — add "$triple" to SUPPORTED_TARGETS.
  2. $sys_dir/src/lib.rs — add a cfg include! arm for it and add it to the
     compile_error! / cfg(not(any(...))) target list. Use the existing arms as
     a template; pick the right (target_arch, target_os, target_env).

Then verify: cargo build --target $triple
And run the live peet tests before committing fingerprint-path changes.
EOF
else
    ok "target already registered in build.rs + lib.rs — run: cargo build --target $triple"
fi
