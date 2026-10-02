use super::scan::Tag;

const VOID: [&str; 13] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Legend {
    Before,
    Inside,
    After,
}

struct Level {
    disabled: bool,
    legend: Legend,
    depth: usize,
}

#[derive(Default)]
pub(super) struct Fieldsets {
    stack: Vec<Level>,
}

impl Fieldsets {
    pub(super) fn tag(&mut self, tag: &Tag<'_>) {
        match (tag.closing, tag.name.as_str()) {
            (false, "fieldset") => self.stack.push(Level {
                disabled: tag.has("disabled"),
                legend: Legend::Before,
                depth: 0,
            }),
            (true, "fieldset") => {
                self.stack.pop();
            }
            (false, name) => self.open(name),
            (true, name) => self.close(name),
        }
    }

    fn open(&mut self, name: &str) {
        let Some(level) = self.stack.last_mut() else {
            return;
        };
        if name == "legend" && level.depth == 0 && level.legend == Legend::Before {
            level.legend = Legend::Inside;
        }
        if !VOID.contains(&name) {
            level.depth += 1;
        }
    }

    fn close(&mut self, name: &str) {
        let Some(level) = self.stack.last_mut() else {
            return;
        };
        level.depth = level.depth.saturating_sub(1);
        if name == "legend" && level.depth == 0 && level.legend == Legend::Inside {
            level.legend = Legend::After;
        }
    }

    pub(super) fn disabled(&self) -> bool {
        self.stack
            .iter()
            .any(|level| level.disabled && level.legend != Legend::Inside)
    }
}
