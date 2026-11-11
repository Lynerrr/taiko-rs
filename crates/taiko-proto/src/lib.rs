//! What the game (`taiko-rs`) and its server (`taiko-server`) both speak: the multiplayer
//! protocol (`net`), the online server's API (`online`), TLS (`tls`), the interface's languages
//! and their `tr!` (`lang`), and the changelog's entries (`changelog_entry`).
//!
//! The game and the server re-export each module under its old name, so `crate::net` and the
//! rest still mean the same thing there.

#[macro_use]
pub mod lang;
pub mod changelog_entry;
pub mod net;
pub mod online;
pub mod tls;

pub use taiko_logic::font;
