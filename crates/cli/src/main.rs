//! Entry point for the `leyline` binary. See [`leyline_cli`] for the
//! full subcommand and flag surface.

#![allow(missing_docs)]

use clap::Parser;
use leyline_cli::{run, Cli};

fn main() {
    let cli = Cli::parse();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("leyline: failed to start tokio runtime: {e}");
            std::process::exit(4);
        }
    };
    let code = rt.block_on(run(cli));
    std::process::exit(code.into());
}
