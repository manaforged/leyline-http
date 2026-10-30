use crate::h2::H2Config;
use crate::h2::config::{DEFAULT_WINDOW_SIZE, PseudoOrder, SettingId};
use crate::profile::H2Profile;

const NO_PRIORITY_FRAMES: &str = "0";

impl H2Config {
    pub fn akamai_fingerprint(&self) -> String {
        let settings_str: String = self
            .settings_order
            .iter()
            .filter_map(|&id| self.setting(id).map(|val| format!("{}:{}", id as u16, val)))
            .collect::<Vec<_>>()
            .join(";");

        let window_update = self
            .initial_connection_window_size
            .saturating_sub(DEFAULT_WINDOW_SIZE);

        let pseudo_str: String = self
            .pseudo_order
            .iter()
            .map(|p| p.label())
            .collect::<Vec<_>>()
            .join(",");

        format!("{settings_str}|{window_update}|{NO_PRIORITY_FRAMES}|{pseudo_str}")
    }
}

fn number<T>(text: &str, what: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    text.trim()
        .parse()
        .map_err(|e| format!("{what} {text:?} is not a number: {e}"))
}

impl H2Profile {
    pub(super) fn apply_akamai(&mut self, raw: &str) -> Result<(), String> {
        let parts: Vec<&str> = raw.trim().split('|').collect();
        let [settings, window, priority, pseudo] = parts.as_slice() else {
            return Err(format!(
                "expected 4 '|'-separated fields (SETTINGS|WINDOW_UPDATE|PRIORITY|pseudo-headers), \
                 got {}",
                parts.len()
            ));
        };
        for id in SettingId::all() {
            self.set_setting(id, None)?;
        }
        self.settings_order.clear();
        for entry in settings.split([';', ',']).filter(|e| !e.is_empty()) {
            let (code, value) = entry
                .split_once(':')
                .ok_or_else(|| format!("SETTINGS entry {entry:?} is not id:value"))?;
            let code: u16 = number(code, "SETTINGS id")?;
            let id = SettingId::from_code(code)
                .ok_or_else(|| format!("SETTINGS id {code} is not one leyline can send"))?;
            if self.settings_order.iter().any(|key| key == id.key()) {
                return Err(format!("SETTINGS id {code} appears twice"));
            }
            self.set_setting(id, Some(number(value, "SETTINGS value")?))?;
            self.settings_order.push(id.key().to_owned());
        }
        let increment: u32 = number(window, "WINDOW_UPDATE")?;
        self.initial_connection_window_size = Some(
            increment
                .checked_add(DEFAULT_WINDOW_SIZE)
                .ok_or_else(|| format!("WINDOW_UPDATE {increment} overflows the window"))?,
        );
        if *priority != NO_PRIORITY_FRAMES {
            return Err(format!(
                "PRIORITY field {priority:?} lists PRIORITY frames; leyline sends none, so the field \
                 must be {NO_PRIORITY_FRAMES}"
            ));
        }
        self.pseudo_order = pseudo
            .split(',')
            .map(|label| {
                PseudoOrder::key_for_label(label.trim())
                    .map(str::to_owned)
                    .ok_or_else(|| format!("pseudo-header {label:?} is not one of m, a, s, p"))
            })
            .collect::<Result<_, _>>()?;
        self.fingerprint = None;
        self.platforms.clear();
        H2Config::from_profile(self).map_err(|err| err.to_string())?;
        Ok(())
    }
}
