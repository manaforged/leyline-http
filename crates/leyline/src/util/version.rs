use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serializer};

pub(crate) trait Versioned: Copy + Default {
    fn number(self) -> u32;
    fn from_number(number: u32) -> Option<Self>;
}

pub(crate) fn serialize<V: Versioned, S: Serializer>(
    version: &V,
    ser: S,
) -> std::result::Result<S::Ok, S::Error> {
    ser.serialize_u32(version.number())
}

pub(crate) fn deserialize<'de, V: Versioned, D: Deserializer<'de>>(
    de: D,
) -> std::result::Result<V, D::Error> {
    let number = u32::deserialize(de)?;
    V::from_number(number).ok_or_else(|| {
        D::Error::custom(format!(
            "unsupported file version {number}; this build reads version {}",
            V::default().number()
        ))
    })
}
