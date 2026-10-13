use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, Stream, StreamExt};
use tokio::time::{Instant, Sleep};
use tokio_util::sync::WaitForCancellationFutureOwned;

use super::frontier::{Queued, page_key};
use super::{Crawl, CrawlPage, SkipReason};
use crate::core::config::Origin;
use crate::core::deadline::Deadline;
use crate::core::error::Result;

const ROOM_RECHECK: Duration = Duration::from_millis(250);
const ALARM_FLOOR: Duration = Duration::from_millis(10);

#[derive(Default)]
pub(super) struct Clock {
    pub(super) stopped: Option<Pin<Box<WaitForCancellationFutureOwned>>>,
    alarm: Option<Pin<Box<Sleep>>>,
    alarm_at: Option<Instant>,
    delayed: Vec<(Instant, Queued)>,
}

impl Crawl {
    pub(super) fn later(&mut self, mut queued: Queued, wait: Duration) {
        let Some(at) = Instant::now().checked_add(wait) else {
            self.stats.skip(SkipReason::Status);
            return self.settle(page_key(&queued.url));
        };
        queued.attempts += 1;
        self.clock.delayed.push((at, queued));
        self.set_alarm(at);
    }

    pub(super) fn promote_due(&mut self) {
        let now = Instant::now();
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.clock.delayed)
            .into_iter()
            .partition(|(at, _)| *at <= now);
        self.clock.delayed = waiting;
        for (_, queued) in due {
            self.frontier.push_back(queued);
        }
    }

    pub(super) fn set_alarm(&mut self, at: Instant) {
        let at = at.max(Instant::now() + ALARM_FLOOR);
        if self.clock.alarm_at.is_none_or(|current| at < current) {
            self.clock.alarm = Some(Deadline::alarm(at));
            self.clock.alarm_at = Some(at);
        }
    }

    pub(super) fn plan_alarm(&mut self) {
        let now = Instant::now();
        let sitemap_hosts: Vec<Origin> = self
            .sitemap_queue
            .iter()
            .filter(|file| file.not_before.is_none_or(|at| at <= now))
            .filter_map(|file| Origin::of(&file.url))
            .collect();
        let blocked = self
            .frontier
            .origins()
            .chain(sitemap_hosts.iter())
            .map(|origin| self.host_wait(origin))
            .chain(self.clock.delayed.iter().map(|(at, _)| Some(*at)))
            .chain(
                self.sitemap_queue
                    .iter()
                    .map(|file| file.not_before.filter(|at| *at > now)),
            )
            .flatten()
            .min();
        if let Some(at) = blocked.filter(|_| !self.budget_spent()) {
            self.set_alarm(at);
        }
    }

    fn host_wait(&self, origin: &Origin) -> Option<Instant> {
        let own = self.session.host_limits();
        match self.pace.ready_at(origin).max(own.ready_at(origin)) {
            Some(at) => Some(at),
            None if !(self.pace.ready(origin) && own.ready(origin)) => {
                Some(Instant::now() + ROOM_RECHECK)
            }
            None => None,
        }
    }

    fn waiting_work(&self) -> bool {
        !self.budget_spent()
            && (!self.frontier.is_empty()
                || !self.clock.delayed.is_empty()
                || !self.sitemap_queue.is_empty())
    }

    fn stopped(&mut self, cx: &mut Context<'_>) -> bool {
        let signalled = self
            .clock
            .stopped
            .as_mut()
            .is_some_and(|stopped| stopped.as_mut().poll(cx).is_ready());
        signalled || self.session.is_shut_down()
    }

    fn halt(&mut self) {
        self.frontier.clear();
        self.sitemap_queue.clear();
        self.clock.delayed.clear();
        self.clock.alarm = None;
        self.clock.alarm_at = None;
        self.tasks = FuturesUnordered::new();
    }
}

impl Stream for Crawl {
    type Item = Result<CrawlPage>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.start();
        loop {
            if this.stopped(cx) {
                this.halt();
                return Poll::Ready(this.ready.pop_front());
            }
            this.schedule();
            if let Some(item) = this.ready.pop_front() {
                return Poll::Ready(Some(item));
            }
            if this.limit_met() && this.pages_in_flight == 0 {
                this.halt();
                return Poll::Ready(None);
            }
            match this.tasks.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => {
                    this.handle(event);
                    continue;
                }
                Poll::Ready(None) if !this.waiting_work() => {
                    if this.release_held() {
                        continue;
                    }
                    return Poll::Ready(None);
                }
                Poll::Ready(None) | Poll::Pending => {}
            }
            let fired = this
                .clock
                .alarm
                .as_mut()
                .is_some_and(|alarm| alarm.as_mut().poll(cx).is_ready());
            if !fired {
                return Poll::Pending;
            }
            this.clock.alarm = None;
            this.clock.alarm_at = None;
        }
    }
}
