use std::io;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};

pub(crate) fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command.current_dir(dir);
    command
}

pub(crate) fn run_command(command: &mut Command) -> io::Result<Output> {
    let out = command.output().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "can't run {}: {e}\n{command:?} failed",
                command.get_program().to_string_lossy(),
            ),
        )
    })?;

    std::io::stderr().write_all(&out.stderr)?;
    std::io::stdout().write_all(&out.stdout)?;

    if !out.status.success() {
        let err = match out.status.code() {
            Some(code) => format!("{command:?} exited with status: {code}"),
            None => format!("{command:?} was terminated by signal"),
        };

        return Err(io::Error::other(err));
    }

    Ok(out)
}
