//! Syntax colors for diffs, with the language definitions and themes `bat` uses (through `two-face`), so code reads
//! like in an editor.

use std::borrow::Cow;
use std::sync::OnceLock;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
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
    let shown: Vec<Vec<Cow<'_, str>>> = hunks.iter().map(|hunk| hunk.lines.iter().map(|line| display_text(&line.text)).collect()).collect();
    let blocks: Vec<Vec<&str>> = shown.iter().map(|lines| lines.iter().map(|line| line.as_ref()).collect()).collect();
    highlight_blocks(path, &blocks, dark)
}

/// Like [`highlight`], for blocks of lines (without line breaks) of the file at `path`; spans are offsets in the lines
/// as given.
pub fn highlight_blocks(path: &str, blocks: &[Vec<&str>], dark: bool) -> Option<Vec<Vec<Spans>>> {
    if blocks.iter().map(Vec::len).sum::<usize>() > MAX_LINES {
        return None;
    }
    let syntaxes = syntaxes();
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let extension = file_name.rsplit_once('.').map_or(file_name, |(_, extension)| extension);
    let syntax = syntaxes.find_syntax_by_extension(extension).or_else(|| syntaxes.find_syntax_by_extension(file_name))?;
    let theme = theme(dark);
    let colored = blocks
        .iter()
        .map(|lines| {
            let mut highlighter = HighlightLines::new(syntax, theme);
            lines
                .iter()
                .map(|text| {
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

/// Appends `text` to `job` in its syntax colors (`spans`), the rest in `color`.
pub fn append(job: &mut LayoutJob, text: &str, spans: Option<&Spans>, font: &FontId, color: Color32) {
    let mut start = 0;
    for &(end, span_color) in spans.into_iter().flatten() {
        job.append(&text[start..end], 0.0, TextFormat::simple(font.clone(), span_color));
        start = end;
    }
    job.append(&text[start..], 0.0, TextFormat::simple(font.clone(), color));
}

/// A whole text of the file at `path` laid out in its syntax colors, character for character (for an editable field).
pub fn code_job(path: &str, text: &str, font: &FontId, color: Color32, dark: bool) -> LayoutJob {
    let lines: Vec<&str> = text.split('\n').collect();
    let spans = highlight_blocks(path, std::slice::from_ref(&lines), dark).and_then(|mut blocks| blocks.pop());
    let mut job = LayoutJob::default();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            job.append("\n", 0.0, TextFormat::simple(font.clone(), color));
        }
        append(&mut job, line, spans.as_ref().and_then(|spans| spans.get(index)), font, color);
    }
    job
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
