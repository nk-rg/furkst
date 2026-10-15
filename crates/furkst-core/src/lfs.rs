//! Git LFS: recognizing pointer files and finding the real content in the local LFS cache.

use crate::Repo;

/// The small text file git stores in place of a large file tracked by Git LFS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsPointer {
    /// SHA-256 of the real content, as 64 lowercase hex digits.
    pub oid: String,
    /// Size of the real content in bytes.
    pub size: u64,
}

impl LfsPointer {
    /// Parses a pointer file as defined by the Git LFS spec; `None` for anything else.
    pub fn parse(data: &[u8]) -> Option<Self> {
        // The spec caps pointer files below 1024 bytes, which keeps this cheap for every diffed file.
        if data.len() >= 1024 {
            return None;
        }
        let mut lines = std::str::from_utf8(data).ok()?.lines();
        if !lines.next()?.starts_with("version https://git-lfs.github.com/spec/") {
            return None;
        }
        let (mut oid, mut size) = (None, None);
        for line in lines {
            match line.split_once(' ') {
                Some(("oid", value)) => oid = value.strip_prefix("sha256:"),
                Some(("size", value)) => size = value.parse().ok(),
                _ => {}
            }
        }
        let oid = oid.filter(|oid| oid.len() == 64 && oid.bytes().all(|b| b.is_ascii_hexdigit()))?;
        Some(Self { oid: oid.to_ascii_lowercase(), size: size? })
    }
}

impl Repo {
    /// Whether the repository uses Git LFS (it has an LFS cache or tracks patterns with it).
    pub fn uses_lfs(&self) -> bool {
        self.gix().common_dir().join("lfs").is_dir() || !self.lfs_patterns().is_empty()
    }

    /// The patterns the root `.gitattributes` stores with Git LFS, e.g. `*.psd`.
    pub fn lfs_patterns(&self) -> Vec<String> {
        let gitattributes = std::fs::read_to_string(self.workdir().join(".gitattributes")).unwrap_or_default();
        lfs_patterns(&gitattributes)
    }

    /// Replaces LFS pointer `content` by the real content from the local cache. `Err` carries the pointer when
    /// the content hasn't been downloaded; anything that isn't a pointer passes through unchanged.
    pub(crate) fn resolve_lfs(&self, content: Option<Vec<u8>>) -> Result<Option<Vec<u8>>, LfsPointer> {
        match content.as_deref().and_then(LfsPointer::parse) {
            Some(pointer) => self.lfs_object(&pointer).map(Some).ok_or(pointer),
            None => Ok(content),
        }
    }

    /// The real content behind `pointer`, if it has been downloaded into the local LFS cache.
    fn lfs_object(&self, pointer: &LfsPointer) -> Option<Vec<u8>> {
        let oid = &pointer.oid;
        let path = self.gix().common_dir().join("lfs/objects").join(&oid[..2]).join(&oid[2..4]).join(oid);
        std::fs::read(path).ok()
    }
}

/// Patterns of `.gitattributes` lines that assign `filter=lfs`.
fn lfs_patterns(gitattributes: &str) -> Vec<String> {
    gitattributes
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pattern = fields.next()?;
            fields.any(|attr| attr == "filter=lfs").then(|| pattern.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";

    #[test]
    fn parses_pointers() {
        let text = format!("version https://git-lfs.github.com/spec/v1\noid sha256:{OID}\nsize 12345\n");
        assert_eq!(LfsPointer::parse(text.as_bytes()), Some(LfsPointer { oid: OID.into(), size: 12345 }));
    }

    #[test]
    fn reads_tracked_patterns() {
        let attributes = "*.psd filter=lfs diff=lfs merge=lfs -text
# *.zip filter=lfs
*.txt text
assets/** filter=lfs
";
        assert_eq!(lfs_patterns(attributes), ["*.psd", "assets/**"]);
    }

    #[test]
    fn rejects_everything_else() {
        assert_eq!(LfsPointer::parse(b"fn main() {}\n"), None);
        assert_eq!(LfsPointer::parse(b"version https://git-lfs.github.com/spec/v1\nsize 1\n"), None);
        let unsafe_oid = "version https://git-lfs.github.com/spec/v1\noid sha256:../../../etc/passwd\nsize 1\n";
        assert_eq!(LfsPointer::parse(unsafe_oid.as_bytes()), None);
        assert_eq!(LfsPointer::parse(&[b'x'; 2048]), None);
    }
}
