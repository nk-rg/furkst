//! Author pictures from GitHub, for authors who can be told apart as GitHub users: by their GitHub no-reply address,
//! or because GitHub itself made the commit (web edits, merged pull requests). Local commits are never looked up.
//! Off unless turned on in Preferences. Pictures are cached on disk, and so is the answer that there is none, so
//! each one is asked for once.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Sender, unbounded};
use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use furkst_core::Signature;

/// GitHub answers addresses it doesn't know with this placeholder, whose ETag is its SHA-256; we show initials instead.
const PLACEHOLDER_ETAG: &str = "\"1506265dd79492985d428bf3ac115e873c30eedde97749ace57dff5a0020a0a2\"";
/// Pixels asked for: enough for the largest avatar (36 points) on a high-DPI screen.
const SIZE: u32 = 80;
/// How long "no picture" is trusted before asking again, in case the person adds one.
const MISSING_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_BYTES: u64 = 1 << 20;
const NO_REPLY_DOMAIN: &str = "@users.noreply.github.com";
/// The committer of commits GitHub makes on its users' behalf.
const GITHUB_COMMITTER: &str = "noreply@github.com";

/// How a GitHub user's picture is found.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Lookup {
    /// `12345+name@users.noreply.github.com`: the account's id is in the address.
    Id(u64),
    /// `name@users.noreply.github.com`, which older accounts have: the user name is.
    User(String),
    /// The author of a commit GitHub made, by the address GitHub knows them by.
    Email(String),
}

impl Lookup {
    /// How to find the author's picture, or `None` for authors not known to be GitHub users (local commits).
    fn of(author: &Signature, committer: &Signature) -> Option<Self> {
        let email = author.email.trim().to_lowercase();
        if let Some(local) = email.strip_suffix(NO_REPLY_DOMAIN) {
            if let Some(id) = local.split_once('+').and_then(|(id, _)| id.parse().ok()) {
                return Some(Self::Id(id));
            }
            let valid_user = !local.is_empty() && local.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
            return valid_user.then(|| Self::User(local.to_owned()));
        }
        committer.email.trim().eq_ignore_ascii_case(GITHUB_COMMITTER).then_some(Self::Email(email))
    }

    fn url(&self) -> String {
        match self {
            Self::Id(id) => format!("https://avatars.githubusercontent.com/u/{id}?s={SIZE}"),
            Self::User(name) => format!("https://github.com/{name}.png?size={SIZE}"),
            Self::Email(email) => format!("https://avatars.githubusercontent.com/u/e?email={}&s={SIZE}", encode(email)),
        }
    }
}

enum Entry {
    Pending,
    Missing,
    Ready(TextureHandle),
}

struct Store {
    enabled: AtomicBool,
    entries: Mutex<HashMap<Lookup, Entry>>,
    /// Pictures to fetch, one at a time by a single background thread.
    queue: Sender<(Lookup, Context)>,
}

fn store() -> &'static Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    STORE.get_or_init(|| {
        let (queue, requests) = unbounded::<(Lookup, Context)>();
        std::thread::spawn(move || {
            let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().new_agent();
            for (lookup, ctx) in requests {
                let entry = match load(&agent, &lookup) {
                    Some(pixels) => Entry::Ready(ctx.load_texture(format!("avatar:{lookup:?}"), pixels, TextureOptions::LINEAR)),
                    None => Entry::Missing,
                };
                store().entries.lock().unwrap_or_else(PoisonError::into_inner).insert(lookup, entry);
                ctx.request_repaint();
            }
        });
        Store { enabled: AtomicBool::new(false), entries: Mutex::default(), queue }
    })
}

/// Follows the Preferences setting; turning it off shows initials everywhere again at once.
pub fn set_enabled(enabled: bool) {
    store().enabled.store(enabled, Ordering::Relaxed);
}

