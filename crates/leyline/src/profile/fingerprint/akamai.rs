use crate::h2::H2Config;
use crate::h2::config::{PseudoOrder, SettingId};
use crate::profile::H2Profile;

const DEFAULT_CONNECTION_WINDOW: u32 = 65_535;

const NO_PRIORITY_FRAMES: &str = "0";

impl H2Profile {
    fn clear_settings(&mut self) {
        self.header_table_size = None;
        self.enable_push = None;
        self.max_concurrent_streams = None;
        self.initial_stream_window_size = None;
        self.max_frame_size = None;
        self.max_header_list_size = None;
        self.unknown_setting8 = None;
        self.unknown_setting9 = None;
    }

    fn set_setting(&mut self, id: SettingId, value: u32) -> Result<(), String> {
        match id {
            SettingId::HeaderTableSize => self.header_table_size = Some(value),
            SettingId::EnablePush => {
                self.enable_push = Some(match value {
                    0 => false,
                    1 => true,
                    other => return Err(format!("ENABLE_PUSH value {other} is not 0 or 1")),
                });
            }
            SettingId::MaxConcurrentStreams => self.max_concurrent_streams = Some(value),
            SettingId::InitialWindowSize => self.initial_stream_window_size = Some(value),
            SettingId::MaxFrameSize => self.max_frame_size = Some(value),
            SettingId::MaxHeaderListSize => self.max_header_list_size = Some(value),
            SettingId::Unknown8 => self.unknown_setting8 = Some(value),
            SettingId::Unknown9 => self.unknown_setting9 = Some(value),
        }
        Ok(())
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
        self.clear_settings();
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
            self.set_setting(id, number(value, "SETTINGS value")?)?;
            self.settings_order.push(id.key().to_owned());
        }
        let increment: u32 = number(window, "WINDOW_UPDATE")?;
        self.initial_connection_window_size = Some(
            increment
                .checked_add(DEFAULT_CONNECTION_WINDOW)
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
