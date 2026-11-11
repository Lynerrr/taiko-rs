//! `taiko_logic::od_override`, and the OD the player picked for OD Override: the crate keeps
//! no process-wide settings, so it is kept here (from the config).

use std::sync::atomic::{AtomicU64, Ordering};

pub use taiko_logic::od_override::*;

/// The OD played with, as bits (`f64::to_bits`). Set from the config, read when a play's built.
static CURRENT: AtomicU64 = AtomicU64::new(DEFAULT.to_bits());

pub fn set(od: f64) {
    CURRENT.store(clamped(od).to_bits(), Ordering::Relaxed);
}

pub fn get() -> f64 {
    f64::from_bits(CURRENT.load(Ordering::Relaxed))
}
