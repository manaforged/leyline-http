# leyline-quiche

A vendored fork of [`cloudflare/quiche`](https://github.com/cloudflare/quiche)
v0.30.0 that links against [`leyline-bssl`](https://crates.io/crates/leyline-bssl) instead
of `boring`, so [`leyline-http`](https://crates.io/crates/leyline-http)'s HTTP/2 and
HTTP/3 ClientHellos share the same patched BoringSSL build.

**Most users do not want this crate directly.** Depend on `leyline-http`;
import `leyline`. It pulls `leyline-quiche` in when the `http3` feature is on.

## Changes from upstream

- The crate links against `leyline-bssl`; BoringSSL symbols carry the
  `LEYLINE_` prefix.
- `Connection::cipher` returns the negotiated TLS 1.3 cipher suite name.
- `Connection::ssl_mut` exposes the TLS handle before the first packet.
- `connect_with_dcid` is always available.
- `Config::set_transport_params_plan` sets the exact client transport
  parameter list and order, including raw and GREASE parameters.
- `h3::Config::set_settings_plan` and `h3::Config::set_control_frames` set the
  exact SETTINGS frame and the frames that follow it on the control stream.
  `h3::Connection::send_priority_update_field_value` sends a PRIORITY_UPDATE
  frame with a caller-supplied field value.
- The QPACK decoder keeps a dynamic table (RFC 9204), with blocked streams and
  decoder stream instructions.
- RESET_STREAM_AT is read as RESET_STREAM; ACK_FREQUENCY and IMMEDIATE_ACK are
  accepted and ignored.

## License

BSD-2-Clause, inherited from upstream `cloudflare/quiche`. Local changes
by Manaforge Technologies, LLC are released under the same license. Full attribution
in the workspace [`NOTICE`](https://github.com/manaforged/leyline-http/blob/main/NOTICE).
