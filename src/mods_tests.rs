//! The mods (taiko-logic) against what the client makes of them: the pp version it plays for and
//! the mod select's width.

use crate::mods::{ALL, HalfRule};

#[test]
fn the_pp_version_played_for_halves_on_the_beat() {
    assert_eq!(
        HalfRule::of_pp_version(crate::online::PP_VERSION),
        HalfRule::OnTheBeat
    );
}

#[test]
fn every_mod_s_description_fits_the_mod_select() {
    for m in ALL {
        // The mod select's description line at 5:4 (600 wide).
        assert!(
            crate::hit::Canvas::text_width(7.0, m.description()) <= 600.0 - 64.0,
            "{}",
            m.name()
        );
    }
}
