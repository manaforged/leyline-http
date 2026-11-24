# Supported platforms

This page lists the targets Leyline builds for and the tools the BoringSSL
build needs.

## Supported targets

`leyline-bssl-sys` compiles BoringSSL from source with the `cc` crate, from
the source lists BoringSSL ships in `gen/sources.json`, and uses the Rust
bindings committed for the target. A build needs no CMake, Go, Perl, or
libclang.

| Target | Notes |
| --- | --- |
| `x86_64-unknown-linux-gnu` | glibc Linux |
| `aarch64-unknown-linux-gnu` | glibc Linux |
| `x86_64-unknown-linux-musl` | musl Linux, static binaries |
| `aarch64-unknown-linux-musl` | musl Linux, static binaries |
| `aarch64-apple-darwin` | Apple silicon macOS |
| `x86_64-pc-windows-msvc` | MSVC toolchain |

On any other target, including Intel macOS (`x86_64-apple-darwin`), 32-bit
targets, and `windows-gnu`, the build stops with an error that names the
supported targets.

The platform a session claims is separate from the build target: a Linux
build can claim Windows or macOS. See
[Choose a platform](sessions.md#choose-a-platform).

## Build BoringSSL

By default, `leyline-bssl-sys` compiles the BoringSSL source shipped in the
crate. Any `LEYLINE_BSSL_*` variable also accepts a target-specific suffix
(`LEYLINE_BSSL_*_<TARGET>`).

| Variable | Purpose | Notes |
| --- | --- | --- |
| `LEYLINE_BSSL_SOURCE_PATH` | Compile an alternate BoringSSL source tree | Applies Leyline's patches in place; requires NASM on Windows |
| `LEYLINE_BSSL_PATH` | Link a prebuilt BoringSSL | Applies no patches; must carry Leyline's patches and `-DBORINGSSL_PREFIX=LEYLINE` (enable `bindgen` if headers differ) |
| `LEYLINE_BSSL_ASSUME_PATCHED` | Skip `git apply` for an already-patched tree | Requires `LEYLINE_BSSL_PATH` or `LEYLINE_BSSL_SOURCE_PATH` |
| `LEYLINE_BSSL_RUST_CPPLIB` | Override C++ standard library link | Default: `c++` on macOS, none on Windows, none on Linux when trial link passes (else `stdc++`) |
| `LEYLINE_BSSL_INCLUDE_PATH` | Custom header path for `bindgen` | Requires the `bindgen` feature of `leyline-bssl-sys` |
| `LEYLINE_BSSL_SYSROOT` | Sysroot passed to `bindgen` | Used when the `bindgen` feature is enabled |

The build needs:

- A C and C++ compiler: Xcode Command Line Tools on macOS, GCC or Clang on
  Linux, the MSVC build tools on Windows. The `cc` crate finds it, and
  `CC`, `CXX`, `CFLAGS`, and `CXXFLAGS` work as they do for any `cc` build.
- Git. Every build runs `git apply` for Leyline's patches unless
  `LEYLINE_BSSL_ASSUME_PATCHED` is set. A source checkout also uses Git to
  fetch the BoringSSL submodule.

In minimal Docker / CI builder images, install both the C++ toolchain and Git:

```sh
# Debian / Ubuntu (e.g. rust:1.96-slim)
apt-get update && apt-get install -y --no-install-recommends build-essential git

# Alpine (native musl build, e.g. rust:1.96-alpine)
apk add --no-cache build-base git
```

On Windows, the x86_64 assembly is NASM source. When NASM is on `PATH`, or
`NASM` names it, the build assembles it. Otherwise it links the objects that
ship in `prebuilt/x86_64-pc-windows-msvc/`, which CI assembles with NASM
2.16.01 from the same source and checks on every run.
`LEYLINE_BSSL_SOURCE_PATH` needs NASM, because the shipped objects match only
the bundled BoringSSL.

The build uses the macOS deployment target that Rust uses
(`MACOSX_DEPLOYMENT_TARGET`, 11.0 by default), honors `+crt-static` on
MSVC, and maps the source and output directories to `/build`, so the
libraries embed no local paths. The first build compiles BoringSSL; later
builds reuse it.

### Windows

Install Visual Studio Build Tools with the C++ workload. A Developer Command
Prompt is not required, and NASM is optional.

With the `bindgen` feature, also install LLVM and set `LIBCLANG_PATH` to
its `bin` directory, so `bindgen` finds `libclang.dll`.

### musl

BoringSSL has C++ code, so a musl build needs a musl C and C++
toolchain. The `musl-gcc` wrapper from `musl-tools` has no C++ compiler;
use a full cross toolchain, such as the musl.cc builds or the one `cross`
ships.

1. Put the toolchain on `PATH`, for example `x86_64-linux-musl-cross/bin`.
2. Set the Rust linker to the musl compiler:

   ```sh
   export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc
   export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc
   ```

3. Build:

   ```sh
   cargo build -p leyline-http --target x86_64-unknown-linux-musl
   ```

The build finds `<arch>-linux-musl-gcc` and `<arch>-linux-musl-g++` on
`PATH`; set `CC_<target>` and `CXX_<target>` to use another compiler. By
default the Linux trial link drops `libstdc++`; when a fallback or
`LEYLINE_BSSL_RUST_CPPLIB=stdc++` links it, musl links it statically.
