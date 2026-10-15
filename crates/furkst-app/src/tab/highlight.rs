//! Syntax colors for diffs, with the language definitions and themes `bat` uses (through `two-face`), so code reads
//! like in an editor.

use std::borrow::Cow;
use std::sync::OnceLock;

use egui::Color32;
use furkst_core::Hunk;
use two_face::re_exports::syntect::easy::HighlightLines;
use two_face::re_exports::syntect::highlighting::Theme;
use two_face::re_exports::syntect::parsing::SyntaxSet;
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

/// Bigger diffs stay plain: highlighting them would hold up the frame they arrive in.
const MAX_LINES: usize = 20_000;
/// Longer lines (minified code, data) stay plain, for the same reason.
const MAX_LINE_LEN: usize = 1_000;

/// The colored pieces of one line, as (end byte offset in its [`display_text`], color); empty for a plain line.
pub type Spans = Vec<(usize, Color32)>;

/// A diff line as it is drawn: tabs become four spaces.
pub fn display_text(text: &str) -> Cow<'_, str> {
    if text.contains('\t') { Cow::Owned(text.replace('\t', "    ")) } else { Cow::Borrowed(text) }
}

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

fn theme(dark: bool) -> &'static Theme {
    static THEMES: OnceLock<EmbeddedLazyThemeSet> = OnceLock::new();
    let themes = THEMES.get_or_init(two_face::theme::extra);
    themes.get(if dark { EmbeddedThemeName::OneHalfDark } else { EmbeddedThemeName::Github })
}

/// Colors for every line of every hunk of the file at `path`, or `None` when its language isn't known (or the diff
/// is too big). Hunks are highlighted one after the other, line by line, so a multi-line comment or string carries on.
pub fn highlight(path: &str, hunks: &[Hunk], dark: bool) -> Option<Vec<Vec<Spans>>> {
    if hunks.iter().map(|h| h.lines.len()).sum::<usize>() > MAX_LINES {
        return None;
    }
    let syntaxes = syntaxes();
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let extension = file_name.rsplit_once('.').map_or(file_name, |(_, extension)| extension);
    let syntax = syntaxes.find_syntax_by_extension(extension).or_else(|| syntaxes.find_syntax_by_extension(file_name))?;
    let theme = theme(dark);
    let colored = hunks
        .iter()
        .map(|hunk| {
            let mut highlighter = HighlightLines::new(syntax, theme);
            hunk.lines
                .iter()
                .map(|line| {
                    let text = display_text(&line.text);
                    if text.len() > MAX_LINE_LEN {
                        return Spans::new();
                    }
                    // These definitions expect each line to end with its newline.
                    let with_newline = format!("{text}\n");
                    let Ok(pieces) = highlighter.highlight_line(&with_newline, syntaxes) else { return Spans::new() };
                    let mut end = 0;
                    pieces
                        .into_iter()
                        .map(|(style, piece)| {
                            end = (end + piece.len()).min(text.len());
                            (end, Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b))
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    Some(colored)
}

#[cfg(test)]
mod tests {
    use furkst_core::{DiffLine, LineKind};

    use super::*;

    fn hunk(lines: &[&str]) -> Hunk {
        let lines: Vec<DiffLine> = lines
            .iter()
            .map(|text| DiffLine { kind: LineKind::Added, old_no: None, new_no: Some(1), text: (*text).to_owned() })
            .collect();
        Hunk { old_start: 0, old_len: 0, new_start: 0, new_len: lines.len(), lines }
    }

    #[test]
    fn colors_known_languages_by_extension() {
        let hunks = [hunk(&["fn main() {", "\tlet name = \"furkst\";", "}"])];
        let lines = highlight("src/main.rs", &hunks, true).expect("Rust is known");
        let distinct = |spans: &Spans| spans.iter().map(|(_, c)| *c).collect::<std::collections::HashSet<_>>().len();
        assert!(distinct(&lines[0][1]) > 1, "keyword, name and string get different colors");
        assert_eq!(lines[0][1].last().map(|(end, _)| *end), Some(display_text(&hunks[0].lines[1].text).len()));
        assert!(highlight("component.tsx", &hunks, false).is_some(), "TypeScript JSX is known too");
        assert!(highlight("notes.unknown-extension", &hunks, true).is_none());
    }
}
