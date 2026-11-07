use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, Visitor};

use super::{H2PlatformOverride, H2Profile};
use crate::profile::{H2Settings, SettingId};

const PROFILE_FIELDS: &[&str] = &[
    "header_table_size",
    "enable_push",
    "max_concurrent_streams",
    "initial_stream_window_size",
    "initial_connection_window_size",
    "max_frame_size",
    "max_header_list_size",
    "unknown_setting8",
    "unknown_setting9",
    "pseudo_order",
    "settings_order",
    "default_priority",
    "fingerprint",
    "platforms",
];

const OVERRIDE_FIELDS: &[&str] = &[
    "header_table_size",
    "enable_push",
    "max_concurrent_streams",
    "initial_stream_window_size",
    "initial_connection_window_size",
    "max_frame_size",
    "max_header_list_size",
    "unknown_setting8",
    "unknown_setting9",
    "omit_settings",
    "pseudo_order",
    "settings_order",
    "fingerprint",
];

#[derive(Clone, Copy)]
enum ProfileField {
    InitialConnectionWindowSize,
    PseudoOrder,
    SettingsOrder,
    DefaultPriority,
    Fingerprint,
    Platforms,
}

const PROFILE_OWN: &[(&str, ProfileField)] = &[
    (
        "initial_connection_window_size",
        ProfileField::InitialConnectionWindowSize,
    ),
    ("pseudo_order", ProfileField::PseudoOrder),
    ("settings_order", ProfileField::SettingsOrder),
    ("default_priority", ProfileField::DefaultPriority),
    ("fingerprint", ProfileField::Fingerprint),
    ("platforms", ProfileField::Platforms),
];

#[derive(Clone, Copy)]
enum OverrideField {
    InitialConnectionWindowSize,
    OmitSettings,
    PseudoOrder,
    SettingsOrder,
    Fingerprint,
}

const OVERRIDE_OWN: &[(&str, OverrideField)] = &[
    (
        "initial_connection_window_size",
        OverrideField::InitialConnectionWindowSize,
    ),
    ("omit_settings", OverrideField::OmitSettings),
    ("pseudo_order", OverrideField::PseudoOrder),
    ("settings_order", OverrideField::SettingsOrder),
    ("fingerprint", OverrideField::Fingerprint),
];

enum Key<F> {
    Setting(SettingId),
    Own(&'static str, F),
}

struct KeySeed<F: 'static> {
    own: &'static [(&'static str, F)],
    fields: &'static [&'static str],
}

impl<F: 'static> Clone for KeySeed<F> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<F: 'static> Copy for KeySeed<F> {}

impl<'de, F: Copy + 'static> DeserializeSeed<'de> for KeySeed<F> {
    type Value = Key<F>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Key<F>, D::Error> {
        deserializer.deserialize_identifier(self)
    }
}

impl<'de, F: Copy + 'static> Visitor<'de> for KeySeed<F> {
    type Value = Key<F>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("field identifier")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Key<F>, E> {
        if let Some(id) = SettingId::from_field(v) {
            return Ok(Key::Setting(id));
        }
        self.own
            .iter()
            .find(|(name, _)| *name == v)
            .map(|&(name, field)| Key::Own(name, field))
            .ok_or_else(|| E::unknown_field(v, self.fields))
    }
}

fn read_setting<'de, A: MapAccess<'de>>(
    map: &mut A,
    settings: &mut H2Settings,
    id: SettingId,
) -> Result<(), A::Error> {
    if settings.get(id).is_some() {
        return Err(de::Error::duplicate_field(id.field()));
    }
    let value = if id == SettingId::EnablePush {
        map.next_value::<Option<bool>>()?.map(u32::from)
    } else {
        map.next_value::<Option<u32>>()?
    };
    settings
        .set(id, value)
        .map_err(<A::Error as de::Error>::custom)
}

fn read<'de, A: MapAccess<'de>, T: Deserialize<'de>>(
    map: &mut A,
    slot: &mut Option<T>,
    name: &'static str,
) -> Result<(), A::Error> {
    if slot.is_some() {
        return Err(de::Error::duplicate_field(name));
    }
    *slot = Some(map.next_value()?);
    Ok(())
}

