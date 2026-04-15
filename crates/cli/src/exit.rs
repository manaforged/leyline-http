//! Exit code taxonomy.
//!
//! Every non-zero exit code is documented in `--help` and stable
//! across releases. Tests assert against these values, not the
//! underlying integers.

use leyline::Error as LeylineError;

/// The numeric exit code returned by the `leyline` binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    /// 2xx (or non-request subcommands that completed).
    Ok = 0,
    /// Usage error (e.g. unrecognized flag). clap emits 2 by convention.
    Usage = 2,
    /// Config error — bad profile name, bad proxy URL, unparseable header.
    Config = 3,
    /// Network-layer failure — DNS, TCP, TLS handshake, HTTP/2 handshake.
    Network = 4,
    /// HTTP protocol error — malformed response, unexpected framing.
    Protocol = 5,
    /// Request timeout hit.
    Timeout = 6,
    /// HTTP 4xx (matches `curl -f` convention).
    ClientError = 22,
    /// HTTP 5xx (matches `curl -f` convention).
    ServerError = 23,
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> Self {
        code as i32
    }
}

impl ExitCode {
    /// Map a response status code into the appropriate exit code.
    /// 2xx → Ok, 4xx → ClientError, 5xx → ServerError, otherwise Ok.
    pub fn from_status(status: u16) -> Self {
        match status {
            400..=499 => Self::ClientError,
            500..=599 => Self::ServerError,
            _ => Self::Ok,
        }
    }
}

impl From<&anyhow::Error> for ExitCode {
    fn from(err: &anyhow::Error) -> Self {
        if let Some(leyline_err) = err.downcast_ref::<LeylineError>() {
            return Self::from(leyline_err);
        }
        Self::Config
    }
}

impl From<&LeylineError> for ExitCode {
    fn from(err: &LeylineError) -> Self {
        match err {
            LeylineError::Config(_) | LeylineError::Url(_) => Self::Config,
            LeylineError::Io(_) | LeylineError::Tls(_) | LeylineError::PinningFailed { .. } => {
                Self::Network
            }
            LeylineError::Timeout => Self::Timeout,
            LeylineError::Http(_) | LeylineError::Json(_) => Self::Protocol,
            LeylineError::Status { code, .. } => Self::from_status(*code),
            // Error is #[non_exhaustive] — future variants default to Config.
            _ => Self::Config,
        }
    }
}
