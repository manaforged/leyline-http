use bytes::{Buf, Bytes};
use std::time::Duration;

use super::Event;
use crate::error::{Error, Kind, Result};

pub(super) struct Parser {
    line: Vec<u8>,
    data: String,
    event: String,
    id: String,
    pub(super) last_id: String,
    pub(super) wait: Duration,
    pub(super) max: usize,
    first: bool,
    cr: bool,
}

impl Parser {
    pub(super) fn new(id: String, max: usize, wait: Duration) -> Self {
        Self {
            line: Vec::new(),
            data: String::new(),
            event: String::new(),
            id: id.clone(),
            last_id: id,
            wait,
            max,
            first: true,
            cr: false,
        }
    }

    pub(super) fn reset(&mut self) {
        self.line.clear();
        self.data.clear();
        self.event.clear();
        self.id.clone_from(&self.last_id);
        self.first = true;
        self.cr = false;
    }

    fn buffered(&self) -> usize {
        self.line
            .len()
            .saturating_add(self.data.len())
            .saturating_add(self.event.len())
            .saturating_add(self.id.len())
    }

    pub(super) fn read(&mut self, bytes: &mut Bytes) -> Result<Option<Event>> {
        if self.buffered() > self.max {
            return Err(Error::new(Kind::BodyLimit)
                .with_message("server-sent event exceeds max_event_size"));
        }
        while bytes.has_remaining() {
            let byte = bytes.get_u8();
            let after_cr = std::mem::replace(&mut self.cr, byte == b'\r');
            if byte == b'\n' && after_cr {
                continue;
            }
            if matches!(byte, b'\r' | b'\n') {
                if let Some(event) = self.line()? {
                    return Ok(Some(event));
                }
            } else {
                if self.buffered() >= self.max {
                    return Err(Error::new(Kind::BodyLimit)
                        .with_message("server-sent event exceeds max_event_size"));
                }
                self.line.push(byte);
            }
        }
        Ok(None)
    }

    fn line(&mut self) -> Result<Option<Event>> {
        let bytes = std::mem::take(&mut self.line);
        let bytes = if std::mem::replace(&mut self.first, false) {
            bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)
        } else {
            &bytes
        };
        let line = String::from_utf8_lossy(bytes);
        if line.is_empty() {
            self.last_id.clone_from(&self.id);
            let event = std::mem::take(&mut self.event);
            if self.data.is_empty() {
                return Ok(None);
            }
            self.data.pop();
            return Ok(Some(Event {
                id: self.last_id.clone(),
                event: if event.is_empty() {
                    "message".into()
                } else {
                    event
                },
                data: std::mem::take(&mut self.data),
            }));
        }
        let (field, value) = line.split_once(':').unwrap_or((&line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        self.field(field, value);
        if self.buffered() > self.max {
            return Err(Error::new(Kind::BodyLimit)
                .with_message("server-sent event exceeds max_event_size"));
        }
        Ok(None)
    }

    fn field(&mut self, field: &str, value: &str) {
        match field {
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            "event" => self.event = value.into(),
            "id" if !value.contains('\0') => self.id = value.into(),
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                if let Ok(ms) = value.parse::<u64>() {
                    self.wait = Duration::from_millis(ms);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
