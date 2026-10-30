A hook that receives one [`Event`](crate::trace::Event) per step of each request.

Add one with [`SessionBuilder::trace`](crate::SessionBuilder::trace). Match the variants you need and ignore the rest with `_`. [`Log`](crate::trace::Log) writes events to `tracing`, and [`Metrics`](crate::trace::Metrics) counts them.

```rust,no_run
use leyline::Session;
use leyline::trace::{Event, Trace};

struct Status;

impl Trace for Status {
    fn event(&self, event: &Event<'_>) {
        match event {
            Event::Head { status, .. } => println!("status {status}"),
            _ => {}
        }
    }
}

# fn run() -> leyline::Result<()> {
let session = Session::builder().trace(Status).build()?;
# drop(session);
# Ok(())
# }
```

See also: [Logging](https://manaforged.github.io/leyline-http/guide/logging.html).
