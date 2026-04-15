//! Subcommand dispatch — one match on the parsed `Cli`, one function
//! per subcommand. No clever indirection.

pub mod completions;
pub mod inspect;
pub mod profile;
pub mod request;
pub mod version;

use anyhow::Result;

use crate::args::{Cli, Command, VerbArgs};
use crate::exit::ExitCode;

/// Route parsed args to the right subcommand handler.
pub async fn dispatch(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Some(Command::Get(v)) => request::run("GET", v).await,
        Some(Command::Post(v)) => request::run("POST", v).await,
        Some(Command::Put(v)) => request::run("PUT", v).await,
        Some(Command::Patch(v)) => request::run("PATCH", v).await,
        Some(Command::Delete(v)) => request::run("DELETE", v).await,
        Some(Command::Head(v)) => request::run("HEAD", v).await,
        Some(Command::Fetch(mut v)) => {
            // `request::run` calls `apply_verbose` itself once the
            // flag is set — one source of truth, no double-apply.
            v.request.verbose = true;
            request::run("GET", v).await
        }
        Some(Command::Inspect(v)) => inspect::run(v).await,
        Some(Command::Profile(sub)) => profile::run(sub),
        Some(Command::Completions { shell }) => completions::run(shell),
        Some(Command::Version) => version::run(),
        None => {
            // Implicit GET form: `leyline <URL>` at the root.
            let url = cli
                .url
                .ok_or_else(|| anyhow::anyhow!("no URL given — run `leyline --help` for usage"))?;
            let v = VerbArgs {
                url,
                request: cli.request,
            };
            request::run("GET", v).await
        }
    }
}
