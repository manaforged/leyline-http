# Qualification

Each subcommand drives leyline through its public API against a local fault-injecting origin. It exits 1 when an invariant fails, printing the invariant and the seed, and writes a JSON receipt.

```sh
cargo run --release -- chaos --seed 1 --seconds 30 --out chaos.json
cargo run --release -- cancel --seed 1 --seconds 30 --out cancel.json
```

`edge` needs an origin in its own process, so the origin's memory and CPU stay out of the client's numbers:

```sh
qualify origin --port 8000 --bad-first 20 --bad-count 50 --seed 1 &
qualify edge --port 8000 --mode shared --rate 100000 --hosts 2000 --bad-first 20 --bad-count 50 --seconds 60 --out edge.json
```

The same seed gives the same fault for each request id.

## Run the scenarios

```sh
python3 run.py                     # build, then every quick scenario in parallel
python3 run.py --only 'hostile-'   # a subset, by regular expression
python3 run.py --force             # run scenarios that already passed on this binary
python3 run.py --tier full --origin-cpus 3-7,19-23 --client-cpus 8-15,24-31
```

The quick tier splits the available CPUs into four-CPU slots and runs one scenario in each. It uses small rates and loose latency limits, so it catches failed requests, hangs, leaks and large regressions. It does not measure performance.

The full tier runs one scenario at a time on all CPUs, at the rates and limits that receipts quote. Build and read its numbers on a quiet machine. Pass `--preload /path/to/libjemalloc.so.2` for the 300,000 request/s scenarios: under glibc's allocator they fail their latency limits on 15 to 40 ms stalls that do not occur under jemalloc.

Scenarios behind nginx run when `nginx` is on the `PATH` or `--nginx` names a binary. The fault origin picks a fault from the address the client called, so nginx opens a new upstream connection for each request and these runs use lower rates.

`differential.py` sends the same response bytes to Leyline, hyper, curl and optionally Go, then compares status and body. Its table holds cases with outcomes taken from RFC 9112 section 6.3, and it exits 1 when Leyline breaks one. `--corpus` adds the fuzz corpus for `h1_head` and `h1_chunked`, and the report lists every split between clients.

```
go build -o target/go-differential differential/client.go
python3 differential.py --go-client target/go-differential --corpus ../../fuzz/corpus
```

A scenario that gave the expected result is skipped until the binary or the scenario changes, so a stopped run resumes where it ended. Each scenario runs in its own network namespace when `sudo -n` and `unshare` allow it. Receipts and logs go to `results/<tier>/`.

## Edge scenarios

Run the client and origin on separate L3 caches. Run each scenario in a private network namespace with `somaxconn` and `tcp_max_syn_backlog` raised, so connection tracking and TIME_WAIT state from another job cannot fail it. A receipt records `listen_overflows` and `syn_retransmits`; a run with an accept queue overflow is void.

| Scenario | Client flags | Origin flags |
| --- | --- | --- |
| Bad hosts in the long tail | `--bad-first 20 --bad-count 50 --max-pending 1024` | `--bad-first 20 --bad-count 50` |
| One popular host stalls | `--bad-first 0 --bad-count 1 --max-pending 1024` | `--bad-first 0 --bad-count 1 --stall-only` |
| 20,000 or 200,000 hosts | `--hosts N --max-connections 2N --max-pending 1024` | |
| Goodput past capacity | `--bad-count 0 --overload --retries 0 --deadline-ms 1000 --min-goodput N` | `--bad-count 0 --capacity-permits P --service-us U --queue-limit Q` |
| Retry storm after a shock | `--bad-count 0 --overload --shock 3 --retries 3 --deadline-ms 1000` | `--bad-count 0 --capacity-permits P --service-us U --queue-limit Q` |

`hostile`, `reconnect` and `idle` run over TLS and HTTP/2 against a peer in its own process:

```sh
qualify hostile-origin --port 8443 --attack ping --ca ca.der &
qualify hostile --port 8443 --attack ping --ca ca.der --seconds 15 --out hostile.json

qualify mux-origin --port 8443 --hosts 2000 --restart-at 16 --ca ca.der &
qualify reconnect --port 8443 --hosts 2000 --rate 100000 --ca ca.der --seconds 30 --out reconnect.json

qualify mux-origin --port 8443 --ca ca.der &
qualify idle --port 8443 --streams 100000 --rate 20000 --ca ca.der --seconds 45 --out idle.json
```

Attacks: `ping`, `settings`, `empty-data`, `empty-continuation`, `growing-continuation`, `rapid-reset`, `zero-window`, `window-flood`, `huge-headers`, `slow-body`, `goaway-loop`, `data-flood`.

The receipt holds healthy-host p99, RSS and client CPU for each second, so a spike has a time and a cause.
