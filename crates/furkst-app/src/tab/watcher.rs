//! Refreshing a tab when its repository changes outside Furkst (an editor, a terminal, another git client).

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use furkst_core::{Change, Repo, Watch};

/// Changes arriving within this window are coalesced into one refresh, so a checkout touching many files
/// or a build writing several files reloads once.
const QUIET: Duration = Duration::from_millis(400);

pub struct Watcher {
    /// `None` if the platform refused to watch (e.g. too many watched folders); focus-refresh still works.
    _watch: Option<Watch>,
    changes: Receiver<Change>,
    debounce: Debounce,
}

impl Watcher {
    pub fn new(ctx: &egui::Context, repo: &Repo) -> Self {
        let (tx, changes) = mpsc::channel();
        let ctx = ctx.clone();
        let watch = repo
            .watch(move |change| {
                let _ = tx.send(change);
                ctx.request_repaint();
            })
            .ok();
        Self { _watch: watch, changes, debounce: Debounce::default() }
    }

    /// The refresh to perform now, if one is due. `busy` holds it back while the tab's own work runs.
    pub fn due(&mut self, ctx: &egui::Context, busy: bool) -> Option<Change> {
        let now = Instant::now();
        for change in self.changes.try_iter() {
            self.debounce.push(change, now);
        }
        match self.debounce.due(now, busy) {
            Ok(change) => change,
            Err(wait) => {
                ctx.request_repaint_after(wait);
                None
            }
        }
    }

    /// Forgets pending changes, e.g. because the tab is reloading everything anyway.
    pub fn discard(&mut self) {
        self.debounce = Debounce::default();
    }
}

#[derive(Default)]
struct Debounce {
    pending: Option<(Change, Instant)>,
}

impl Debounce {
    fn push(&mut self, change: Change, now: Instant) {
        let strongest = self.pending.map_or(change, |(pending, _)| pending.max(change));
        self.pending = Some((strongest, now));
    }

    /// `Ok(Some(_))` when a quiet period has passed, `Err(wait)` while it hasn't yet.
    fn due(&mut self, now: Instant, busy: bool) -> Result<Option<Change>, Duration> {
        let Some((change, last)) = self.pending else { return Ok(None) };
        let quiet_for = now.saturating_duration_since(last);
        if quiet_for < QUIET {
            return Err(QUIET - quiet_for);
        }
        if busy {
            // Re-checked when the running work finishes and the tab repaints.
            return Ok(None);
        }
        self.pending = None;
        Ok(Some(change))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_bursts_and_keeps_the_strongest_change() {
        let start = Instant::now();
        let mut d = Debounce::default();
        d.push(Change::WorkingCopy, start);
        d.push(Change::Refs, start + Duration::from_millis(100));
        d.push(Change::WorkingCopy, start + Duration::from_millis(200));
        assert!(d.due(start + Duration::from_millis(300), false).is_err());
        assert_eq!(d.due(start + Duration::from_millis(600), false), Ok(Some(Change::Refs)));
        assert_eq!(d.due(start + Duration::from_millis(700), false), Ok(None));
    }

    #[test]
    fn waits_while_busy() {
        let start = Instant::now();
        let mut d = Debounce::default();
        d.push(Change::WorkingCopy, start);
        let later = start + QUIET;
        assert_eq!(d.due(later, true), Ok(None));
        assert_eq!(d.due(later, false), Ok(Some(Change::WorkingCopy)));
    }
}
