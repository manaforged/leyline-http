use std::collections::BTreeMap;

use serde::ser::{Serialize, SerializeStruct, Serializer};

use super::Device;

const FIELDS: usize = 16;

impl Serialize for Device {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let device = self.sanitized();
        let Device {
            identity,
            platform,
            brand,
            profile_toml,
            profile_id,
            user_agent,
            proxy,
            proxy_password_env,
            languages,
            env_proxy,
            jar_path,
            jar,
            state,
            app: values,
            strict,
            page,
        } = &device;
        let mut out = serializer.serialize_struct("Device", FIELDS)?;
        out.serialize_field("identity", identity)?;
        out.serialize_field("platform", platform)?;
        out.serialize_field("brand", brand)?;
        optional(&mut out, "profile_toml", profile_toml.as_ref())?;
        out.serialize_field("profile_id", profile_id)?;
        optional(&mut out, "user_agent", user_agent.as_ref())?;
        network(
            &mut out,
            proxy,
            proxy_password_env.as_ref(),
            languages,
            *env_proxy,
        )?;
        storage(&mut out, jar_path.as_ref(), jar.as_ref(), state)?;
        app(&mut out, values)?;
        out.serialize_field("strict", strict)?;
        optional(&mut out, "page", page.as_ref())?;
        out.end()
    }
}

fn network<S: SerializeStruct>(
    out: &mut S,
    proxy: &Option<crate::core::ProxyUrl>,
    password_env: Option<&String>,
    languages: &Option<Vec<String>>,
    env_proxy: bool,
) -> std::result::Result<(), S::Error> {
    out.serialize_field("proxy", proxy)?;
    optional(out, "proxy_password_env", password_env)?;
    out.serialize_field("languages", languages)?;
    out.serialize_field("env_proxy", &env_proxy)
}

fn storage<S: SerializeStruct>(
    out: &mut S,
    jar_path: Option<&std::path::PathBuf>,
    jar: Option<&crate::cookie::Jar>,
    state: &crate::core::SessionState,
) -> std::result::Result<(), S::Error> {
    optional(out, "jar_path", jar_path)?;
    optional(out, "jar", jar)?;
    out.serialize_field("state", state)
}

fn optional<S: SerializeStruct, T: Serialize>(
    out: &mut S,
    name: &'static str,
    value: Option<&T>,
) -> std::result::Result<(), S::Error> {
    match value {
        Some(value) => out.serialize_field(name, value),
        None => out.skip_field(name),
    }
}

fn app<S: SerializeStruct>(
    out: &mut S,
    app: &BTreeMap<String, serde_json::Value>,
) -> std::result::Result<(), S::Error> {
    if app.is_empty() {
        out.skip_field("app")
    } else {
        out.serialize_field("app", app)
    }
}
