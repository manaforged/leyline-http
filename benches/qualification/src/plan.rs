use std::time::Duration;

pub const ID_HEADER: &str = "x-qualify-id";
pub const SIZE_HEADER: &str = "x-qualify-size";

pub fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fault {
    None,
    CloseBeforeResponse,
    CloseMidHead,
    TruncatedLength,
    TruncatedChunked,
    MalformedHead,
    Stall,
    SlowDrip,
    CloseAfterResponse,
    Busy,
    Delay(Duration),
}

const FAULTS: [Fault; 9] = [
    Fault::CloseBeforeResponse,
    Fault::CloseMidHead,
    Fault::TruncatedLength,
    Fault::TruncatedChunked,
    Fault::MalformedHead,
    Fault::Stall,
    Fault::SlowDrip,
    Fault::CloseAfterResponse,
    Fault::Busy,
];

#[derive(Clone, Copy, Debug)]
pub struct Planned {
    pub fault: Fault,
    pub sticky: bool,
}

pub fn chaos_plan(seed: u64, id: u64, percent: u64) -> Planned {
    let roll = mix(seed ^ mix(id));
    if roll % 100 >= percent {
        return Planned {
            fault: Fault::None,
            sticky: false,
        };
    }
    Planned {
        fault: FAULTS[((roll >> 8) % FAULTS.len() as u64) as usize],
        sticky: (roll >> 16) & 1 == 1,
    }
}

pub fn stall() -> Planned {
    Planned {
        fault: Fault::Stall,
        sticky: true,
    }
}

pub fn sticky_fault(seed: u64, id: u64) -> Planned {
    Planned {
        fault: FAULTS[(mix(seed ^ mix(id)) % FAULTS.len() as u64) as usize],
        sticky: true,
    }
}

impl Planned {
    pub fn applies(self, attempt: u32) -> Fault {
        if self.sticky || attempt == 1 {
            self.fault
        } else {
            Fault::None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expected {
    Success,
    Status503,
    Failure,
    Timeout,
    Either,
}

pub fn expected(planned: Planned) -> Expected {
    match (planned.fault, planned.sticky) {
        (Fault::None | Fault::SlowDrip | Fault::CloseAfterResponse | Fault::Delay(_), _) => {
            Expected::Success
        }
        (Fault::Busy, true) => Expected::Status503,
        (Fault::Stall, true) => Expected::Timeout,
        (_, true) => Expected::Failure,
        (_, false) => Expected::Either,
    }
}

pub fn body_for(id: u64) -> String {
    format!("id={id}\n")
}
