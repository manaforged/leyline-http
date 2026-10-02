use super::scan::Tag;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Legend {
    Before,
    Inside,
    After,
}

struct Level {
    disabled: bool,
    legend: Legend,
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
            }),
            (true, "fieldset") => {
                self.stack.pop();
            }
            (false, "legend") => self.step(Legend::Before, Legend::Inside),
            (true, "legend") => self.step(Legend::Inside, Legend::After),
            _ => {}
        }
    }

    pub(super) fn disabled(&self) -> bool {
        self.stack
            .iter()
            .any(|level| level.disabled && level.legend != Legend::Inside)
    }

    fn step(&mut self, from: Legend, to: Legend) {
        if let Some(level) = self.stack.last_mut().filter(|level| level.legend == from) {
            level.legend = to;
        }
    }
}
