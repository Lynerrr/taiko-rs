//! pp (taiko-logic) against the online server's rules the client plays by.

use crate::mods::Mods;
use crate::pp::{MAX_CUSTOM_FACTOR, custom_factor};

#[test]
fn the_servers_list_of_mods_without_a_factor_is_right() {
    for code in crate::online::NO_FACTOR_MODS {
        assert_eq!(custom_factor(Mods::parse(code)), 1.0, "{code}");
    }
    let most = crate::mods::ALL
        .iter()
        .filter(|m| !crate::online::NO_FACTOR_MODS.contains(&m.acronym()))
        .map(|m| m.multiplier())
        .fold(0.0, f64::max);
    assert!(most.powi(crate::online::MODS_BELOW_CAP as i32) < MAX_CUSTOM_FACTOR);
}
