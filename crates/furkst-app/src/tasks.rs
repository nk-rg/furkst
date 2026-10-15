//! Runs blocking git work off the UI thread and hands results back on the next frame.

use crossbeam_channel::{Receiver, Sender, unbounded};

pub struct Tasks<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
    ctx: egui::Context,
    running: usize,
}

impl<T: Send + 'static> Tasks<T> {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = unbounded();
        Self { tx, rx, ctx: ctx.clone(), running: 0 }
    }

    pub fn spawn(&mut self, job: impl FnOnce() -> T + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.running += 1;
        std::thread::spawn(move || {
            // The receiver only disappears when the tab is closed; the result is moot then.
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }

    /// Results that arrived since the last call.
    pub fn poll(&mut self) -> Vec<T> {
        let done: Vec<T> = self.rx.try_iter().collect();
        self.running -= done.len();
        done
    }

    pub fn is_busy(&self) -> bool {
        self.running > 0
    }
}

/// The latest progress line of running work, written by its background thread and shown by the UI.
#[derive(Clone)]
pub struct Progress {
    line: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    ctx: egui::Context,
}

impl Progress {
    pub fn new(ctx: &egui::Context) -> Self {
        Self { line: Default::default(), ctx: ctx.clone() }
    }

    pub fn report(&self, line: &str) {
        *self.slot() = Some(line.to_owned());
        self.ctx.request_repaint();
    }

    pub fn current(&self) -> Option<String> {
        self.slot().clone()
    }

    pub fn clear(&self) {
        *self.slot() = None;
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.line.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
