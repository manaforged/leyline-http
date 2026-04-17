# Leyline fuzz targets

Uses [`cargo-fuzz`](https://rust-fuzz.github.io/book/cargo-fuzz.html) with
libFuzzer. Requires nightly Rust to run.

## Install

```bash
cargo install cargo-fuzz
rustup toolchain install nightly --profile minimal --component rust-src
```

## Targets

| Target | What it hits |
|---|---|
| `hpack_integer` | HPACK decoder entry point — varint, huffman, table refs through `Decoder::decode_header_block` |
| `hpack_header_block` | Two consecutive HPACK decodes on the same decoder to hit dynamic-table state transitions |
| `h2_frame` | `Frame::parse` for every frame type — malformed lengths, unknown types, truncated payloads |
| `cookie_set` | `CookieJar::store_set_cookie` — malformed Set-Cookie headers, attribute parsing |


### Seeds committed

Each corpus directory contains hand-crafted seeds for the target plus
libFuzzer-discovered entries from the short run:

- `hpack_integer/`: static-table indexed fields (GET/POST/:path/:scheme/:status),
  literal-with-indexing authority, literal-new-name, pathological
  varint spill (`0xFF, 0x81, 0x01`), huffman-encoded path, dynamic-table
  size update (to 0 and to 4096), truncated literal, never-indexed literal.
- `hpack_header_block/`: add-then-reference dyn entry at idx 62,
  evict-then-reference, two valid pseudo-header blocks, empty-first-block.
- `h2_frame/`: one valid input per frame type (DATA, HEADERS±PRIORITY,
  PRIORITY, RST_STREAM, SETTINGS, SETTINGS ACK, PUSH_PROMISE, PING,
  PING ACK, GOAWAY, WINDOW_UPDATE, CONTINUATION), plus unknown type
  `0xFF` and a truncated frame.
- `cookie_set/`: simple `a=b`, full attribute strings, `Path`/`Domain`/
  `Expires`/`Max-Age`/`SameSite` (None/Lax), `__Host-` and `__Secure-`
  prefixes, quoted / malformed-quoted / empty / bare-name / unicode-attr
  values, pathological `Max-Age=99999999999999999999`, negative `Max-Age`,
  `Priority=High`, leading-whitespace, trailing-semicolons.

## To reproduce a run

```bash
cd fuzz
cargo +nightly fuzz run <target> -- -runs=10000000 -max_total_time=45
```

Swap `<target>` for one of `hpack_integer`, `hpack_header_block`,
`h2_frame`, `cookie_set`. Drop the `-runs`/`-max_total_time` flags for
an open-ended run. To minimize the corpus after a run:

```bash
cargo +nightly fuzz cmin <target>
```

If a crash is found libFuzzer writes it to `fuzz/artifacts/<target>/`.
Reproduce with:

```bash
cargo +nightly fuzz run <target> artifacts/<target>/crash-<hash>
```

## Scope

This is **defensive fuzzing** — finding panics, integer overflows, and
runaway allocations in the inbound parsing paths. It is not a
correctness suite; successful parses are not asserted against a spec
oracle. For correctness tests, see
[`crates/h2/tests/frame_roundtrip.rs`](../crates/h2/tests/frame_roundtrip.rs)
and the parse/encode round-trips in each frame module.
