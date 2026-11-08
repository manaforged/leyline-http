#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResponseTiming {
    pub(crate) reused: bool,
    pub(crate) connect_ms: Option<u32>,
    pub(crate) send_ms: u32,
    pub(crate) total_ms: u32,
}

impl ResponseTiming {
    pub(crate) fn accumulator() -> Self {
        Self {
            reused: true,
            connect_ms: None,
            send_ms: 0,
            total_ms: 0,
        }
    }

    pub(crate) fn leg(started: std::time::Instant, connect_ms: Option<u32>) -> Self {
        let total_ms = Self::millis(started);
        Self {
            reused: connect_ms.is_none(),
            connect_ms,
            send_ms: total_ms.saturating_sub(connect_ms.unwrap_or(0)),
            total_ms,
        }
    }

    pub(crate) fn millis(start: std::time::Instant) -> u32 {
        Self::span(start.elapsed())
    }

    pub(crate) fn span(elapsed: std::time::Duration) -> u32 {
        u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX)
    }

    pub(crate) fn add_leg(&mut self, leg: &ResponseTiming) {
        self.total_ms = self.total_ms.saturating_add(leg.total_ms);
        self.send_ms = self.send_ms.saturating_add(leg.send_ms);
        if let Some(c) = leg.connect_ms {
            self.connect_ms = Some(self.connect_ms.unwrap_or(0).saturating_add(c));
        }
        self.reused &= leg.reused;
    }
}
