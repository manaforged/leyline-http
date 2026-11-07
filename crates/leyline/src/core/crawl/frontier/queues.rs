use super::{Origin, Queued};
use serde::{Deserialize, Deserializer, Serializer};
use std::collections::{HashMap, VecDeque};

pub(super) fn serialize<S: Serializer>(
    queues: &HashMap<Origin, VecDeque<Queued>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(queues.iter())
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<HashMap<Origin, VecDeque<Queued>>, D::Error> {
    let rows = Vec::<(Origin, VecDeque<Queued>)>::deserialize(deserializer)?;
    let mut queues = HashMap::with_capacity(rows.len());
    for (origin, queue) in rows {
        if queue.is_empty()
            || queue
                .iter()
                .any(|queued| Origin::of(&queued.url).as_ref() != Some(&origin))
        {
            return Err(serde::de::Error::custom(
                "checkpoint contains a mismatched origin queue",
            ));
        }
        if queues.insert(origin, queue).is_some() {
            return Err(serde::de::Error::custom(
                "checkpoint contains duplicate origin queues",
            ));
        }
    }
    Ok(queues)
}
