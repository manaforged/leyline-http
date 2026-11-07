use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, Stream, StreamExt};
use tokio::time::{Instant, Sleep};

use super::frontier::{Queued, page_key};
use super::{Crawl, Page, SkipReason};
use crate::core::config::Origin;
use crate::deadline::Deadline;
use crate::error::Result;

const ALARM_FLOOR: Duration = Duration::from_millis(10);

#[derive(Default)]
pub(super) struct Clock {
    pub(super) stopped: Option<crate::core::shutdown::Stopped>,
    alarm: Option<Pin<Box<Sleep>>>,
    alarm_at: Option<Instant>,
    room: bool,
    pub(super) delayed: Vec<(Instant, Queued)>,
}

enum Wait {
    Ready,
    Until(Instant),
    Room,
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

    fn due_sitemap_hosts(&self, now: Instant) -> Vec<Origin> {
        self.sitemaps
            .queue
            .iter()
            .filter(|file| file.not_before.is_none_or(|at| at <= now))
            .filter_map(|file| Origin::of(&file.url))
            .collect()
    }

    pub(super) fn plan_alarm(&mut self) {
        let now = Instant::now();
        let sitemap_hosts = self.due_sitemap_hosts(now);
        let mut room = false;
        let blocked = self
            .frontier
            .origins()
            .chain(sitemap_hosts.iter())
            .map(|origin| match self.host_wait(origin) {
                Wait::Ready => None,
                Wait::Until(at) => Some(at),
                Wait::Room => {
                    room = true;
                    None
                }
            })
            .chain(self.clock.delayed.iter().map(|(at, _)| Some(*at)))
            .chain(
                self.sitemaps
                    .queue
                    .iter()
                    .map(|file| file.not_before.filter(|at| *at > now)),
            )
            .flatten()
            .min();
        let spent = self.budget_spent();
        self.clock.room = room && !spent;
        if let Some(at) = blocked.filter(|_| !spent) {
            self.set_alarm(at);
        }
    }

    fn host_wait(&self, origin: &Origin) -> Wait {
        let own = self.session.host_limits();
        match self.pace.ready_at(origin).max(own.ready_at(origin)) {
            Some(at) => Wait::Until(at),
            None if !(self.pace.has_room(origin) && own.has_room(origin)) => Wait::Room,
            None => Wait::Ready,
        }
    }

    fn watch_room(&self, cx: &Context<'_>) -> bool {
        self.pace.watch_room(cx.waker());
        self.session.host_limits().watch_room(cx.waker());
        let sitemap_hosts = self.due_sitemap_hosts(Instant::now());
        !self
            .frontier
            .origins()
            .chain(sitemap_hosts.iter())
            .any(|origin| matches!(self.host_wait(origin), Wait::Room))
    }

    fn waiting_work(&self) -> bool {
        !self.budget_spent()
            && (!self.frontier.is_empty()
                || !self.clock.delayed.is_empty()
                || !self.sitemaps.queue.is_empty())
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
        self.sitemaps.queue.clear();
        self.clock.delayed.clear();
        self.clock.alarm = None;
        self.clock.alarm_at = None;
        self.clock.room = false;
        self.tasks = FuturesUnordered::new();
    }
}

impl Stream for Crawl {
    type Item = Result<Page>;

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
                if this.clock.room && this.watch_room(cx) {
                    continue;
                }
                return Poll::Pending;
            }
            this.clock.alarm = None;
            this.clock.alarm_at = None;
        }
    }
}
