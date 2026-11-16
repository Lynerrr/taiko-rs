//! The client's regression suite: gameplay, scoring, file formats, the network protocol and
//! every screen, compared with golden files in `snapshots/` (see `docs/regression-tests.md`).
//!
//! Any change to the client must keep these passing; a change in behaviour that is meant must
//! come with its updated golden files (`TAIKO_BLESS=1`), and new behaviour with new tests here.

pub mod harness;
pub mod maps;
pub mod snapshot;

mod clicks;
mod collections;
mod custom_folder;
mod custom_maps;
mod drawing;
mod epic;
mod flows;
mod font_search;
mod formats;
mod gameplay;
mod glyphs;
mod map_delete;
mod network;
mod phone;
mod pointer;
mod scoring;
mod screens;
mod secrets;
mod select;
mod store;

mod language_catalog;
