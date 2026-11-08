# How this suite is built and scored

The scorer treats every library the same. The rules below apply to every
program in every language.

- Each program is a complete file that a reader can run. Setup lines count:
  `#[tokio::main]`, `asyncio.run(main())`, imports, and `Ok(())`.
- `rustfmt`, `black`, and `prettier` format each program before it is scored,
  with default settings, so line counts do not depend on hand layout.
- `lexer.py` applies one regular-expression lexer to all three languages.
  A string is one token, so a hand-built multipart body looks cheaper in
  tokens than it is. Read lines and code as well as tokens.
- Each program runs against `fixture.py`. A program passes only if it exits 0,
  prints the expected output, and the server's own checks agree.
- A task with no library support is written by hand with that library. The
  `native` table in `sources.toml` records which programs are hand-rolled and
  which extra package each one uses. Extra packages come out of the import
  statements, not from that table.
- Concepts are the distinct identifiers a program uses in member, path,
  keyword-argument, or import position that also appear in the public surface
  of the packages it imports. `surface.py` extracts that surface with the same
  rules per language. Names that come from the standard library are removed.
  This count is an approximation.
- The fixture uses one plain-HTTP port and HTTP/1.1. It does not test TLS,
  HTTP/2, or a CONNECT tunnel to a TLS origin.
- Task 3 uses `localhost` because aiohttp's default cookie jar refuses cookies
  from an IP address, as its documentation says. That is a property of the
  fixture, not of a real site.

## Run

```sh
python surface.py
python run.py
python footprint.py <leyline python package dir> <leyline node package dir>
python run.py --table
```

`run.py` writes `results.json` and `results.md`. `footprint.py` writes
`footprint.json`. Cold-build timings share the machine with other work, so
each is the median of three runs and can be noisy.

## Setup

- Python: `pip install -r python/requirements.txt`, and the Leyline binding.
- Node: `npm install` in `node/`, a `leyline-http` link in `node/node_modules`,
  and `node builtin-names.mjs` in `node/`.
- Rust: each directory in `rust/` is its own Cargo project with its own lock
  file. `rust/leyline-http` takes Leyline from this repository by path.
