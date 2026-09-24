# Supported platforms

Leyline links a prebuilt BoringSSL. The packaged crate supports four targets:

| Target | Notes |
| --- | --- |
| `x86_64-unknown-linux-gnu` | glibc 2.34 or later |
| `aarch64-unknown-linux-gnu` | glibc Linux |
| `aarch64-apple-darwin` | Apple silicon macOS |
| `x86_64-pc-windows-msvc` | MSVC toolchain |

The `x86_64-unknown-linux-gnu` library is built against glibc 2.34, so a
binary that links it needs glibc 2.34 or later at run time. Ubuntu 20.04 has
glibc 2.31 and is not supported. Ubuntu 22.04, Debian 12, and RHEL 9 meet the
minimum.

## Unsupported targets

On any other target, `leyline-bssl-sys` stops the build with a
`compile_error!` that names the supported targets. This includes Intel macOS
(`x86_64-apple-darwin`), musl Linux, 32-bit targets, and `windows-gnu`. To
add a target you need native BoringSSL libraries, generated bindings, and a
target entry in `leyline-bssl-sys`. See
[Features and targets](features-and-targets.md#other-targets).

The platform that a session claims is separate from the target you build
for. A Linux build can claim Windows or macOS. See
[Choose a platform](sessions.md#choose-a-platform).
