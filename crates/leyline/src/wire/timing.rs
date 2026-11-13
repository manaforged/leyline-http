#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResponseTiming {
    pub(crate) reused: bool,
    pub(crate) connect_ms: Option<u32>,
    pub(crate) send_ms: u32,
    pub(crate) total_ms: u32,
    pub(crate) from_cache: bool,
}

impl ResponseTiming {
    pub(crate) fn accumulator() -> Self {
        Self {
            reused: true,
            connect_ms: None,
            send_ms: 0,
            total_ms: 0,
            from_cache: false,
        }
    }

    pub(crate) fn leg(started: std::time::Instant, connect_ms: Option<u32>) -> Self {
        let total_ms = Self::millis(started);
        let send_ms = total_ms.saturating_sub(connect_ms.unwrap_or(0));
        Self::measured(connect_ms, send_ms, total_ms)
    }

    pub(crate) fn measured(connect_ms: Option<u32>, send_ms: u32, total_ms: u32) -> Self {
        Self {
            reused: connect_ms.is_none(),
            connect_ms,
            send_ms,
            total_ms,
            from_cache: false,
        }
    }

    pub(crate) fn cached(self) -> Self {
        Self {
            from_cache: true,
            ..self
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
        self.from_cache = leg.from_cache;
    }
}
