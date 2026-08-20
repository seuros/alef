//! Locating toolchain executables before spawning them.

/// Build a [`std::process::Command`] for `program`, resolving it on `PATH` first.
///
/// Windows resolves a bare program name by appending `.exe` and nothing else, while
/// [`which::which`] resolves it against the whole of `PATHEXT`. A toolchain shipped as a batch
/// shim -- `kotlinc.bat`, `gradle.bat`, `mix.bat`, `npm.cmd` -- is therefore *found* by every
/// `is_available()` check in this crate and then fails to spawn at all, with
/// `NotFound: program not found` and no mention of the extension. Handing the resolved path to
/// `Command` closes that gap: the path carries the `.bat`/`.cmd` suffix, which std routes
/// through `cmd.exe`.
///
/// Falls back to the bare name when resolution fails, so the caller still gets std's own
/// spawn error rather than a different one from here. ~keep
pub fn tool_command(program: &str) -> std::process::Command {
    which::which(program).map_or_else(|_| std::process::Command::new(program), std::process::Command::new)
}

#[cfg(test)]
mod tests {
    use super::tool_command;

    /// A resolvable tool must be spawned by absolute path, not by the bare name it was asked
    /// for -- that substitution is the entire fix, and a `Command` still holding the bare name
    /// is the Windows failure this guards. ~keep
    #[test]
    fn a_resolvable_tool_is_spawned_by_its_resolved_path() {
        let resolved = which::which("cargo").expect("cargo is on PATH for the test suite");

        let command = tool_command("cargo");

        assert_eq!(std::path::Path::new(command.get_program()), resolved);
        assert!(std::path::Path::new(command.get_program()).is_absolute());
    }

    /// An unresolvable name must still produce a runnable `Command` so the caller reports std's
    /// own spawn failure, rather than this helper inventing an error of its own. ~keep
    #[test]
    fn an_unresolvable_tool_falls_back_to_the_bare_program_name() {
        let command = tool_command("alef-no-such-toolchain-exists");

        assert_eq!(command.get_program(), "alef-no-such-toolchain-exists");
    }
}
