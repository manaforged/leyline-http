#!/usr/bin/env bash
# Produce + check in a prebuilt BoringSSL bundle for the *host* target.
#
# Leyline's local btls-sys shim links checked-in prebuilt BoringSSL so most
# developers never need CMake/Perl/libclang/Go. Adding a new target means
# running the upstream source build once on a machine of that target and
# copying the resulting static libs + bindgen output into the repo. This
# script automates the copy-and-strip steps the btls-sys README spells out
# for Linux and macOS, and works for any host (including the targets we don't
# ship yet: x86_64-apple-darwin, aarch64-unknown-linux-gnu).
#
# RUN THIS ON THE TARGET HOST — BoringSSL is not cross-compiled here.
#
# Prereqs on the host: a Rust toolchain plus the upstream source-build deps
#   (cmake, perl, go, and libclang — `LIBCLANG_PATH` may need setting on macOS).
#
# Usage:
#   ./scripts/package-bssl.sh            # build + install bundle for the host target
#   ./scripts/package-bssl.sh --check    # only report what's missing, build nothing
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
export PATH="$HOME/.cargo/bin:$PATH"

step() { printf '\n\033[1;34m== %s ==\033[0m\n' "$*"; }
ok()   { printf '\033[1;32mOK %s\033[0m\n'  "$*"; }
warn() { printf '\033[1;33mWARN %s\033[0m\n' "$*"; }
die()  { printf '\033[1;31mERROR %s\033[0m\n' "$*" >&2; exit 1; }

check_only=0
[[ "${1:-}" == "--check" ]] && check_only=1

# --- host target triple ----------------------------------------------------
triple="$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')"
[[ -n "$triple" ]] || die "could not determine host target from 'rustc -vV' — is Rust installed?"

case "$triple" in
    *-msvc)  is_msvc=1; crypto_lib="crypto.lib"; ssl_lib="ssl.lib" ;;
    *)       is_msvc=0; crypto_lib="libcrypto.a"; ssl_lib="libssl.a" ;;
esac

native_dir="crates/btls-sys/native/$triple/lib"
bindings_file="crates/btls-sys/src/bindings/$triple.rs"

step "host target: $triple"
have_libs=0
[[ -f "$native_dir/$crypto_lib" && -f "$native_dir/$ssl_lib" ]] && have_libs=1
have_bindings=0
[[ -f "$bindings_file" ]] && have_bindings=1
registered_build=0
grep -q "\"$triple\"" crates/btls-sys/build.rs && registered_build=1
registered_lib=0
grep -q "$triple" crates/btls-sys/src/lib.rs && registered_lib=1

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

# --- build upstream btls-sys from source in a throwaway crate ---------------
# The temp crate lives outside this workspace, so the local
# [patch.crates-io] btls-sys override does NOT apply and we get the real
# source build that produces the static libs + bindings.rs we want.
step "building upstream btls-sys from source (this can take 5-15 min)"
for tool in cmake perl go; do
    command -v "$tool" >/dev/null 2>&1 || warn "$tool not found on PATH — the source build will likely fail without it"
done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
btls_ver="$(awk -F'"' '/^btls-sys *= *"/{print $2; exit}' Cargo.toml)"
btls_ver="${btls_ver:-0.5}"
cat > "$tmp/Cargo.toml" <<EOF
[package]
name = "btls-rebuild"
version = "0.0.0"
edition = "2024"

[dependencies]
btls-sys = "$btls_ver"

[lib]
path = "src/lib.rs"
EOF
mkdir -p "$tmp/src"
: > "$tmp/src/lib.rs"

( cd "$tmp" && CARGO_TARGET_DIR="$tmp/target" cargo build --release ) \
    || die "upstream btls-sys source build failed — install cmake/perl/go/libclang and retry"

src_out="$(ls -d "$tmp"/target/release/build/btls-sys-*/out 2>/dev/null | tail -1)"
[[ -n "$src_out" && -d "$src_out" ]] || die "could not locate btls-sys OUT_DIR after build"

# --- install libs -----------------------------------------------------------
step "installing artifacts into the repo"
mkdir -p "$native_dir"
cp "$src_out/build/$crypto_lib" "$native_dir/$crypto_lib"
cp "$src_out/build/$ssl_lib"    "$native_dir/$ssl_lib"
if [[ $is_msvc == 0 ]]; then
    case "$triple" in
        *-apple-*) strip -S "$native_dir/$crypto_lib" "$native_dir/$ssl_lib" ;;
        *)         strip --strip-debug "$native_dir/$crypto_lib" "$native_dir/$ssl_lib" ;;
    esac
fi
ok "native libs -> $native_dir/"

cp "$src_out/bindings.rs" "$bindings_file"
ok "bindings -> $bindings_file"

# --- tell the maintainer what code still needs touching ---------------------
if [[ $registered_build == 0 || $registered_lib == 0 ]]; then
    step "MANUAL STEP: register the target"
    cat <<EOF
The bundle is in place, but $triple still needs to be taught to the shim:

  1. crates/btls-sys/build.rs — add "$triple" to SUPPORTED_TARGETS.
  2. crates/btls-sys/src/lib.rs — add a cfg include! arm for it and add it to
     the compile_error! / cfg(not(any(...))) target list. Use the existing
     arms as a template; pick the right (target_arch, target_os, target_env).

Then verify: cargo build --target $triple
And add a live test pass per CONTRIBUTING.md before committing fingerprint-path changes.
EOF
else
    ok "target already registered in build.rs + lib.rs — run: cargo build --target $triple"
fi
