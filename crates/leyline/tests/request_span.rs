use std::sync::{Arc, Mutex};

use leyline::Session;
use leyline::testing::{Response, Server};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};
use tracing_core::span::Current;

#[derive(Default)]
struct Spans {
    opened: Mutex<Vec<&'static Metadata<'static>>>,
    entered: Mutex<Vec<u64>>,
    hits: Mutex<Vec<(&'static str, bool)>>,
}

struct Recorder(Arc<Spans>);

struct PoolHit(Option<bool>);

impl Visit for PoolHit {
    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "pool.hit" {
            self.0 = Some(value);
        }
    }

    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}

impl Subscriber for Recorder {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        let mut opened = self.0.opened.lock().unwrap();
        opened.push(attrs.metadata());
        Id::from_u64(opened.len() as u64)
    }

    fn record(&self, id: &Id, values: &Record<'_>) {
        let mut hit = PoolHit(None);
        values.record(&mut hit);
        if let Some(hit) = hit.0 {
            let name = self.0.opened.lock().unwrap()[id.into_u64() as usize - 1].name();
            self.0.hits.lock().unwrap().push((name, hit));
        }
    }

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, id: &Id) {
        self.0.entered.lock().unwrap().push(id.into_u64());
    }

    fn exit(&self, _: &Id) {
        self.0.entered.lock().unwrap().pop();
    }

    fn current_span(&self) -> Current {
        match self.0.entered.lock().unwrap().last() {
            Some(&id) => Current::new(
                Id::from_u64(id),
                self.0.opened.lock().unwrap()[id as usize - 1],
            ),
            None => Current::none(),
        }
    }
}

#[tokio::test]
async fn the_request_span_reports_whether_the_connection_was_pooled() {
    let spans = Arc::new(Spans::default());
    let _default = tracing::subscriber::set_default(Recorder(Arc::clone(&spans)));
    let server = Server::http(|_| Response::new(200)).await.unwrap();
    let session = Session::new();

    session.get(server.url("/first")).send().await.unwrap();
    session.get(server.url("/second")).send().await.unwrap();

    assert_eq!(
        *spans.hits.lock().unwrap(),
        [("session.execute", false), ("session.execute", true)]
    );
}
