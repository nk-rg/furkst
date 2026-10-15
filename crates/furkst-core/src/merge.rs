//! Parsing of conflict markers in a working tree file, and assembling the merged result from per-region choices.

use crate::error::{Error, Result};
use crate::{FileChange, Repo};

/// One `<<<<<<<` … `>>>>>>>` block. Texts keep their line terminators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictRegion {
    pub ours: String,
    pub ours_label: String,
    /// The common ancestor, present for `diff3`/`zdiff3` style markers.
    pub base: Option<String>,
    pub base_label: String,
    pub theirs: String,
    pub theirs_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Common(String),
    Conflict(ConflictRegion),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Ours,
    Theirs,
    OursThenTheirs,
    TheirsThenOurs,
}

impl Resolution {
    pub const ALL: [Self; 4] = [Self::Ours, Self::Theirs, Self::OursThenTheirs, Self::TheirsThenOurs];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ours => "Use ours",
            Self::Theirs => "Use theirs",
            Self::OursThenTheirs => "Ours, then theirs",
            Self::TheirsThenOurs => "Theirs, then ours",
        }
    }
}

impl ConflictRegion {
    /// The text this region contributes to the merged file under `resolution`.
    pub fn resolved(&self, resolution: Resolution) -> String {
        match resolution {
            Resolution::Ours => self.ours.clone(),
            Resolution::Theirs => self.theirs.clone(),
            Resolution::OursThenTheirs => format!("{}{}", self.ours, self.theirs),
            Resolution::TheirsThenOurs => format!("{}{}", self.theirs, self.ours),
        }
    }
}

/// Number of conflict regions in `segments`.
pub fn conflict_count(segments: &[Segment]) -> usize {
    segments.iter().filter(|s| matches!(s, Segment::Conflict(_))).count()
}

/// If `line` (without terminator) is a marker of kind `ch`, returns its label.
fn marker(line: &str, ch: char) -> Option<&str> {
    let rest = line.strip_prefix(&ch.to_string().repeat(7))?;
    match rest.strip_prefix(' ') {
        Some(label) => Some(label.trim()),
        None => rest.is_empty().then_some(""),
    }
}

/// Splits `text` into common text and conflict regions.
pub fn parse_conflicts(text: &str) -> Result<Vec<Segment>> {
    #[derive(PartialEq)]
    enum Part {
        Ours,
        Base,
        Theirs,
    }
    let mut segments = Vec::new();
    let mut common = String::new();
    let mut open: Option<(ConflictRegion, Part, usize)> = None;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let number = index + 1;
        let bare = line.trim_end_matches(['\n', '\r']);
        let Some((region, part, start)) = &mut open else {
            if let Some(label) = marker(bare, '<') {
                if !common.is_empty() {
                    segments.push(Segment::Common(std::mem::take(&mut common)));
                }
                let region = ConflictRegion {
                    ours: String::new(),
                    ours_label: label.to_owned(),
                    base: None,
                    base_label: String::new(),
                    theirs: String::new(),
                    theirs_label: String::new(),
                };
                open = Some((region, Part::Ours, number));
            } else if marker(bare, '>').is_some() {
                return Err(Error::msg(format!("unexpected conflict marker on line {number}")));
            } else {
                common.push_str(line);
            }
            continue;
        };
        if marker(bare, '<').is_some() {
            return Err(Error::msg(format!("nested conflict marker on line {number} (region opened on line {start})")));
        }
        if let Some(label) = marker(bare, '|').filter(|_| *part == Part::Ours) {
            region.base = Some(String::new());
            region.base_label = label.to_owned();
            *part = Part::Base;
        } else if bare == "=======" && *part != Part::Theirs {
            *part = Part::Theirs;
        } else if let Some(label) = marker(bare, '>').filter(|_| *part == Part::Theirs) {
            region.theirs_label = label.to_owned();
            if let Some((region, ..)) = open.take() {
                segments.push(Segment::Conflict(region));
            }
        } else {
            match part {
                Part::Ours => region.ours.push_str(line),
                Part::Base => region.base.get_or_insert_with(String::new).push_str(line),
                Part::Theirs => region.theirs.push_str(line),
            }
        }
    }
    if let Some((.., start)) = open {
        return Err(Error::msg(format!("conflict region opened on line {start} is never closed")));
    }
    if !common.is_empty() {
        segments.push(Segment::Common(common));
    }
    Ok(segments)
}

/// Builds the merged text; `choices` has one entry per conflict region (in order). A region without a choice
/// contributes nothing, which is only meant for previews.
pub fn assemble(segments: &[Segment], choices: &[Option<Resolution>]) -> String {
    let mut regions = choices.iter();
    segments
        .iter()
        .map(|segment| match segment {
            Segment::Common(text) => text.clone(),
            Segment::Conflict(region) => regions.next().copied().flatten().map_or_else(String::new, |r| region.resolved(r)),
        })
        .collect()
}

impl Repo {
    /// Reads and parses the conflict markers of a working tree file.
    pub fn conflict_segments(&self, path: &str) -> Result<Vec<Segment>> {
        let bytes = std::fs::read(self.workdir().join(path))?;
        let text = String::from_utf8(bytes).map_err(|_| Error::msg("the file is binary or not valid UTF-8"))?;
        parse_conflicts(&text)
    }

