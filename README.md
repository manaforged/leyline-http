# Leyline

**Browser-accurate TLS fingerprinting for Rust.**

Leyline makes HTTP requests that are indistinguishable from real browsers at every network layer. Built on a patched BoringSSL fork with APIs no other TLS library exposes.

## Quick start

```rust
use leyline::Session;

// One line — Chrome 147 on Windows, perfect fingerprint
let session = Session::chrome()?;
let resp = session.navigate("https://example.com").await?;
println!("{}", resp.text());

// Or Firefox, Safari
let session = Session::firefox()?;
let session = Session::safari()?;

// Full control
let session = Session::builder()
    .browser(Browser::Chrome147)
    .platform(Platform::Linux)
    .proxy("socks5://user:pass@host:port")
    .grease_seed(b"seed")  // deterministic fingerprint
    .build()?;

let resp = session.post_json("https://api.example.com", &data).await?;
```

## What it controls

Every TLS library gives you cipher suites. Leyline controls these layers:

| Layer | What | Status |
|-------|------|--------|
| TLS ClientHello | Ciphers, extensions, curves, GREASE, key shares, ECH | Full |
| HTTP/2 SETTINGS | Frame values, ordering, WINDOW_UPDATE, pseudo-headers | Full |
| TCP SYN | TTL, MSS, window size, DF bit, window scale (JA4T) | Full |
| HTTP Headers | sec-fetch-*, accept, ordering via presets | Full |

## Browser profiles

Profiles are TOML data files — adding Chrome 148 is copy-paste, not code:

| Profile | JA4 | H2 Fingerprint |
|---------|-----|----------------|
| Chrome 147 | `t13d1516h2_8daaf6152771_d8a2da3f94cd` | `1:65536;2:0;4:6291456;6:262144\|15663105\|0\|m,a,s,p` |
| Chrome 146 | same | same |
| Chrome 145 | same | `1:65536;2:0;3:1000;4:6291456;6:262144\|15663105\|0\|m,a,s,p` |
| Firefox 148 | `t13d1716h2_5b57614c22b0_36c4f964cab1` | `1:65536;2:0;4:131072;5:16384\|12517377\|0\|m,p,a,s` |
| Safari 18 | `t13d2015h2_a09f3c656075_2a10bb534ace` | `2:0;3:100;4:2097152;8:1;9:1\|10420225\|0\|m,s,a,p` |

Plus: OkHttp Android 7/10, Safari iOS 15/17/18, Edge.

## Architecture

```
leyline              Facade — use this
  core               Session, request builder, response
  profile            TOML-driven browser profiles + registry
  h2                 HTTP/2 SETTINGS ordering + fingerprint
  tcp                JA4T TCP fingerprinting via socket2
  cookies            RFC 6265 cookie jar
```

## Adding a new browser version

1. Copy a TOML profile: `cp profiles/chrome/147.toml profiles/chrome/148.toml`
2. Edit version, user-agent, sec-ch-ua
3. Add 2 lines to the Browser enum

Zero core code changes. The profile is self-documenting.

## License

MIT OR Apache-2.0
