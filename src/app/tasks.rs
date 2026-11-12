//! Background work. Each task runs on its own thread and hands a callback
//! back to the UI thread, which applies it to the app state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;

use super::App;

pub type Callback = Box<dyn FnOnce(&mut App) + Send>;

pub struct Tasks {
    tx: Sender<Callback>,
    rx: Receiver<Callback>,
    ctx: egui::Context,
}

impl Tasks {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        Tasks { tx, rx, ctx }
    }

    pub fn spawn<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut App, T) + Send + 'static,
    ) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = work();
            let _ = tx.send(Box::new(move |app: &mut App| done(app, result)));
            ctx.request_repaint();
        });
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    pub fn finished(&self) -> Vec<Callback> {
        self.rx.try_iter().collect()
    }
}

/// Request kinds; a newer request of the same kind makes older answers stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    Buckets,
    List,
    Preview,
    Download,
    Upload,
    SaveText,
    Check,
}

/// Bytes (or files) done and planned for the running transfer, shared with the worker.
#[derive(Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    pub fn add(&self, amount: u64) {
        self.done.fetch_add(amount, Ordering::Relaxed);
    }

    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
}