    /// Writes the resolved file as is (it is already in working tree form) and stages it, marking it resolved.
    pub(crate) fn resolve_with_content(&self, change: &FileChange, content: &str) -> Result<()> {
        std::fs::write(self.workdir().join(&change.path), content)?;
        self.stage(std::slice::from_ref(change))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(segments: &[Segment], n: usize) -> &ConflictRegion {
        segments.iter().filter_map(|s| if let Segment::Conflict(r) = s { Some(r) } else { None }).nth(n).unwrap()
    }

    #[test]
    fn single_region_with_labels() {
        let s = parse_conflicts("a\n<<<<<<< HEAD\nmine\n=======\ntheirs\n>>>>>>> feature\nz\n").unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], Segment::Common("a\n".into()));
        let r = region(&s, 0);
        assert_eq!((r.ours.as_str(), r.theirs.as_str()), ("mine\n", "theirs\n"));
        assert_eq!((r.ours_label.as_str(), r.theirs_label.as_str()), ("HEAD", "feature"));
        assert_eq!(r.base, None);
        assert_eq!(s[2], Segment::Common("z\n".into()));
    }

    #[test]
    fn multiple_regions() {
        let s = parse_conflicts("<<<<<<< a\n1\n=======\n2\n>>>>>>> b\nmid\n<<<<<<< a\n3\n=======\n>>>>>>> b\n").unwrap();
        assert_eq!(conflict_count(&s), 2);
        assert_eq!(s[1], Segment::Common("mid\n".into()));
        assert_eq!(region(&s, 1).theirs, "");
        assert_eq!(region(&s, 1).ours, "3\n");
    }

    #[test]
    fn diff3_base_section() {
        let s = parse_conflicts("<<<<<<< HEAD\nours\n||||||| merged common ancestors\nbase1\nbase2\n=======\ntheirs\n>>>>>>> topic\n")
            .unwrap();
        let r = region(&s, 0);
        assert_eq!(r.base.as_deref(), Some("base1\nbase2\n"));
        assert_eq!(r.base_label, "merged common ancestors");
        assert_eq!(r.ours, "ours\n");
        assert_eq!(r.theirs, "theirs\n");
    }

    #[test]
    fn empty_base_section_is_present() {
        let s = parse_conflicts("<<<<<<< a\nx\n|||||||\n=======\ny\n>>>>>>> b\n").unwrap();
        assert_eq!(region(&s, 0).base.as_deref(), Some(""));
    }

    #[test]
    fn markers_at_start_and_end_without_trailing_newline() {
        let s = parse_conflicts("<<<<<<< a\nx\n=======\ny\n>>>>>>> b").unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(region(&s, 0).theirs_label, "b");
        assert_eq!(assemble(&s, &[Some(Resolution::OursThenTheirs)]), "x\ny\n");
    }

    #[test]
    fn common_text_without_trailing_newline_is_kept() {
        let s = parse_conflicts("<<<<<<< a\nx\n=======\ny\n>>>>>>> b\ntail").unwrap();
        assert_eq!(assemble(&s, &[Some(Resolution::Theirs)]), "y\ntail");
    }

    #[test]
    fn crlf_is_preserved() {
        let s = parse_conflicts("a\r\n<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> topic\r\nz\r\n").unwrap();
        let r = region(&s, 0);
        assert_eq!((r.ours.as_str(), r.theirs.as_str()), ("x\r\n", "y\r\n"));
        assert_eq!((r.ours_label.as_str(), r.theirs_label.as_str()), ("HEAD", "topic"));
        assert_eq!(assemble(&s, &[Some(Resolution::TheirsThenOurs)]), "a\r\ny\r\nx\r\nz\r\n");
    }

    #[test]
    fn no_markers_is_all_common() {
        let s = parse_conflicts("just text\n").unwrap();
        assert_eq!(s, vec![Segment::Common("just text\n".into())]);
        assert_eq!(conflict_count(&s), 0);
        assert!(parse_conflicts("").unwrap().is_empty());
    }

    #[test]
    fn all_resolutions() {
        let s = parse_conflicts("<<<<<<< a\no\n=======\nt\n>>>>>>> b\n").unwrap();
        let out = |r| assemble(&s, &[Some(r)]);
        assert_eq!(out(Resolution::Ours), "o\n");
        assert_eq!(out(Resolution::Theirs), "t\n");
        assert_eq!(out(Resolution::OursThenTheirs), "o\nt\n");
        assert_eq!(out(Resolution::TheirsThenOurs), "t\no\n");
        assert_eq!(assemble(&s, &[None]), "");
    }

    #[test]
    fn malformed_input_is_an_error() {
        for bad in [
            "<<<<<<< a\nx\n",
            "<<<<<<< a\nx\n=======\ny\n",
            "<<<<<<< a\n<<<<<<< b\n=======\n>>>>>>> c\n",
            "stray\n>>>>>>> b\n",
            "<<<<<<< a\nx\n>>>>>>> b\n",
        ] {
            assert!(parse_conflicts(bad).is_err(), "{bad:?}");
        }
    }
}
