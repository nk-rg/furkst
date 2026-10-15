//! User preferences and session state, stored as JSON in the platform config directory.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub recent: Vec<PathBuf>,
    pub open_tabs: Vec<PathBuf>,
    pub active_tab: usize,
    pub theme: ThemeChoice,
    pub diff_context: usize,
    pub ignore_whitespace: bool,
    pub side_by_side: bool,
    /// Whether the commit details under the history are collapsed to their tab row.
    pub details_collapsed: bool,
    pub fetch_prune: bool,
    pub pull_rebase: bool,
    /// Where new clones go by default.
    pub clone_directory: Option<PathBuf>,
    /// Explicit git executable; `None` detects one automatically.
    pub git_path: Option<PathBuf>,
    /// Minutes between background fetches of every open repository; 0 turns it off.
    pub auto_fetch_minutes: u32,
    pub diff_font_size: f32,
    /// Installed font families for the interface and for code; `None` keeps the built-in ones.
    pub interface_font: Option<String>,
    pub code_font: Option<String>,
    /// Named groups of repositories shown on the start page.
    pub groups: Vec<RepoGroup>,
    /// Show the GitHub pictures of authors known to be GitHub users (see `avatars`).
    pub show_avatars: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoGroup {
    pub name: String,
    pub repos: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            open_tabs: Vec::new(),
            active_tab: 0,
            theme: ThemeChoice::default(),
            diff_context: 3,
            ignore_whitespace: false,
            side_by_side: false,
            details_collapsed: false,
            fetch_prune: true,
            pull_rebase: false,
            clone_directory: None,
            git_path: None,
            auto_fetch_minutes: 0,
            diff_font_size: 12.5,
            interface_font: None,
            code_font: None,
            groups: Vec::new(),
            show_avatars: false,
        }
    }
}

/// Furkst's own folders for configuration and caches, per platform.
pub fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("dev", "furkst", "Furkst")
}

impl Settings {
    /// The settings file: `$FURKST_SETTINGS` if set (handy for testing without touching real settings),
    /// otherwise `settings.json` in the platform's config directory.
    fn path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("FURKST_SETTINGS") {
            return Some(path.into());
        }
        project_dirs().map(|dirs| dirs.config_dir().join("settings.json"))
    }

    /// Loads the saved settings. A file that can't be read is kept as `settings.json.bak` before falling back to
    /// defaults, so the next save can't silently destroy the user's repositories and preferences.
    pub fn load() -> Self {
        let Some(path) = Self::path() else { return Self::default() };
        let Ok(bytes) = std::fs::read(&path) else { return Self::default() };
        Self::parse(&bytes).unwrap_or_else(|| {
            let _ = std::fs::copy(&path, path.with_extension("json.bak"));
            Self::default()
        })
    }

    /// Parses settings JSON, accepting the byte-order mark Notepad and PowerShell put at the start of UTF-8 files.
    fn parse(bytes: &[u8]) -> Option<Self> {
        let json = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        serde_json::from_slice(json).ok()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Every repository the user knows about: grouped ones first, then recent ones, without duplicates.
    pub fn known_repositories(&self) -> Vec<&PathBuf> {
        let mut all: Vec<&PathBuf> = self.groups.iter().flat_map(|g| &g.repos).chain(&self.recent).collect();
        let mut seen = std::collections::HashSet::new();
        all.retain(|path| seen.insert(*path));
        all
    }

    pub fn remember(&mut self, repo: PathBuf) {
        self.recent.retain(|p| p != &repo);
        self.recent.insert(0, repo);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn diff_options(&self) -> furkst_core::DiffOptions {
        furkst_core::DiffOptions { context_lines: self.diff_context, ignore_whitespace: self.ignore_whitespace }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_files_with_or_without_a_byte_order_mark() {
        let json = br#"{"recent": ["/repo"], "diff_context": 5}"#;
        let with_bom = [b"\xEF\xBB\xBF".as_slice(), json].concat();
        for bytes in [json.as_slice(), &with_bom] {
            let settings = Settings::parse(bytes).unwrap();
            assert_eq!((settings.recent.len(), settings.diff_context), (1, 5));
        }
        assert!(Settings::parse(b"not json").is_none());
    }
}
