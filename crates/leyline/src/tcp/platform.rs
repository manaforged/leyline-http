use socket2::Socket;

use crate::tcp::{TcpProfile, log_once};

#[cfg(unix)]
unsafe fn set_int_opt(
    fd: libc::c_int,
    level: libc::c_int,
    opt: libc::c_int,
    val: libc::c_int,
    label: &'static str,
) {
    // SAFETY: see the `# Safety` contract above — `fd` is a live socket and `&val` outlives the call. Edition 2024 requires the unsafe op to sit in an explicit `unsafe` block even inside an `unsafe fn`.
    let ret = unsafe {
        libc::setsockopt(
            fd,
            level,
            opt,
            &val as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if ret != 0 {
        log_once(label, &std::io::Error::last_os_error());
    }
}

#[cfg(target_os = "linux")]
pub(super) fn apply_platform_options(socket: &Socket, profile: &TcpProfile, is_v6: bool) {
    use std::os::unix::io::AsRawFd;
    let fd = socket.as_raw_fd();

    if profile.mss > 0 {
        // SAFETY: see module-level SAFETY; fd lives for the call.
        unsafe {
            set_int_opt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_MAXSEG,
                profile.mss as libc::c_int,
                "TCP_MAXSEG",
            )
        };
    }

    if profile.df {
        let (level, opt, label) = if is_v6 {
            (
                libc::IPPROTO_IPV6,
                libc::IPV6_MTU_DISCOVER,
                "IPV6_MTU_DISCOVER",
            )
        } else {
            (libc::IPPROTO_IP, libc::IP_MTU_DISCOVER, "IP_MTU_DISCOVER")
        };
        // SAFETY: see module-level SAFETY.
        unsafe { set_int_opt(fd, level, opt, 2, label) };
    }

    if profile.window_scale > 0 {
        const TCP_WINDOW_CLAMP: libc::c_int = 10;
        let clamp = (profile.window_size as u64)
            .checked_shl(profile.window_scale)
            .unwrap_or(u32::MAX as u64)
            .min(i32::MAX as u64) as libc::c_int;
        // SAFETY: see module-level SAFETY.
        unsafe {
            set_int_opt(
                fd,
                libc::IPPROTO_TCP,
                TCP_WINDOW_CLAMP,
                clamp,
                "TCP_WINDOW_CLAMP",
            )
        };
    }
}

#[cfg(target_os = "macos")]
pub(super) fn apply_platform_options(socket: &Socket, profile: &TcpProfile, is_v6: bool) {
    use std::os::unix::io::AsRawFd;
    let fd = socket.as_raw_fd();

    if profile.mss > 0 {
        // SAFETY: see module-level SAFETY. macOS TCP_MAXSEG = 2.
        unsafe {
            set_int_opt(
                fd,
                libc::IPPROTO_TCP,
                2,
                profile.mss as libc::c_int,
                "TCP_MAXSEG",
            )
        };
    }

    if profile.df {
        let (level, opt, label) = if is_v6 {
            (libc::IPPROTO_IPV6, 62, "IPV6_DONTFRAG")
        } else {
            (libc::IPPROTO_IP, 28, "IP_DONTFRAG")
        };
        // SAFETY: see module-level SAFETY.
        unsafe { set_int_opt(fd, level, opt, 1, label) };
    }
}

#[cfg(target_os = "windows")]
pub(super) fn apply_platform_options(socket: &Socket, profile: &TcpProfile, is_v6: bool) {
    use std::os::windows::io::AsRawSocket;

    if profile.df {
        unsafe extern "system" {
            fn setsockopt(
                s: usize,
                level: i32,
                optname: i32,
                optval: *const u8,
                optlen: i32,
            ) -> i32;
        }
        let (level, optname, label) = if is_v6 {
            (41, 14, "IPV6_DONTFRAG")
        } else {
            (0, 14, "IP_DONTFRAGMENT")
        };
        let val: u32 = 1;
        // SAFETY: `socket` is a live `Socket`, so `as_raw_socket()` is a valid open SOCKET handle for the duration of this call. `val` is a stack-local u32 that outlives the call, and the passed length is exactly sizeof(u32).
        let ret = unsafe {
            setsockopt(
                socket.as_raw_socket() as usize,
                level,
                optname,
                &val as *const u32 as *const u8,
                std::mem::size_of::<u32>() as i32,
            )
        };
        if ret != 0 {
            log_once(label, &std::io::Error::last_os_error());
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) fn apply_platform_options(_socket: &Socket, _profile: &TcpProfile, _is_v6: bool) {}
