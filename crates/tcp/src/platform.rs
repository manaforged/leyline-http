//! Platform-specific TCP socket options (MSS, DF bit, window scale).
//!
//! Each platform has a thin wrapper around `setsockopt`. The `unsafe`
//! blocks below all share the same safety story:
//!
//! - The fd/raw-socket comes from a live [`socket2::Socket`] held by
//!   `apply_platform_options`, so it is a valid open descriptor for the
//!   duration of the call.
//! - The `optval` pointer refers to a stack-local that outlives the
//!   FFI call, and the passed `optlen` is `size_of::<T>()` of that
//!   stack-local.
//! - A non-zero return is logged (once per option) and treated as
//!   best-effort — profile application never poisons the socket.

use socket2::Socket;

use crate::{log_once, TcpProfile};

#[cfg(unix)]
/// # Safety
/// See module-level SAFETY. `fd` must be a valid open socket descriptor
/// owned by the caller; `val` is read by the kernel for `size_of::<i32>()`
/// bytes and must therefore point at an `i32`-sized value that lives for
/// the duration of the call.
unsafe fn set_int_opt(
    fd: libc::c_int,
    level: libc::c_int,
    opt: libc::c_int,
    val: libc::c_int,
    label: &'static str,
) {
    let ret = libc::setsockopt(
        fd,
        level,
        opt,
        &val as *const _ as *const libc::c_void,
        std::mem::size_of::<libc::c_int>() as libc::socklen_t,
    );
    if ret != 0 {
        log_once(label, &std::io::Error::last_os_error());
    }
}

#[cfg(target_os = "linux")]
pub fn apply_platform_options(socket: &Socket, profile: &TcpProfile) {
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
        // SAFETY: see module-level SAFETY.
        unsafe {
            set_int_opt(
                fd,
                libc::IPPROTO_IP,
                libc::IP_MTU_DISCOVER,
                2, // IP_PMTUDISC_DO
                "IP_MTU_DISCOVER",
            )
        };
    }

    if profile.window_scale > 0 {
        const TCP_WINDOW_CLAMP: libc::c_int = 10;
        let clamp = (profile.window_size as u64)
            .checked_shl(profile.window_scale)
            .unwrap_or(u32::MAX as u64)
            .min(i32::MAX as u64) as libc::c_int;
        // SAFETY: see module-level SAFETY.
        unsafe {
            set_int_opt(fd, libc::IPPROTO_TCP, TCP_WINDOW_CLAMP, clamp, "TCP_WINDOW_CLAMP")
        };
    }
}

#[cfg(target_os = "macos")]
pub fn apply_platform_options(socket: &Socket, profile: &TcpProfile) {
    use std::os::unix::io::AsRawFd;
    let fd = socket.as_raw_fd();

    if profile.mss > 0 {
        // SAFETY: see module-level SAFETY. macOS TCP_MAXSEG = 2.
        unsafe { set_int_opt(fd, libc::IPPROTO_TCP, 2, profile.mss as libc::c_int, "TCP_MAXSEG") };
    }

    if profile.df {
        // SAFETY: see module-level SAFETY. macOS IP_DONTFRAG = 67.
        unsafe { set_int_opt(fd, libc::IPPROTO_IP, 67, 1, "IP_DONTFRAG") };
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
        // SAFETY: `socket` is a live `Socket`, so `as_raw_socket()` is a
        // valid open SOCKET handle for the duration of this call. `val`
        // is a stack-local u32 that outlives the call, and the passed
        // length is exactly sizeof(u32).
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
