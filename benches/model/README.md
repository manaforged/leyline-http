# Concurrency models

Each test explores every thread interleaving of one protocol with [loom](https://docs.rs/loom). A model restates the protocol's atomic and wake steps. It does not compile the shipped code, so change the model when the protocol changes.

```sh
cargo test --release
```

| Model | Protocol | Source |
| --- | --- | --- |
| `command_queue` | A sender waits for room in the HTTP/2 command queue | `h2/client/driver/protocol.rs` |
| `room` | A blocked caller waits for host limit room | `core/config/host_gate.rs` |

`the_earlier_wake_rule_loses_a_sender` keeps the protocol as it was before a fix, to show that the model finds that bug. Loom aborts the process on the deadlock, so the test is ignored by default:

```sh
cargo test --release --test command_queue -- --ignored
```
