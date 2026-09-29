# Supported platforms

Leyline builds BoringSSL from source when `leyline-bssl-sys` compiles. It
supports six targets:

| Target | Notes |
| --- | --- |
| `x86_64-unknown-linux-gnu` | glibc Linux |
| `aarch64-unknown-linux-gnu` | glibc Linux |
| `x86_64-unknown-linux-musl` | musl Linux, static binaries |
| `aarch64-unknown-linux-musl` | musl Linux, static binaries |
| `aarch64-apple-darwin` | Apple silicon macOS |
| `x86_64-pc-windows-msvc` | MSVC toolchain |

The build needs these tools:

- CMake 3.22 or later.
- A C and C++ compiler: Xcode Command Line Tools on macOS, GCC or Clang on
  Linux, and the MSVC build tools on Windows.
- libclang, because `bindgen` generates the bindings at build time.
- NASM on Windows, for the BoringSSL assembly.
- Git. Every build from source runs `git apply` to add Leyline's patches,
  unless `LEYLINE_BSSL_ASSUME_PATCHED` is set. A source checkout also uses Git
  to fetch the BoringSSL submodule.

On Windows, install Visual Studio Build Tools with the C++ workload, which
includes CMake. Install LLVM and NASM. Then:

1. Put the LLVM `bin` directory, the NASM directory, and the CMake `bin`
   directory on `PATH`.
2. Set `LIBCLANG_PATH` to the LLVM `bin` directory, so `bindgen` finds
   `libclang.dll`.

A Developer Command Prompt is not required.

The first build compiles BoringSSL. Later builds reuse it.

## musl

BoringSSL has C++ code, so a musl build needs a musl C and C++ toolchain.
The `musl-gcc` wrapper from the `musl-tools` package has no C++ compiler.
Use a full cross toolchain, such as the musl.cc builds or the toolchain that
`cross` ships.

1. Put the toolchain on `PATH`, for example
   `x86_64-linux-musl-cross/bin`.
2. Set the Rust linker to the musl compiler:

   ```sh
   export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc
   export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc
   ```

3. Build:

   ```sh
   cargo build -p leyline-http --target x86_64-unknown-linux-musl
   ```

The build script finds `<arch>-linux-musl-gcc` and `<arch>-linux-musl-g++` on
`PATH`. To use another compiler, set `CC_<target>` and `CXX_<target>`. To
give `bindgen` a musl sysroot, set `LEYLINE_BSSL_SYSROOT`. The linker links
the musl `libstdc++` statically.

## Unsupported targets

On any other target, `leyline-bssl-sys` stops the build with an error that
names the supported targets. This includes Intel macOS
(`x86_64-apple-darwin`), 32-bit targets, and `windows-gnu`. See
[Features and targets](features-and-targets.md#other-targets).

The platform that a session claims is separate from the target you build
for. A Linux build can claim Windows or macOS. See
[Choose a platform](sessions.md#choose-a-platform).

Read the [Profile reference](profiles.md) for each bundled profile and the capture it comes from.
