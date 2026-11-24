# Throughput

Start with one shared multi-threaded Tokio runtime and one cloned `Session`.
When a single process needs maximum throughput or lowest tail latency, choose
between the three layouts below:

| Layout | Entry point | Best for | v0.3.0 loopback benchmark (`BENCHMARKS.md`) |
| --- | --- | --- | --- |
| Default async | `Session::builder().build()?` on multi-threaded Tokio | Most apps, shared services, single-origin HTTP/2 | 1.44M req/s H2 `8×256` (5.68 µs CPU/req) |
| Per-core async | `SessionBuilder::run_per_core(\|s\| ...)` | Many connections/hosts, lowest open-loop p99 latency, high-concurrency H1 | 921k req/s H1 `64par`; 28 µs p99 H1 `c64`, 78 µs p99 H2 `c8x256` |
| Blocking per thread | `SessionBuilder::build_blocking()?` | Sync threads, low-concurrency direct H1 | 423k req/s H1 `8par` (zero task switches on direct H1) |

## One runtime per core

`SessionBuilder::run_per_core` starts one single-threaded runtime per CPU core,
runs your closure with a core-local `Session`, and returns each core's result.
It blocks the calling thread until every core finishes, so call it from `main`
or a plain thread rather than async code.

| State under `run_per_core` | Scope |
| --- | --- |
| Cookie jar, HTTP cache, bearer token, HSTS store, `host_limits`, proxy pool, Digest nonces, DoH cache | Shared across all cores |
| Connection pool (`max_connections`, `max_connections_per_host`), `shutdown()` | Per core (connections never cross runtimes) |

```rust,no_run
use leyline::Session;

fn main() -> leyline::Result<()> {
    let pages = Session::builder().run_per_core(|session| async move {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..64 {
            let session = session.clone();
            tasks.spawn(async move {
                session.get("https://example.com/").bytes().await
            });
        }
        let mut done = 0;
        while let Some(page) = tasks.join_next().await {
            page.expect("request task panicked")?;
            done += 1;
        }
        leyline::Result::Ok(done)
    })?;
    let total: usize = pages.into_iter().collect::<leyline::Result<Vec<_>>>()?.into_iter().sum();
    println!("{total} pages");
    Ok(())
}
```

The closure's future does not need to be `Send`. Each request runs start to
finish on one thread, eliminating cross-thread wakeup jitter: in
[BENCHMARKS.md](https://github.com/manaforged/leyline-http/blob/main/BENCHMARKS.md),
`run_per_core` (`leyline-cores`) cuts open-loop HTTP/1.1 `c64` (20k req/s) p99
from 62 µs to 28 µs and HTTP/2 `c8x256` (50k req/s) p99 from 115 µs to 78 µs at
4.31 µs CPU/req.

The gain requires at least as many connections as cores. A workload that sends
everything over one HTTP/2 connection runs faster on a shared runtime.

## Check for I/O more often on a shared runtime

On a multi-threaded Tokio runtime with many concurrent requests, lowering
`event_interval` from the default 61 to 4 makes busy workers poll epoll/kqueue
more frequently so responses wait less in the kernel:

```rust,no_run
let runtime = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .event_interval(4)
    .build()?;
# Ok::<(), std::io::Error>(())
```

The gain is workload-dependent and trades slightly more CPU per request;
measure on your target workload and leave the default for low-concurrency or
sequential traffic.

## Manual per-thread runtimes

If you spawn your own single-threaded runtimes instead of using `run_per_core`,
build one `Session` inside each runtime rather than sharing a `Session` across
runtimes (HTTP/2 connection drivers stay on the runtime that opened them, so
sharing across runtimes reintroduces cross-thread wakeups).

Unlike `run_per_core`, completely separate `Session` instances share nothing:

- Divide per-host pool limits and `host_limits` by your core count to keep the same aggregate cap.
- Cookies, the HTTP cache, and HSTS stay local to each session. To seed each thread with the same cookies and identity, save once with `Session::save(path)` and load in each thread with `Session::load(path)`.

## Choose an allocator for sustained load

At sustained rates in the hundreds of thousands of requests per second, use
`jemalloc` rather than glibc's default allocator. In the
[QUALIFICATION.md](https://github.com/manaforged/leyline-http/blob/main/QUALIFICATION.md)
300,000 req/s / 2,000-host run, glibc produced 15–40 ms latency stalls when
faults ended and failed all 3 runs, while `jemalloc` passed all 3 paired runs
for roughly 10% more peak RSS. Leyline does not set a global allocator; set one
in your binary crate (for example with `tikv-jemallocator`):

```rust,ignore
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
```
