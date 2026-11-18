//! What Coquou itself is busy with, shown by the pet (the Codex rows
//! `running`, `review` and `failed`; see doc/pet-format.md §4).
//!
//! Work holds a [`Busy`] guard for as long as it runs. The guards are
//! counted, so overlapping work (several hatch rows at once) reads as one
//! stretch, and a guard is released on every exit path, errors included,
//! so the pet can never be left stuck "working". Each change sends the
//! whole state to the overlay as `pet-activity`; a failure is a one-off
//! `pet-react` ("failed").

use crate::sync::lock;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};

#[derive(Clone, Copy)]
pub enum Kind {
    /// A request to the AI service (hatching).
    Running,
    /// Checking and installing a pet package.
    Review,
}

#[derive(Default, Clone, Copy)]
struct Counts {
    running: u32,
    review: u32,
}

impl Counts {
    fn slot(&mut self, kind: Kind) -> &mut u32 {
        match kind {
            Kind::Running => &mut self.running,
            Kind::Review => &mut self.review,
        }
    }
}

#[derive(Default)]
pub struct Activity(Mutex<Counts>);

/// Marks work of one kind as running until dropped.
pub struct Busy<R: Runtime> {
    app: AppHandle<R>,
    kind: Kind,
}

pub fn begin<R: Runtime>(app: &AppHandle<R>, kind: Kind) -> Busy<R> {
    change(app, |c| *c.slot(kind) += 1);
    Busy { app: app.clone(), kind }
}

impl<R: Runtime> Drop for Busy<R> {
    fn drop(&mut self) {
        let kind = self.kind;
        change(&self.app, |c| *c.slot(kind) = c.slot(kind).saturating_sub(1));
    }
}

/// Something the user asked for went wrong: the pet looks sad for a moment.
pub fn failed<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit_to(crate::overlay::LABEL, "pet-react", "failed");
}

fn change<R: Runtime>(app: &AppHandle<R>, f: impl FnOnce(&mut Counts)) {
    let Some(state) = app.try_state::<Activity>() else { return };
    // Copy out, then emit without holding the lock.
    let now = {
        let mut counts = lock(&state.0);
        f(&mut counts);
        *counts
    };
    let _ = app.emit_to(crate::overlay::LABEL, "pet-activity", serde_json::json!({
        "running": now.running > 0,
        "review": now.review > 0,
    }));
}

