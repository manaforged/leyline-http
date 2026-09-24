use crate::h2::H2Config;
use crate::h2::config::{PseudoOrder, SettingId};
use crate::profile::H2Profile;

const DEFAULT_CONNECTION_WINDOW: u32 = 65_535;

const NO_PRIORITY_FRAMES: &str = "0";

fn clear_settings(h2: &mut H2Profile) {
    h2.header_table_size = None;
    h2.enable_push = None;
    h2.max_concurrent_streams = None;
    h2.initial_stream_window_size = None;
    h2.max_frame_size = None;
    h2.max_header_list_size = None;
    h2.unknown_setting8 = None;
    h2.unknown_setting9 = None;
}

fn set_setting(h2: &mut H2Profile, id: SettingId, value: u32) -> Result<(), String> {
    match id {
        SettingId::HeaderTableSize => h2.header_table_size = Some(value),
        SettingId::EnablePush => {
            h2.enable_push = Some(match value {
                0 => false,
                1 => true,
                other => return Err(format!("ENABLE_PUSH value {other} is not 0 or 1")),
            });
        }
        SettingId::MaxConcurrentStreams => h2.max_concurrent_streams = Some(value),
        SettingId::InitialWindowSize => h2.initial_stream_window_size = Some(value),
        SettingId::MaxFrameSize => h2.max_frame_size = Some(value),
        SettingId::MaxHeaderListSize => h2.max_header_list_size = Some(value),
        SettingId::Unknown8 => h2.unknown_setting8 = Some(value),
        SettingId::Unknown9 => h2.unknown_setting9 = Some(value),
    }
    Ok(())
}

fn number<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("{what} {text:?} is not a number"))
}

pub(super) fn apply(h2: &mut H2Profile, raw: &str) -> Result<(), String> {
    let parts: Vec<&str> = raw.trim().split('|').collect();
    let [settings, window, priority, pseudo] = parts.as_slice() else {
        return Err(format!(
            "expected 4 '|'-separated fields (SETTINGS|WINDOW_UPDATE|PRIORITY|pseudo-headers), \
             got {}",
            parts.len()
        ));
    };
    clear_settings(h2);
    h2.settings_order.clear();
    for entry in settings.split([';', ',']).filter(|e| !e.is_empty()) {
        let (code, value) = entry
            .split_once(':')
            .ok_or_else(|| format!("SETTINGS entry {entry:?} is not id:value"))?;
        let code: u16 = number(code, "SETTINGS id")?;
        let id = SettingId::from_code(code)
            .ok_or_else(|| format!("SETTINGS id {code} is not one leyline can send"))?;
        if h2.settings_order.iter().any(|key| key == id.key()) {
            return Err(format!("SETTINGS id {code} appears twice"));
        }
        set_setting(h2, id, number(value, "SETTINGS value")?)?;
        h2.settings_order.push(id.key().to_owned());
    }
    let increment: u32 = number(window, "WINDOW_UPDATE")?;
    h2.initial_connection_window_size = Some(
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
    h2.pseudo_order = pseudo
        .split(',')
        .map(|label| {
            PseudoOrder::key_for_label(label.trim())
                .map(str::to_owned)
                .ok_or_else(|| format!("pseudo-header {label:?} is not one of m, a, s, p"))
        })
        .collect::<Result<_, _>>()?;
    h2.fingerprint = None;
    h2.platforms.clear();
    H2Config::from_profile(h2).map_err(|err| err.to_string())?;
    Ok(())
}
