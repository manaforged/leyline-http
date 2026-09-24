# Supported platforms

Leyline builds BoringSSL from source when `leyline-bssl-sys` compiles. It
supports four targets:

| Target | Notes |
| --- | --- |
| `x86_64-unknown-linux-gnu` | glibc Linux |
| `aarch64-unknown-linux-gnu` | glibc Linux |
| `aarch64-apple-darwin` | Apple silicon macOS |
| `x86_64-pc-windows-msvc` | MSVC toolchain |

The build needs these tools:

- CMake 3.22 or later.
- A C and C++ compiler: Xcode Command Line Tools on macOS, GCC or Clang on
  Linux, and the MSVC build tools on Windows.
- libclang, because `bindgen` generates the bindings at build time.
- NASM on Windows, for the BoringSSL assembly.

The first build compiles BoringSSL. Later builds reuse it.

## Unsupported targets

On any other target, `leyline-bssl-sys` stops the build with an error that
names the supported targets. This includes Intel macOS
(`x86_64-apple-darwin`), musl Linux, 32-bit targets, and `windows-gnu`. See
[Features and targets](features-and-targets.md#other-targets).

The platform that a session claims is separate from the target you build
for. A Linux build can claim Windows or macOS. See
[Choose a platform](sessions.md#choose-a-platform).
