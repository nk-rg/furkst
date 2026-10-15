//! The one place that shells out to the `git` executable, for operations gitoxide does not provide yet.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, PoisonError, RwLock};

use crate::error::{Error, Result};

#[derive(Clone, Debug)]
pub struct GitCli {
    workdir: PathBuf,
}

impl GitCli {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self { workdir: workdir.into() }
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Runs `git <args>` and returns its combined output, or an error carrying stderr.
    pub fn run<S: AsRef<str>>(&self, args: &[S]) -> Result<String> {
        self.exec(args, &[], &|_| {})
    }

    /// Like [`run`](Self::run) with extra environment variables, which override the defaults.
    pub fn run_env<S: AsRef<str>>(&self, args: &[S], envs: &[(&str, &str)]) -> Result<String> {
        self.exec(args, envs, &|_| {})
    }

    /// Like [`run`](Self::run), calling `on_progress` with each progress line git prints while it works
    /// (e.g. `Receiving objects:  45% (450/1000)`).
    pub fn run_with_progress<S: AsRef<str>>(&self, args: &[S], on_progress: &dyn Fn(&str)) -> Result<String> {
        self.exec(args, &[], on_progress)
    }

    fn exec<S: AsRef<str>>(&self, args: &[S], envs: &[(&str, &str)], on_progress: &dyn Fn(&str)) -> Result<String> {
        let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
        let mut child = self
            .command(&args)
            .envs(envs.iter().copied())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| Error::msg(format!("could not run git (is it installed and on PATH?): {err}")))?;
        let mut stdout_pipe = child.stdout.take().expect("stdout is piped");
        // Read stdout concurrently so a full pipe can't block git while we stream stderr.
        let stdout_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout_pipe.read_to_end(&mut buf);
            buf
        });
        let stderr = read_reporting_lines(child.stderr.take().expect("stderr is piped"), on_progress)?;
        let stdout = stdout_reader.join().unwrap_or_default();
        let status = child.wait()?;
        let (stdout, stderr) = (terminal_text(&stdout), terminal_text(&stderr));
        if status.success() {
            Ok(format!("{stdout}{stderr}").trim().to_owned())
        } else {
            let details = if stderr.trim().is_empty() { stdout } else { stderr };
            Err(Error::Command { command: args.join(" "), stderr: details.trim().to_owned() })
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(git_executable());
        cmd.args(args)
            .current_dir(&self.workdir)
            .stdin(Stdio::null())
            // Never block waiting for input on a terminal nobody sees; GUI credential helpers still work.
            .env("GIT_TERMINAL_PROMPT", "0")
            // Operations that would open an editor accept the prepared message instead.
            .env("GIT_EDITOR", "true")
            .env("GIT_SEQUENCE_EDITOR", "true");
        if let Some(path) = gui_path() {
            cmd.env("PATH", path);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }
}

/// Places git is commonly installed in, for apps launched without a login shell's `PATH`
/// (on macOS, apps started from Finder only see `/usr/bin:/bin:/usr/sbin:/sbin`).
const EXTRA_BIN_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

/// A user-chosen git executable (from preferences); takes precedence over automatic detection.
static GIT_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Sets (or, with `None` or an empty path, clears) the git executable to use from now on.
pub fn set_git_executable(path: Option<PathBuf>) {
    *GIT_OVERRIDE.write().unwrap_or_else(PoisonError::into_inner) = path.filter(|p| !p.as_os_str().is_empty());
}

/// The output of `git --version` for the executable currently in use.
pub fn git_version() -> Result<String> {
    GitCli::new(std::env::temp_dir()).run(&["--version"])
}

/// The git executable: the preferences override, else `$FURKST_GIT`, else `git` from `PATH`,
/// else the first one found in [`EXTRA_BIN_DIRS`].
fn git_executable() -> PathBuf {
    if let Some(path) = GIT_OVERRIDE.read().unwrap_or_else(PoisonError::into_inner).clone() {
        return path;
    }
    static GIT: OnceLock<PathBuf> = OnceLock::new();
    GIT.get_or_init(|| {
        if let Some(git) = std::env::var_os("FURKST_GIT") {
            return git.into();
        }
        // Windows finds git through `PATH` alone. Probing it there would also flash a console window: this runs before
        // `command` hides git's consoles.
        if cfg!(windows) || Command::new("git").arg("--version").stdin(Stdio::null()).output().is_ok_and(|o| o.status.success()) {
            return "git".into();
        }
        EXTRA_BIN_DIRS.iter().map(|dir| Path::new(dir).join("git")).find(|git| git.is_file()).unwrap_or_else(|| "git".into())
    })
    .clone()
}

/// `PATH` extended with [`EXTRA_BIN_DIRS`] on Unix, so hooks and credential helpers git spawns are found too.
fn gui_path() -> Option<&'static std::ffi::OsStr> {
    static PATH: OnceLock<Option<std::ffi::OsString>> = OnceLock::new();
    PATH.get_or_init(|| {
        if cfg!(windows) {
            return None;
        }
        let current = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs: Vec<PathBuf> = std::env::split_paths(&current).collect();
        for extra in EXTRA_BIN_DIRS.iter().map(PathBuf::from) {
            if !dirs.contains(&extra) {
                dirs.push(extra);
            }
        }
        std::env::join_paths(dirs).ok()
    })
    .as_deref()
}

/// Reads `pipe` to the end, calling `on_line` with every non-empty line as soon as it is complete. Git redraws
/// progress lines in place with `\r`, so both `\r` and `\n` end a line.
fn read_reporting_lines(mut pipe: impl Read, on_line: &dyn Fn(&str)) -> std::io::Result<Vec<u8>> {
    let mut all = Vec::new();
    let mut line_start = 0;
    let mut chunk = [0; 4096];
    loop {
        let read = pipe.read(&mut chunk)?;
        if read == 0 {
            return Ok(all);
        }
        for &byte in &chunk[..read] {
            if byte == b'\r' || byte == b'\n' {
                let line = String::from_utf8_lossy(&all[line_start..]);
                if !line.trim().is_empty() {
                    on_line(line.trim());
                }
                all.push(byte);
                line_start = all.len();
            } else {
                all.push(byte);
            }
        }
    }
}

/// The text a terminal would end up showing: of each line redrawn with `\r`, only the last version.
fn terminal_text(output: &[u8]) -> String {
    String::from_utf8_lossy(output)
        .split('\n')
        .map(|line| line.trim_end_matches('\r').rsplit('\r').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn reports_progress_lines_and_keeps_the_final_text() {
        let output = b"Receiving objects:  50% (1/2)\rReceiving objects: 100% (2/2), done.\nDone\r\n";
        let lines = RefCell::new(Vec::new());
        let all = read_reporting_lines(&output[..], &|line| lines.borrow_mut().push(line.to_owned())).unwrap();
        assert_eq!(lines.into_inner(), ["Receiving objects:  50% (1/2)", "Receiving objects: 100% (2/2), done.", "Done"]);
        assert_eq!(terminal_text(&all), "Receiving objects: 100% (2/2), done.\nDone\n");
    }
}
