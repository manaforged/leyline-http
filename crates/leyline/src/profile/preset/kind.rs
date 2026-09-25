use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Preset {
    Native,
    Navigate,
    Script,
    Xhr,
    Form,
    CrossOrigin,
    SameSite,
    FormNavigate,
}
