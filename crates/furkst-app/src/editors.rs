//! Code editors and IDEs installed on this computer, for opening a file at a line ("Reveal line in").

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// How an editor is told which line to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineArgument {
    /// `--goto file:line`: VS Code and its relatives.
    Goto,
    /// `file:line`: Sublime Text, Zed.
    Suffix,
    /// `--line line file`: JetBrains IDEs.
    Flag,
    /// `-nline file`: Notepad++.
    Notepad,
}

/// An editor Furkst knows how to find and drive.
struct Known {
    name: &'static str,
    /// Command names looked up on `PATH`.
    commands: &'static [&'static str],
    /// Usual install locations; `%NAME%` stands for an environment variable, `~` for the home folder.
    paths: &'static [&'static str],
    line: LineArgument,
}

const JETBRAINS: [(&str, &str); 8] = [
    ("IntelliJ IDEA", "idea"),
    ("RustRover", "rustrover"),
    ("WebStorm", "webstorm"),
    ("PyCharm", "pycharm"),
    ("CLion", "clion"),
    ("GoLand", "goland"),
    ("Rider", "rider"),
    ("PhpStorm", "phpstorm"),
];

const KNOWN: &[Known] = &[
    Known {
        name: "Visual Studio Code",
        commands: &["code"],
        paths: &[
            "%LOCALAPPDATA%/Programs/Microsoft VS Code/bin/code.cmd",
            "%ProgramFiles%/Microsoft VS Code/bin/code.cmd",
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
        ],
        line: LineArgument::Goto,
    },
    Known {
        name: "Visual Studio Code Insiders",
        commands: &["code-insiders"],
        paths: &["/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/bin/code-insiders"],
        line: LineArgument::Goto,
    },
    Known {
        name: "Cursor",
        commands: &["cursor"],
        paths: &[
            "%LOCALAPPDATA%/Programs/cursor/resources/app/bin/cursor.cmd",
            "/Applications/Cursor.app/Contents/Resources/app/bin/cursor",
        ],
        line: LineArgument::Goto,
    },
    Known {
        name: "Windsurf",
        commands: &["windsurf"],
        paths: &["/Applications/Windsurf.app/Contents/Resources/app/bin/windsurf"],
        line: LineArgument::Goto,
    },
    Known { name: "VSCodium", commands: &["codium"], paths: &[], line: LineArgument::Goto },
    Known {
        name: "Zed",
        commands: &["zed"],
        paths: &["%LOCALAPPDATA%/Programs/Zed/zed.exe", "/Applications/Zed.app/Contents/MacOS/cli"],
        line: LineArgument::Suffix,
    },
    Known {
        name: "Sublime Text",
        commands: &["subl"],
        paths: &["%ProgramFiles%/Sublime Text/subl.exe", "/Applications/Sublime Text.app/Contents/SharedSupport/bin/subl"],
        line: LineArgument::Suffix,
    },
    Known { name: "Notepad++", commands: &["notepad++"], paths: &["%ProgramFiles%/Notepad++/notepad++.exe"], line: LineArgument::Notepad },
];

pub struct Editor {
    pub name: String,
    program: PathBuf,
    line: LineArgument,
}

impl Editor {
    /// Opens `file` at `line` (1-based).
    pub fn open(&self, file: &Path, line: usize) -> std::io::Result<()> {
        let mut command = Command::new(&self.program);
        command.args(self.line.arguments(file, line));
        #[cfg(windows)]
        {
            // The `.cmd` launchers (VS Code's, JetBrains Toolbox's) would otherwise flash a console window.
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command.spawn().map(drop)
    }

    /// The shell command `git mergetool` runs to resolve a conflicted file in this editor's merge view, for editors
    /// that have one (VS Code and its relatives). It waits until the merge view closes.
    pub fn merge_command(&self) -> Option<String> {
        if self.line != LineArgument::Goto {
            return None;
        }
        // Git runs it with its own shell, which takes the shell script these editors ship beside their `.cmd`
        // launcher on Windows.
        let script = self.program.with_extension("");
        let program = if script != self.program && script.is_file() { script } else { self.program.clone() };
        let program = program.display().to_string().replace('\\', "/");
        Some(format!(r#""{program}" --wait --merge "$REMOTE" "$LOCAL" "$BASE" "$MERGED""#))
    }
}

impl LineArgument {
    fn arguments(self, file: &Path, line: usize) -> Vec<String> {
        let file = file.display().to_string();
        match self {
            LineArgument::Goto => vec!["--goto".to_owned(), format!("{file}:{line}")],
            LineArgument::Suffix => vec![format!("{file}:{line}")],
            LineArgument::Flag => vec!["--line".to_owned(), line.to_string(), file],
            LineArgument::Notepad => vec![format!("-n{line}"), file],
        }
    }
}

/// The editors found on this computer, looked for once.
pub fn installed() -> &'static [Editor] {
    static INSTALLED: OnceLock<Vec<Editor>> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let mut found: Vec<Editor> = KNOWN
            .iter()
            .filter_map(|known| {
                let program = known.commands.iter().find_map(|c| on_path(c)).or_else(|| known.paths.iter().find_map(|p| existing(p)))?;
                Some(Editor { name: known.name.to_owned(), program, line: known.line })
            })
            .collect();
        found.extend(jetbrains());
        found
    })
}

