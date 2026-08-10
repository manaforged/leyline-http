# cfnetwork-capture

Permanent recapture rig for the **CFNetwork / app-identity wire fingerprint**


A CFNetwork ClientHello is a property of the **OS framework version**, not of
the app — any URLSession client on a given iOS/macOS build produces the same
handshake. So the probe is a 20-line URLSession GET, and the capture rig is
the tool that turns that fact into profile material for future iOS releases.

## Layout

- `probe/` — SwiftPM executable, URLSession GET. Builds for macOS and iOS
  Simulator (`--triple arm64-apple-ios-simulator`). Accepts any server cert
  (trusts only our own capture server; the ClientHello precedes cert
  evaluation anyway).
- `server/` — Go TLS-terminating HTTP/2 capture target. Logs every client
  frame in order via `x/net/http2.Framer`: SETTINGS (per-setting order),
  WINDOW_UPDATE, HEADERS (HPACK-decoded pseudo-header order), PING,
  RST_STREAM, GOAWAY. Responds 200 so the exchange completes.
- `capture.sh` — orchestrator: boots simulator → builds probe → starts server
  → `sudo tcpdump` on loopback → runs probe N times → tshark ClientHello dump
  → evidence pack.
- `out/` — evidence packs (`<date>-<platform>-<hostbuild>/`), gitignored.

## Usage

```sh
./capture.sh --platform ios18 --runs 3        # iOS 18.6 simulator
./capture.sh --platform macos --runs 3        # macOS host
./capture.sh --platform ios26 --runs 3        # once the iOS 26 runtime is installed
```

Requires interactive sudo (tcpdump on loopback needs root). Outputs under
`out/<name>/`:

- `raw.pcap` — full loopback capture
- `server.log` — H2 frame-order log
- `clienthello.txt` — order-preserving ClientHello field dump (ciphers,
  groups, sigalgs, ALPN, extension types — GREASE positions visible as
  `0x0a0a`-family values)
- `meta.txt` — platform, OS build, runtime, exact `captured_against` value
- `server-cert.crt/.key` — persisted server identity when needed

## Clean-room

Shipped profiles derive only from these captures.
specs have exactly
one sanctioned use: a neighborhood cross-check note that lives in the lab,
never in leyline. See the plan's Contamination rules.

## Notes

- Loopback capture means our own server's TLS termination is the only
  post-handshake traffic — captures are clean by construction (no ECH, no
  proxies).
- The simulator boot is headless (`simctl boot`); no Simulator.app needed.
- `capture.sh` requires interactive sudo; `sudo -v` beforehand for scripting.
