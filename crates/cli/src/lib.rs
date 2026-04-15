//! `leyline` command-line interface.
//!
//! The binary lives at [`main`][bin], but every non-`main` symbol is
//! `pub` from this library crate so integration tests under `tests/`
//! can drive the CLI without respawning a child process.
//!
//! User-facing documentation is the `leyline --help` output (hand-
//! tuned in [`args`]) — we deliberately do not duplicate it as
//! rustdoc, so this crate opts out of the workspace's `missing_docs`
//! lint.
//!
//! [bin]: ../../bin/leyline.rs

#![allow(missing_docs)]

pub mod args;
pub mod cmd;
pub mod exit;
pub mod output;
pub mod session;

pub use args::Cli;
pub use exit::ExitCode;

/// Run the CLI with the given parsed arguments. Returns an
/// [`ExitCode`] the caller can translate into a `std::process::exit`
/// value. Keeping this in the library crate makes it callable from
/// integration tests.
pub async fn run(cli: Cli) -> ExitCode {
    match cmd::dispatch(cli).await {
        Ok(code) => code,
        Err(err) => {
            eprint_error(&err);
            ExitCode::from(&err)
        }
    }
}

fn eprint_error(err: &anyhow::Error) {
    use owo_colors::OwoColorize;
    let mut stderr = anstream::stderr().lock();
    let _ = std::io::Write::write_all(
        &mut stderr,
        format!("{}: {err}\n", "error".red().bold()).as_bytes(),
    );
    let mut source = err.source();
    while let Some(cause) = source {
        let _ = std::io::Write::write_all(
            &mut stderr,
            format!("  {} {cause}\n", "caused by:".dimmed()).as_bytes(),
        );
        source = cause.source();
    }
}