/// JetBrains IDEs: from `PATH`, JetBrains Toolbox's scripts, or a standalone install's `bin` folder.
fn jetbrains() -> Vec<Editor> {
    let toolbox = [
        expand("%LOCALAPPDATA%/JetBrains/Toolbox/scripts"),
        expand("~/Library/Application Support/JetBrains/Toolbox/scripts"),
        expand("~/.local/share/JetBrains/Toolbox/scripts"),
    ];
    let installs: Vec<PathBuf> = [expand("%ProgramFiles%/JetBrains"), Some(PathBuf::from("/Applications"))]
        .into_iter()
        .flatten()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    JETBRAINS
        .iter()
        .filter_map(|&(name, command)| {
            let in_toolbox = || toolbox.iter().flatten().find_map(|dir| with_extension(&dir.join(command)));
            let standalone = || {
                installs.iter().find_map(|install| {
                    [format!("bin/{command}64.exe"), format!("bin/{command}.exe"), format!("Contents/MacOS/{command}")]
                        .iter()
                        .map(|launcher| install.join(launcher))
                        .find(|launcher| launcher.is_file())
                })
            };
            let program = on_path(command).or_else(in_toolbox).or_else(standalone)?;
            Some(Editor { name: name.to_owned(), program, line: LineArgument::Flag })
        })
        .collect()
}

/// `command` found on `PATH`, with Windows' program extensions.
fn on_path(command: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| with_extension(&dir.join(command)))
}

/// `path` itself, or with an `.exe`, `.cmd` or `.bat` extension on Windows, if that file exists.
fn with_extension(path: &Path) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) { &["exe", "cmd", "bat"] } else { &[""] };
    extensions
        .iter()
        .map(|extension| if extension.is_empty() { path.to_path_buf() } else { path.with_extension(extension) })
        .find(|candidate| candidate.is_file())
}

fn existing(path: &str) -> Option<PathBuf> {
    expand(path).filter(|path| path.is_file())
}

/// `path` with `%NAME%` replaced by that environment variable and a leading `~` by the home folder; `None` when one
/// isn't set.
fn expand(path: &str) -> Option<PathBuf> {
    let mut expanded = String::new();
    let mut rest = path;
    if let Some(after) = rest.strip_prefix('~') {
        expanded.push_str(&std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok()?);
        rest = after;
    }
    while let Some(start) = rest.find('%') {
        let end = start + 1 + rest[start + 1..].find('%')?;
        expanded.push_str(&rest[..start]);
        expanded.push_str(&std::env::var(&rest[start + 1..end]).ok()?);
        rest = &rest[end + 1..];
    }
    expanded.push_str(rest);
    Some(PathBuf::from(expanded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_each_kind_of_editor_the_line() {
        let file = Path::new("src/main.rs");
        assert_eq!(LineArgument::Goto.arguments(file, 7), ["--goto", "src/main.rs:7"]);
        assert_eq!(LineArgument::Suffix.arguments(file, 7), ["src/main.rs:7"]);
        assert_eq!(LineArgument::Flag.arguments(file, 7), ["--line", "7", "src/main.rs"]);
        assert_eq!(LineArgument::Notepad.arguments(file, 7), ["-n7", "src/main.rs"]);
    }

    #[test]
    fn merges_in_editors_with_a_merge_view() {
        let editor = |program: &str, line| Editor { name: String::new(), program: PathBuf::from(program), line };
        let code = editor(r"C:\Tools\Code\bin\code.cmd", LineArgument::Goto);
        assert_eq!(
            code.merge_command().as_deref(),
            Some(r#""C:/Tools/Code/bin/code.cmd" --wait --merge "$REMOTE" "$LOCAL" "$BASE" "$MERGED""#)
        );
        assert_eq!(editor("/usr/bin/subl", LineArgument::Suffix).merge_command(), None);
    }

    #[test]
    #[ignore = "prints what this computer has"]
    fn lists_installed_editors() {
        for editor in installed() {
            println!("{} -> {}", editor.name, editor.program.display());
        }
    }

    #[test]
    fn expands_environment_variables_and_home() {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap();
        assert_eq!(expand("~/x"), Some(PathBuf::from(format!("{home}/x"))));
        let path = std::env::var("PATH").unwrap();
        assert_eq!(expand("a/%PATH%/b"), Some(PathBuf::from(format!("a/{path}/b"))));
        assert_eq!(expand("%FURKST_SURELY_UNSET%/b"), None);
    }
}
