# Qualification

This page lists what is known about Leyline's behaviour under failure and load, and how each fact was established. Each row says which of five kinds of evidence backs it:

| Kind | Meaning | Limit |
| --- | --- | --- |
| Proved | A Kani harness checks every input inside a stated bound | Says nothing outside the bound, or about sockets, time or load |
| Model-checked | A loom model in `benches/model` explores every thread interleaving of one protocol | The model restates the protocol's atomic and lock steps; it does not compile the shipped code |
| Tested | A scenario in `benches/qualification` exits 1 when the invariant breaks | Holds for the inputs and durations that ran |
| Measured | A number from one machine, with its receipt | Holds for that machine, build and load only |
| Not claimed | Not run, or run and failed | |

The scenarios are commands a user can rerun: see [`benches/qualification/README.md`](benches/qualification/README.md). `qualify.sh` reruns the full tier, the parser differential, every fuzz target and the 1-hour soak, and exits 1 when any step fails.

## Proved

`cargo kani --no-default-features --lib -Z stubbing` runs 11 harnesses.

| Harness | Property |
| --- | --- |
| `a_chunk_size_follows_rfc_9112_section_7_1` | Chunk sizes up to 17 bytes parse as RFC 9112 section 7.1 defines, and overflow is an error |
| `a_content_length_follows_rfc_9110_section_8_6` | Content-Length values up to 7 bytes parse as RFC 9110 section 8.6 defines |
| `a_content_length_overflow_is_exact_at_u64_max` | The largest accepted Content-Length is `u64::MAX` |
| `a_body_limit_admits_exactly_what_fits` | A body limit accepts a body exactly when it fits |
| `a_body_limit_room_is_exactly_what_is_left` | The room a body limit reports is the exact remainder |
| `every_hpack_integer_round_trips` | HPACK integers encode and decode to the same value |
| `arbitrary_bytes_never_decode_past_the_hpack_limit` | No byte string decodes to an HPACK integer past the limit |
| `a_frame_header_round_trips_and_ignores_the_reserved_bit` | HTTP/2 frame headers round-trip, and the reserved bit is ignored |
| `a_server_setting_follows_rfc_9113_section_6_5_2` | Server SETTINGS values are accepted or refused as RFC 9113 section 6.5.2 defines |
| `every_stream_transition_follows_rfc_9113_section_5_1` | Every HTTP/2 stream state transition matches RFC 9113 section 5.1 |
| `a_flow_window_is_exact_and_never_passes_the_protocol_maximum` | Flow-control windows are exact and stay at or below 2^31 - 1 |

## Model-checked

`cargo test --release` in `benches/model` runs each loom model.

| Model | Property |
| --- | --- |
| `room` | A caller blocked on a host limit sees room freed while it registers |
| `reaper` | While the pool holds a connection, exactly one idle reaper runs, also when connections return while the reaper retires |

## Tested

Each scenario fails when the stated invariant breaks.

| Area | Scenarios | Fails when |
| --- | --- | --- |
| Faults in the origin | `chaos`: resets, GOAWAY, stalls, truncated bodies, slow drip, malformed frames | A request has no outcome by its deadline, has two outcomes, or exceeds its retry bound |
| Cancellation | `cancel`: request futures dropped at random points | Connections, tasks or memory leak, or a later request reads another response's bytes |
| Isolation | `q10`, `q11`, `q15`: 2,000 to 200,000 hosts, some failing or stalling | Healthy-host p99 during faults passes 1.5 times its value before them |
| Overload | `q16`, `q17`, `q18`: offered load past the origin's capacity, and a retry storm | Requests outlive their deadline, goodput falls toward zero, or goodput stays low after the shock |
| Connection change | `life-*`: address loss, DNS move, certificate rotation, graceful shutdown | A request fails beyond the changed host, or a finished request is lost |
| Hostile HTTP/2 peers | `hostile-*`: PING, RST, SETTINGS, empty frame and CONTINUATION floods, zero window, huge headers | CPU or memory grows without bound, or one peer slows another host |
| Real origin | `q28-nginx`: the edge workload through nginx | Any failure the origin did not cause |
| Response parsing | `differential.py`: 61 cases from RFC 9112 section 6.3 | Leyline reads a case differently from the RFC |
| Crashes | 11 fuzz targets, including `h2_session`, which drives the client with server frames | A crash or a hung request |

Two defaults come from these scenarios:

- A host accepts 1,024 waiting HTTP/1.1 requests; the next request fails with `Overloaded`. `SessionBuilder::max_pending_per_host(None)` removes the limit.
- An address that fails to connect 5 times in a row moves to the back of the dial order for 30 seconds times the number of times it was ejected, up to 300 seconds. A successful connection clears its record.

## Measured

One Ryzen 9 9950X3D. Client and origin run on separate L3 caches, each scenario in its own network namespace, with jemalloc in the client. Receipts are JSON files written by each scenario.

| Scenario | Result |
| --- | --- |
| Peak rate | 500,000 req/s at p99 7.9 ms on 9.1 logical CPUs. Saturates near 620,000 req/s |
| Core scaling | About 40,000 req/s per logical CPU through 8 CPUs |
| 1-hour soak at 100,000 req/s | 0 failures. Per 10-minute window: median p99 4.3 to 4.4 ms, RSS 579 to 580 MiB, 5,745 to 5,811 open fds |
| 200,000 hosts | Connection reuse 0.99932, p99 4.7 ms, 2.6 GiB |
| Overload at 2.5 times origin capacity, 5 minutes | 0 failures, 0 requests past their deadline, 225 MiB peak, p99 back to 4.6 ms after |
| 10,000 slow readers on large bodies | 0 failures, 295 KiB per reader |
| 1 GiB download and upload | 316 KiB RSS growth |
| Graceful shutdown under 2,000 streams | Quiet in 2 ms, 0 wrong digests |
| Fuzzing | 676 million executions across 11 targets, 0 crashes |
| Parser differential | 0 of 61 RFC cases read wrongly |

## Not claimed

- Production traffic. Nothing here ran outside one machine, except a two-machine run over Wi-Fi at 5,000 req/s.
- A soak longer than 1 hour.
- Thread interleavings in HTTP/2 stream accounting and token refresh: there is no model for them.
- The HTTP/2 command queue batch drain: its loom model (`command_queue`) did not finish exploring in 2 h 45 min, so no result is claimed.
- A client conformance suite. h2spec tests HTTP/2 servers, so it does not apply to a client; the parser differential and the fuzz targets cover the client side.
- Healthy-host p99 at 300,000 req/s while 50 of 2,000 hosts fail: the 1.5 times limit fails in about 1 of 3 runs, in the second the faults start.
- Strict field names: Leyline drops response header fields with invalid names or values instead of failing the response.
