# Throughput

Most programs run one multi-threaded Tokio runtime and share one `Session`
across it. That is the simplest setup, and it is the one to start with. When
one process has to send as many requests per second as the machine allows,
run one single-threaded runtime per CPU core and give each runtime its own
`Session`.

## One runtime per core

`SessionBuilder::run_per_core` starts one single-threaded runtime per CPU
core, runs your future there with a session built from the builder, and returns
each core's output. The cores share one cookie jar, cache, token and HSTS list;
each core keeps its own connections, so connections never cross runtimes. It blocks the calling thread until every core
finishes, so call it from `main` or a plain thread, not from async code.

```rust,no_run
use leyline::Session;

fn main() -> leyline::Result<()> {
    let pages = Session::builder().run_per_core(|session| async move {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..64 {
            let session = session.clone();
            tasks.spawn(async move {
                session.get("https://example.com/").send().await?.bytes().await
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

The future does not need to be `Send`, because it never leaves its core.

Each request then runs start to finish on one thread. Tokio never moves it to
another worker or wakes a sleeping worker for it, and each session's
connection pool is used by one thread only.

The gain needs at least as many connections as cores. Requests that share one
HTTP/2 connection all run on the core that opened it, so a program that sends
everything to one host over one connection is faster on a shared runtime.
[BENCHMARKS.md](https://github.com/manaforged/leyline-http/blob/main/BENCHMARKS.md)
lists both layouts as `leyline` and `leyline-cores`.

## Keep one session per runtime

A session's connections, and the task that drives each HTTP/2 connection,
belong to the runtime that opened them. A session shared by several runtimes
brings back the cross-thread wakeups this layout removes, so build the
session inside the runtime that uses it.

Separate sessions share nothing:

- Each one opens its own connections, so a host sees up to one set of
  connections per core.
- `max_connections_per_host` and `host_limits` apply to each session on its
  own. Divide a limit by the number of cores to keep the same total.
- Cookies, the HTTP cache, and HSTS state stay in the session that received
  them. To start every core with the same cookies and identity, save one
  session with `session.save(path)` and build each core's session from
  `Session::load(path)`.

## When to keep the default

Keep one shared runtime when requests are a small part of the program's
work, when the program already runs on a multi-threaded runtime for other
reasons, or when all requests must share one cookie jar or one per-host
limit.
