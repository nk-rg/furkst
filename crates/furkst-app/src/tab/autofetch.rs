//! Scheduling for the periodic background fetch of a repository tab.

use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct AutoFetch {
    /// When the interval last restarted: the tab opened or an auto-fetch was started.
    timer: Instant,
    last_success: Option<Instant>,
}

/// What the tab should do right now.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Fetch,
    /// Check again after this long (`None`: auto-fetch is off or not applicable).
    Wait(Option<Duration>),
}

impl AutoFetch {
    pub fn new(now: Instant) -> Self {
        Self { timer: now, last_success: None }
    }

    /// Decides whether to fetch now. A fetch is only due once the interval has elapsed, and only
    /// starts when the tab is idle and has remotes; starting one restarts the interval.
    pub fn tick(&mut self, now: Instant, minutes: u32, busy: bool, has_remotes: bool) -> Decision {
        if minutes == 0 || !has_remotes {
            return Decision::Wait(None);
        }
        let interval = Duration::from_secs(u64::from(minutes) * 60);
        let remaining = interval.saturating_sub(now.saturating_duration_since(self.timer));
        if !remaining.is_zero() {
            return Decision::Wait(Some(remaining));
        }
        if busy {
            // Retried as soon as the running operation finishes and the tab repaints.
            return Decision::Wait(None);
        }
        self.timer = now;
        Decision::Fetch
    }

    pub fn succeeded(&mut self, now: Instant) {
        self.last_success = Some(now);
    }

    /// "5 min ago" style text for the last successful fetch.
    pub fn last_success_text(&self, now: Instant) -> Option<String> {
        let minutes = now.saturating_duration_since(self.last_success?).as_secs() / 60;
        Some(match minutes {
            0 => "just now".to_owned(),
            1..=59 => format!("{minutes} min ago"),
            _ => format!("{} h ago", minutes / 60),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_the_interval_then_fetches_once() {
        let start = Instant::now();
        let mut fetch = AutoFetch::new(start);
        assert_eq!(fetch.tick(start, 5, false, true), Decision::Wait(Some(Duration::from_secs(300))));
        let due = start + Duration::from_secs(300);
        assert_eq!(fetch.tick(due, 5, false, true), Decision::Fetch);
        assert_eq!(fetch.tick(due, 5, false, true), Decision::Wait(Some(Duration::from_secs(300))));
    }

    #[test]
    fn never_fetches_when_off_busy_or_without_remotes() {
        let start = Instant::now();
        let mut fetch = AutoFetch::new(start);
        let late = start + Duration::from_secs(3600);
        assert_eq!(fetch.tick(late, 0, false, true), Decision::Wait(None));
        assert_eq!(fetch.tick(late, 5, false, false), Decision::Wait(None));
        assert_eq!(fetch.tick(late, 5, true, true), Decision::Wait(None));
        // Still due once the tab is idle.
        assert_eq!(fetch.tick(late, 5, false, true), Decision::Fetch);
    }

    #[test]
    fn describes_the_last_success() {
        let start = Instant::now();
        let mut fetch = AutoFetch::new(start);
        assert_eq!(fetch.last_success_text(start), None);
        fetch.succeeded(start);
        assert_eq!(fetch.last_success_text(start).as_deref(), Some("just now"));
        assert_eq!(fetch.last_success_text(start + Duration::from_secs(150)).as_deref(), Some("2 min ago"));
        assert_eq!(fetch.last_success_text(start + Duration::from_secs(7300)).as_deref(), Some("2 h ago"));
    }
}