/// The author's GitHub picture, if avatars are on, the author is a known GitHub user and the picture has arrived;
/// otherwise `None`, after asking for it when it can be found.
pub fn get(ctx: &Context, author: &Signature, committer: &Signature) -> Option<TextureHandle> {
    let store = store();
    if !store.enabled.load(Ordering::Relaxed) {
        return None;
    }
    let lookup = Lookup::of(author, committer)?;
    let mut entries = store.entries.lock().unwrap_or_else(PoisonError::into_inner);
    match entries.get(&lookup) {
        Some(Entry::Ready(texture)) => Some(texture.clone()),
        Some(Entry::Pending | Entry::Missing) => None,
        None => {
            entries.insert(lookup.clone(), Entry::Pending);
            let _ = store.queue.send((lookup, ctx.clone()));
            None
        }
    }
}

/// The picture from the disk cache, or else from GitHub (caching the answer either way); `None` if there is none or
/// GitHub can't be reached (then it is asked again next time Furkst starts).
fn load(agent: &ureq::Agent, lookup: &Lookup) -> Option<ColorImage> {
    let cached = cache_file(lookup);
    let missing = cached.as_ref().map(|file| file.with_extension("none"));
    let recent = |path: &PathBuf| {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        modified.and_then(|time| SystemTime::now().duration_since(time).ok()).is_some_and(|age| age < MISSING_FOR)
    };
    if missing.as_ref().is_some_and(recent) {
        return None;
    }
    if let Some(bytes) = cached.as_ref().and_then(|file| std::fs::read(file).ok()) {
        return crate::tab::image_view::decode(&bytes, SIZE).map(|(pixels, _)| pixels);
    }
    let remember_missing = || {
        if let Some(missing) = &missing {
            let _ = std::fs::write(missing, b"");
        }
    };
    let mut response = match agent.get(&lookup.url()).call() {
        Ok(response) => response,
        // No such user name; anything else (offline, GitHub down) is retried next time.
        Err(ureq::Error::StatusCode(404)) => {
            remember_missing();
            return None;
        }
        Err(_) => return None,
    };
    let etag = response.headers().get("etag").and_then(|value| value.to_str().ok()).unwrap_or_default();
    if etag == PLACEHOLDER_ETAG {
        remember_missing();
        return None;
    }
    let bytes = response.body_mut().with_config().limit(MAX_BYTES).read_to_vec().ok()?;
    let (pixels, _) = crate::tab::image_view::decode(&bytes, SIZE)?;
    if let Some(cached) = cached {
        let _ = std::fs::write(cached, &bytes);
    }
    Some(pixels)
}

/// Where a picture is cached, named by a hash so the folder doesn't list anyone's address.
fn cache_file(lookup: &Lookup) -> Option<PathBuf> {
    let dir = crate::settings::project_dirs()?.cache_dir().join("avatars");
    std::fs::create_dir_all(&dir).ok()?;
    let mut hasher = DefaultHasher::new();
    lookup.hash(&mut hasher);
    Some(dir.join(format!("{:016x}.img", hasher.finish())))
}

/// Percent-encodes everything but unreserved URL characters.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(email: &str) -> Signature {
        Signature { email: email.into(), ..Default::default() }
    }

    #[test]
    fn looks_up_only_github_users() {
        let local = sig("ada@example.com");
        let github = sig("noreply@github.com");
        assert_eq!(Lookup::of(&sig("583231+Octocat@users.noreply.github.com"), &local), Some(Lookup::Id(583231)));
        assert_eq!(Lookup::of(&sig("octocat@users.noreply.github.com"), &local), Some(Lookup::User("octocat".into())));
        assert_eq!(Lookup::of(&sig("Ada@Example.com"), &github), Some(Lookup::Email("ada@example.com".into())));
        assert_eq!(Lookup::of(&sig("ada@example.com"), &local), None, "a local commit");
        assert_eq!(Lookup::of(&sig("not a user@users.noreply.github.com"), &local), None);
    }

    #[test]
    fn encodes_addresses_for_urls() {
        assert_eq!(encode("ada+git@example.com"), "ada%2Bgit%40example.com");
        assert_eq!(encode("jos\u{e9}@example.com"), "jos%C3%A9%40example.com");
    }
}
