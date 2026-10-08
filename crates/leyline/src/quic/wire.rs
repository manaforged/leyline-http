use leyline_quiche as quiche;
use quiche::TransportParamEntry;
use rand::seq::SliceRandom;
use rand::{Rng, RngCore};

use crate::profile::{
    H3ConnectionIdLength, H3CryptoReorder, H3CryptoSplit, H3Grease, H3Order, H3Profile, H3Setting,
    H3TransportParam, H3VersionGrease, H3VersionInformation,
};

const TRANSPORT_GREASE_BASE: u64 = 27;
const H3_GREASE_BASE: u64 = 33;
const MAX_DATAGRAM_FRAME_SIZE: u64 = 0x20;

pub(crate) fn connection_id_len(len: &H3ConnectionIdLength) -> usize {
    match len {
        H3ConnectionIdLength::Fixed(n) => *n,
        H3ConnectionIdLength::Weighted { weights } => {
            let total: u64 = weights.iter().map(|(_, w)| u64::from(*w)).sum();
            let mut pick = rand::rng().random_range(0..total.max(1));
            for (len, weight) in weights {
                if pick < u64::from(*weight) {
                    return *len;
                }
                pick -= u64::from(*weight);
            }
            weights.last().map_or(0, |(len, _)| *len)
        }
    }
}

pub(crate) fn random_bytes(len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    rand::rng().fill_bytes(&mut out);
    out
}

impl H3Profile {
    pub(crate) fn sends_datagrams(&self) -> bool {
        self.transport_parameters
            .iter()
            .flatten()
            .any(|p| p.id == Some(MAX_DATAGRAM_FRAME_SIZE))
    }

    pub(crate) fn transport_plan(&self) -> Result<Option<Vec<TransportParamEntry>>, String> {
        let Some(params) = &self.transport_parameters else {
            return Ok(None);
        };
        let entries = params
            .iter()
            .map(transport_entry)
            .collect::<Result<Vec<_>, String>>()?;
        let pinned: Vec<bool> = params.iter().map(|p| p.pinned).collect();
        Ok(Some(reorder(entries, &pinned, self.transport_order)))
    }
}

fn transport_entry(param: &H3TransportParam) -> Result<TransportParamEntry, String> {
    if let Some(grease) = &param.grease {
        let id = grease_id(grease.id_bits, TRANSPORT_GREASE_BASE);
        let len = rand::rng().random_range(0..=grease.max_len);
        return Ok(TransportParamEntry::Raw(id, random_bytes(len)));
    }
    let id = param.id.ok_or("transport parameter without id")?;
    if let Some(value) = param.varint {
        return Ok(TransportParamEntry::Raw(id, varint(value)));
    }
    if let Some(hex) = &param.hex {
        let value = hex::decode(hex).map_err(|e| format!("transport parameter {id}: {e}"))?;
        return Ok(TransportParamEntry::Raw(id, value));
    }
    if let Some(versions) = &param.versions {
        return Ok(TransportParamEntry::Raw(id, version_information(versions)));
    }
    Ok(TransportParamEntry::Local(id))
}

fn reorder<T>(entries: Vec<T>, pinned: &[bool], order: H3Order) -> Vec<T> {
    let mut slots: Vec<Option<T>> = entries.into_iter().map(Some).collect();
    let mut movable: Vec<T> = slots
        .iter_mut()
        .zip(pinned)
        .filter(|(_, pinned)| !**pinned)
        .filter_map(|(slot, _)| slot.take())
        .collect();
    let mut rng = rand::rng();
    match order {
        H3Order::Fixed => {}
        H3Order::Shuffle => movable.shuffle(&mut rng),
        H3Order::Rotate if !movable.is_empty() => {
            let by = rng.random_range(0..movable.len());
            movable.rotate_left(by);
        }
        H3Order::Rotate => {}
    }
    let mut movable = movable.into_iter();
    slots
        .into_iter()
        .filter_map(|slot| slot.or_else(|| movable.next()))
        .collect()
}

impl H3Profile {
    pub(crate) fn crypto_split(&self) -> quiche::InitialCryptoSplit {
        match (self.initial_crypto_split, self.initial_crypto_reorder) {
            (H3CryptoSplit::Fill, _) => quiche::InitialCryptoSplit::Fill,
            (H3CryptoSplit::Even, H3CryptoReorder::None) => quiche::InitialCryptoSplit::Even,
            (H3CryptoSplit::Even, H3CryptoReorder::SniMidpoint) => {
                quiche::InitialCryptoSplit::EvenSniSlice
            }
        }
    }

    pub(crate) fn compatible_versions(&self) -> Vec<u32> {
        let versions: Vec<u32> = self
            .transport_parameters
            .iter()
            .flatten()
            .filter_map(|param| param.versions.as_ref())
            .flat_map(|info| info.available.iter().copied())
            .collect();
        if versions.is_empty() {
            vec![quiche::PROTOCOL_VERSION]
        } else {
            versions
        }
    }
}

fn version_information(info: &H3VersionInformation) -> Vec<u8> {
    let mut versions = info.available.clone();
    if let Some(position) = info.grease {
        let grease = rand::rng().random::<u32>() & 0xf0f0_f0f0 | 0x0a0a_0a0a;
        let at = match position {
            H3VersionGrease::First => 0,
            H3VersionGrease::Random => rand::rng().random_range(0..=versions.len()),
        };
        versions.insert(at, grease);
    }
    std::iter::once(info.chosen)
        .chain(versions)
        .flat_map(u32::to_be_bytes)
        .collect()
}

pub(crate) fn settings_plan(settings: &[H3Setting]) -> Vec<(u64, u64)> {
    settings
        .iter()
        .map(|setting| match &setting.grease {
            Some(grease) => (
                grease_id(grease.id_bits, H3_GREASE_BASE),
                random_bits(grease.value_bits),
            ),
            None => (
                setting.id.unwrap_or_default(),
                setting.value.unwrap_or_default(),
            ),
        })
        .collect()
}

pub(crate) fn control_frames(grease: Option<&H3Grease>) -> Vec<(u64, Vec<u8>)> {
    grease
        .map(|grease| {
            let len = rand::rng().random_range(0..=grease.max_len);
            (grease_id(grease.id_bits, H3_GREASE_BASE), random_bytes(len))
        })
        .into_iter()
        .collect()
}

fn random_bits(bits: u32) -> u64 {
    match bits {
        0 => 0,
        bits => rand::rng().random::<u64>() >> (64 - bits.min(62)),
    }
}

fn grease_id(bits: u32, base: u64) -> u64 {
    31 * random_bits(bits.min(57)) + base
}

fn varint(value: u64) -> Vec<u8> {
    match value {
        0..=0x3f => vec![value as u8],
        0x40..=0x3fff => (value as u16 | 0x4000).to_be_bytes().to_vec(),
        0x4000..=0x3fff_ffff => (value as u32 | 0x8000_0000).to_be_bytes().to_vec(),
        _ => (value | 0xc000_0000_0000_0000).to_be_bytes().to_vec(),
    }
}
