//! tko: the gimmick charts that move the lane and the notes. The model (`gimmick`, its keyframe
//! `track`s, `graph`s, `path`s and `clips`), its file `format`, and the `runtime` that plays it
//! and `scroll`s the notes. The client draws it (`tko::screen`, `tko::recipes`).

pub mod clips;
pub mod format;
pub mod gimmick;
pub mod graph;
pub mod path;
pub mod runtime;
pub mod scroll;
pub mod track;

/// The newest format this build reads and writes. Raising it needs a migration in
/// `format::MIGRATIONS` and a fixture of the old format in its tests.
pub const TKO_VERSION: u32 = 4;
