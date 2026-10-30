use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FileVersion {
    #[default]
    V1,
}

impl FileVersion {
    const fn number(self) -> u32 {
        match self {
            FileVersion::V1 => 1,
        }
    }

    fn from_number(number: u32) -> Option<Self> {
        match number {
            1 => Some(FileVersion::V1),
            _ => None,
        }
    }
}

impl Serialize for FileVersion {
    fn serialize<S: Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        ser.serialize_u32(self.number())
    }
}

impl<'de> Deserialize<'de> for FileVersion {
    fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        let number = u32::deserialize(de)?;
        FileVersion::from_number(number).ok_or_else(|| {
            D::Error::custom(format!(
                "unsupported file version {number}; this build reads version {}",
                FileVersion::default().number()
            ))
        })
    }
}
