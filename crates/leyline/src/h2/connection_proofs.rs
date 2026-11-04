use super::PeerSettings;
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

const MAX_WINDOW: u32 = 0x7FFF_FFFF;
const MIN_FRAME: u32 = 1 << 14;
const MAX_FRAME: u32 = (1 << 24) - 1;

fn unformatted(_: std::fmt::Arguments<'_>) -> String {
    String::new()
}

fn rfc_9113_section_6_5_2(id: u16, value: u32) -> Option<ErrorCode> {
    match id {
        0x2 if value != 0 => Some(ErrorCode::ProtocolError),
        0x4 if value > MAX_WINDOW => Some(ErrorCode::FlowControlError),
        0x5 if !(MIN_FRAME..=MAX_FRAME).contains(&value) => Some(ErrorCode::ProtocolError),
        0x8 if value > 1 => Some(ErrorCode::ProtocolError),
        _ => None,
    }
}

#[kani::proof]
#[kani::stub(std::fmt::format, unformatted)]
fn a_server_setting_follows_rfc_9113_section_6_5_2() {
    let id: u16 = kani::any();
    let value: u32 = kani::any();
    let mut settings = PeerSettings::default();
    let before = settings.clone();
    let result = settings.apply(&[(id, value)]);
    match (rfc_9113_section_6_5_2(id, value), result) {
        (Some(code), Err(H2Error::Connection { code: got, .. })) => assert_eq!(got, code),
        (None, Ok(applied)) => {
            match id {
                0x1 => assert_eq!(settings.header_table_size, value),
                0x2 => assert!(!settings.enable_push),
                0x3 => assert_eq!(settings.max_concurrent_streams, Some(value)),
                0x4 => assert_eq!(settings.initial_window_size, value),
                0x5 => assert_eq!(settings.max_frame_size, value),
                0x6 => assert_eq!(settings.max_header_list_size, Some(value)),
                0x8 => assert_eq!(settings.enable_connect_protocol, value == 1),
                _ => {
                    assert_eq!(settings.header_table_size, before.header_table_size);
                    assert_eq!(settings.initial_window_size, before.initial_window_size);
                    assert_eq!(settings.max_frame_size, before.max_frame_size);
                }
            }
            let delta =
                i64::from(settings.initial_window_size) - i64::from(before.initial_window_size);
            assert_eq!(applied.window_size_delta, (delta != 0).then_some(delta));
        }
        _ => panic!("setting {id} = {value} does not follow RFC 9113 section 6.5.2"),
    }
}
