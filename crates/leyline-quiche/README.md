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
  decoder stream instructions. Table entries are shared, so Duplicate and
  indexed references do not copy them. An integer that overflows on the
  encoder stream is a QPACK_ENCODER_STREAM_ERROR. The connection closes with
  H3_EXCESSIVE_LOAD when more than 64 KiB of decoder stream instructions wait
  for flow-control credit.
- `h3::Config::set_field_section_limit` sets a local limit on decoded header
  sections and HEADERS frames. It applies even when SETTINGS does not
  advertise SETTINGS_MAX_FIELD_SECTION_SIZE.
- A RESET_STREAM on a stream that waits for QPACK inserts cancels the stream:
  the decoder sends Stream Cancellation and `poll` returns `Event::Reset`.
  `h3::Connection::cancel_stream` frees the blocked slot of a stream that the
  application drops.
- QUIC version 2 (RFC 9369): the Initial salt, the `quicv2` HKDF labels, the
  long header packet types, and the Retry integrity key and nonce.
  `Config::set_compatible_versions` lets a client follow a server that
  switches to another listed version (RFC 9368 compatible version
  negotiation) once a packet in that version decrypts. Version Negotiation
  picks only a listed version. The client writes its current version as the
  Chosen Version, stops 0-RTT after a switch, and checks the server's
  `version_information` (RFC 9368 section 4). A mismatch closes the
  connection with VERSION_NEGOTIATION_ERROR (0x11).
- `Config::set_initial_datagram_size` sets the size of client datagrams that
  carry Initial packets during the handshake. First-flight Initial packets
  are padded with PADDING frames to that size. The size is at most 1232
  bytes over IPv6. After a loss timeout with no packet received, Initials
  fall back to 1200 bytes.
- RESET_STREAM_AT is read as RESET_STREAM, and a Reliable Size larger than the
  Final Size is a FRAME_ENCODING_ERROR. ACK_FREQUENCY and IMMEDIATE_ACK are
  accepted and ignored. Each frame type is accepted only in 1-RTT packets, and
  only when the local transport parameter plan advertises its extension.

## License

BSD-2-Clause, inherited from upstream `cloudflare/quiche`. Local changes
by Manaforge Technologies, LLC are released under the same license. Full attribution
in the workspace [`NOTICE`](https://github.com/manaforged/leyline-http/blob/main/NOTICE).
