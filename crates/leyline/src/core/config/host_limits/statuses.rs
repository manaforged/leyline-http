use http::StatusCode;
use serde::{Deserialize, Deserializer, Serializer};

pub(super) fn serialize<S: Serializer>(
    statuses: &[StatusCode],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(statuses.iter().map(StatusCode::as_u16))
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<StatusCode>, D::Error> {
    Vec::<u16>::deserialize(deserializer)?
        .into_iter()
        .map(|code| StatusCode::from_u16(code).map_err(serde::de::Error::custom))
        .collect()
}
