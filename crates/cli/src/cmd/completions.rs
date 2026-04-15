//! `leyline completions <shell>` — emits shell completion scripts
//! via `clap_complete`. The generated script is written to stdout;
//! users redirect it into their shell's completion path.

use anyhow::Result;
use clap::CommandFactory;
use clap_complete::{generate, shells};

use crate::args::{Cli, CompletionShell};
use crate::exit::ExitCode;

pub fn run(shell: CompletionShell) -> Result<ExitCode> {
    let mut cmd = Cli::command();
    let bin = "leyline";
    let mut out = std::io::stdout();
    match shell {
        CompletionShell::Bash => generate(shells::Bash, &mut cmd, bin, &mut out),
        CompletionShell::Zsh => generate(shells::Zsh, &mut cmd, bin, &mut out),
        CompletionShell::Fish => generate(shells::Fish, &mut cmd, bin, &mut out),
        CompletionShell::Elvish => generate(shells::Elvish, &mut cmd, bin, &mut out),
        CompletionShell::Powershell => generate(shells::PowerShell, &mut cmd, bin, &mut out),
    }
    Ok(ExitCode::Ok)
}
