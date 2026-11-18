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
mod controls;
mod custom_folder;
mod custom_install;
mod custom_maps;
mod custom_migration;
mod custom_routing;
mod drawing;
mod epic;
mod flows;
mod font_search;
mod formats;
mod gameplay;
mod glyphs;
mod lens;
mod map_delete;
mod menu_shift;
mod music_player;
mod network;
mod phone;
mod pointer;
mod preview_reach;
mod scoring;
mod screens;
mod secrets;
mod select;
mod song_menu;
mod store;
mod text_fields;
mod tja_library;

mod language_catalog;