struct StructVisitor<T>(PhantomData<T>);

impl<'de> Deserialize<'de> for H2Profile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_struct(
            "H2Profile",
            PROFILE_FIELDS,
            StructVisitor::<Self>(PhantomData),
        )
    }
}

impl<'de> Visitor<'de> for StructVisitor<H2Profile> {
    type Value = H2Profile;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("struct H2Profile")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<H2Profile, A::Error> {
        let seed = KeySeed {
            own: PROFILE_OWN,
            fields: PROFILE_FIELDS,
        };
        let mut settings = H2Settings::default();
        let mut window = None;
        let mut pseudo_order = None;
        let mut settings_order = None;
        let mut default_priority = None;
        let mut fingerprint = None;
        let mut platforms = None;
        while let Some(key) = map.next_key_seed(seed)? {
            match key {
                Key::Setting(id) => read_setting(&mut map, &mut settings, id)?,
                Key::Own(name, field) => match field {
                    ProfileField::InitialConnectionWindowSize => read(&mut map, &mut window, name)?,
                    ProfileField::PseudoOrder => read(&mut map, &mut pseudo_order, name)?,
                    ProfileField::SettingsOrder => read(&mut map, &mut settings_order, name)?,
                    ProfileField::DefaultPriority => read(&mut map, &mut default_priority, name)?,
                    ProfileField::Fingerprint => read(&mut map, &mut fingerprint, name)?,
                    ProfileField::Platforms => read(&mut map, &mut platforms, name)?,
                },
            }
        }
        let pseudo_order =
            pseudo_order.ok_or_else(|| <A::Error as de::Error>::missing_field("pseudo_order"))?;
        let settings_order = settings_order
            .ok_or_else(|| <A::Error as de::Error>::missing_field("settings_order"))?;
        Ok(H2Profile {
            settings,
            initial_connection_window_size: window.unwrap_or_default(),
            pseudo_order,
            settings_order,
            default_priority: default_priority.unwrap_or_default(),
            fingerprint: fingerprint.unwrap_or_default(),
            platforms: platforms.unwrap_or_default(),
        })
    }
}

impl<'de> Deserialize<'de> for H2PlatformOverride {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_struct(
            "H2PlatformOverride",
            OVERRIDE_FIELDS,
            StructVisitor::<Self>(PhantomData),
        )
    }
}

impl<'de> Visitor<'de> for StructVisitor<H2PlatformOverride> {
    type Value = H2PlatformOverride;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("struct H2PlatformOverride")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<H2PlatformOverride, A::Error> {
        let seed = KeySeed {
            own: OVERRIDE_OWN,
            fields: OVERRIDE_FIELDS,
        };
        let mut settings = H2Settings::default();
        let mut window = None;
        let mut omit_settings = None;
        let mut pseudo_order = None;
        let mut settings_order = None;
        let mut fingerprint = None;
        while let Some(key) = map.next_key_seed(seed)? {
            match key {
                Key::Setting(id) => read_setting(&mut map, &mut settings, id)?,
                Key::Own(name, field) => match field {
                    OverrideField::InitialConnectionWindowSize => {
                        read(&mut map, &mut window, name)?;
                    }
                    OverrideField::OmitSettings => read(&mut map, &mut omit_settings, name)?,
                    OverrideField::PseudoOrder => read(&mut map, &mut pseudo_order, name)?,
                    OverrideField::SettingsOrder => read(&mut map, &mut settings_order, name)?,
                    OverrideField::Fingerprint => read(&mut map, &mut fingerprint, name)?,
                },
            }
        }
        Ok(H2PlatformOverride {
            settings,
            initial_connection_window_size: window.unwrap_or_default(),
            omit_settings: omit_settings.unwrap_or_default(),
            pseudo_order: pseudo_order.unwrap_or_default(),
            settings_order: settings_order.unwrap_or_default(),
            fingerprint: fingerprint.unwrap_or_default(),
        })
    }
}
