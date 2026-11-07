use crate::profile::{
    DEFAULT_WINDOW_SIZE, H2Profile, NO_PRIORITY_FRAMES, PseudoOrder, SettingId, parse_pseudo_order,
};

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
        self.apply_akamai_settings(settings)?;
        self.initial_connection_window_size = Some(connection_window(window)?);
        if *priority != NO_PRIORITY_FRAMES {
            return Err(format!(
                "PRIORITY field {priority:?} lists PRIORITY frames; leyline sends none, so the field \
                 must be {NO_PRIORITY_FRAMES}"
            ));
        }
        self.pseudo_order = pseudo_order(pseudo)?;
        self.fingerprint = None;
        self.platforms.clear();
        parse_pseudo_order(&self.pseudo_order, "H2").map_err(|err| err.to_string())?;
        Ok(())
    }

    fn apply_akamai_settings(&mut self, settings: &str) -> Result<(), String> {
        for id in SettingId::all() {
            self.settings.set(id, None)?;
        }
        self.settings_order.clear();
        for entry in settings.split([';', ',']).filter(|e| !e.is_empty()) {
            self.apply_akamai_setting(entry)?;
        }
        Ok(())
    }

    fn apply_akamai_setting(&mut self, entry: &str) -> Result<(), String> {
        let (code, value) = entry
            .split_once(':')
            .ok_or_else(|| format!("SETTINGS entry {entry:?} is not id:value"))?;
        let code: u16 = number(code, "SETTINGS id")?;
        let id = SettingId::from_code(code)
            .ok_or_else(|| format!("SETTINGS id {code} is not one leyline can send"))?;
        if self.settings_order.iter().any(|key| key == id.key()) {
            return Err(format!("SETTINGS id {code} appears twice"));
        }
        self.settings
            .set(id, Some(number(value, "SETTINGS value")?))?;
        self.settings_order.push(id.key().to_owned());
        Ok(())
    }
}

fn connection_window(window: &str) -> Result<u32, String> {
    let increment: u32 = number(window, "WINDOW_UPDATE")?;
    increment
        .checked_add(DEFAULT_WINDOW_SIZE)
        .ok_or_else(|| format!("WINDOW_UPDATE {increment} overflows the window"))
}

fn pseudo_order(pseudo: &str) -> Result<Vec<String>, String> {
    pseudo
        .split(',')
        .map(|label| {
            PseudoOrder::key_for_label(label.trim())
                .map(str::to_owned)
                .ok_or_else(|| format!("pseudo-header {label:?} is not one of m, a, s, p"))
        })
        .collect()
}

/// Formats the Akamai HTTP/2 fingerprint of the SETTINGS, connection window and pseudo-header order a connection sends.
pub fn akamai_fingerprint(
    settings: &[(SettingId, u32)],
    connection_window: u32,
    pseudo_order: &[PseudoOrder],
) -> String {
    let settings = settings
        .iter()
        .map(|(id, value)| format!("{}:{value}", id.code()))
        .collect::<Vec<_>>()
        .join(";");
    let window_update = connection_window.saturating_sub(DEFAULT_WINDOW_SIZE);
    let pseudo = pseudo_order
        .iter()
        .map(|p| p.label())
        .collect::<Vec<_>>()
        .join(",");
    format!("{settings}|{window_update}|{NO_PRIORITY_FRAMES}|{pseudo}")
}
