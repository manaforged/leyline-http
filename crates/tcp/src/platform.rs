//! Platform-specific TCP socket options (MSS, DF bit, window scale).

use socket2::Socket;

use crate::{log_once, TcpProfile};

#[cfg(target_os = "linux")]
pub fn apply_platform_options(socket: &Socket, profile: &TcpProfile) {
    use std::os::unix::io::AsRawFd;
    let fd = socket.as_raw_fd();

    if profile.mss > 0 {
        let val = profile.mss as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_MAXSEG,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            log_once("TCP_MAXSEG", &std::io::Error::last_os_error());
        }
    }

    if profile.df {
        let val: libc::c_int = 2; // IP_PMTUDISC_DO
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_IP,
                libc::IP_MTU_DISCOVER,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            log_once("IP_MTU_DISCOVER", &std::io::Error::last_os_error());
        }
    }

    if profile.window_scale > 0 {
        const TCP_WINDOW_CLAMP: libc::c_int = 10;
        let clamp = (profile.window_size as u64)
            .checked_shl(profile.window_scale)
            .unwrap_or(u32::MAX as u64)
            .min(i32::MAX as u64) as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                TCP_WINDOW_CLAMP,
                &clamp as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            log_once("TCP_WINDOW_CLAMP", &std::io::Error::last_os_error());
        }
    }
}

#[cfg(target_os = "macos")]
pub fn apply_platform_options(socket: &Socket, profile: &TcpProfile) {
    use std::os::unix::io::AsRawFd;
    let fd = socket.as_raw_fd();

    if profile.mss > 0 {
        let val = profile.mss as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                2, // TCP_MAXSEG on macOS
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            log_once("TCP_MAXSEG", &std::io::Error::last_os_error());
        }
    }

    if profile.df {
        let val: libc::c_int = 1;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_IP,
                67, // IP_DONTFRAG on macOS
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            log_once("IP_DONTFRAG", &std::io::Error::last_os_error());
        }
    }
}

#[cfg(target_os = "windows")]
pub fn apply_platform_options(socket: &Socket, profile: &TcpProfile) {
    use std::os::windows::io::AsRawSocket;

    if profile.df {
        extern "system" {
            fn setsockopt(
                s: usize,
                level: i32,
                optname: i32,
                optval: *const u8,
                optlen: i32,
            ) -> i32;
        }
        let val: u32 = 1;
        let ret = unsafe {
            setsockopt(
                socket.as_raw_socket() as usize,
                0,  // IPPROTO_IP
                14, // IP_DONTFRAGMENT
                &val as *const u32 as *const u8,
                std::mem::size_of::<u32>() as i32,
            )
        };
        if ret != 0 {
            log_once("IP_DONTFRAGMENT", &std::io::Error::last_os_error());
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn apply_platform_options(_socket: &Socket, _profile: &TcpProfile) {}
