#!/usr/bin/env bash
# Produce + check in a prebuilt BoringSSL bundle for the *host* target.
#
# Leyline's leyline-bssl-sys crate links checked-in prebuilt BoringSSL so most
# developers never need CMake/Perl/libclang/Go. Adding a new target means
# running the in-repo source build once on a machine of that target and copying
# the resulting static libs + bindgen output into the repo. This script
# automates the build-and-strip steps, and works for any host (including the
# targets we don't ship yet: x86_64-apple-darwin, aarch64-unknown-linux-gnu).
#
# RUN THIS ON THE TARGET HOST — BoringSSL is not cross-compiled here.
#
# Prereqs on the host: a Rust toolchain plus the source-build deps
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

sys_dir="crates/leyline-bssl-sys"

# --- host target triple ----------------------------------------------------
triple="$(rustc -vV 2>/dev/null | awk '/^host:/{print $2}')"
[[ -n "$triple" ]] || die "could not determine host target from 'rustc -vV' — is Rust installed?"

case "$triple" in
    *-msvc)  is_msvc=1; crypto_lib="crypto.lib"; ssl_lib="ssl.lib" ;;
    *)       is_msvc=0; crypto_lib="libcrypto.a"; ssl_lib="libssl.a" ;;
esac

native_dir="$sys_dir/native/$triple/lib"
bindings_file="$sys_dir/src/bindings/$triple.rs"

step "host target: $triple"
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

git submodule update --init "$sys_dir/deps/boringssl"

restore_build() { sed -i 's|^build = "build/main.rs"|build = "build.rs"|' "$sys_dir/Cargo.toml"; }
build_root="$(mktemp -d)"
trap 'restore_build; rm -rf "$build_root"' EXIT
sed -i 's|^build = "build.rs"|build = "build/main.rs"|' "$sys_dir/Cargo.toml"

( cd "$sys_dir" && CARGO_TARGET_DIR="$build_root/target" cargo build --release --features source-build ) \
    || die "leyline-bssl-sys source build failed — install cmake/perl/go/libclang and retry"
restore_build

src_out="$(dirname "$(find "$build_root/target/release" -name bindings.rs 2>/dev/null | head -1)")"
[[ -n "$src_out" && -d "$src_out" ]] || die "could not locate leyline-bssl-sys OUT_DIR after build"

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
The bundle is in place, but $triple still needs to be taught to the crate:

  1. $sys_dir/build.rs — add "$triple" to SUPPORTED_TARGETS.
  2. $sys_dir/src/lib.rs — add a cfg include! arm for it and add it to the
     compile_error! / cfg(not(any(...))) target list. Use the existing arms as
     a template; pick the right (target_arch, target_os, target_env).

Then verify: cargo build --target $triple
And add a live test pass per CONTRIBUTING.md before committing fingerprint-path changes.
EOF
else
    ok "target already registered in build.rs + lib.rs — run: cargo build --target $triple"
fi
