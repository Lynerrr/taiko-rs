//! `taiko_logic::tact`, and the step the player plays Tact with: the crate keeps no
//! process-wide settings, so it is kept here (from the config).

use std::sync::atomic::{AtomicUsize, Ordering};

pub use taiko_logic::tact::*;

static CURRENT: AtomicUsize = AtomicUsize::new(DEFAULT);

/// The step Tact plays with (from the config).
pub fn set(i: usize) {
    CURRENT.store(i.min(STEPS.len() - 1), Ordering::Relaxed);
}

pub fn get() -> Step {
    step(CURRENT.load(Ordering::Relaxed))
}
