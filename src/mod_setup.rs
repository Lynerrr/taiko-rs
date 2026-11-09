//! The setup panels of the mods that have settings (opened from the mod select). The settings
//! themselves, and what the mods do with them, live in the mods' own modules.

pub mod multiverse;
pub mod od_override;
pub mod shapeshift;
pub mod tact;

#[cfg(test)]
mod tests {
    #[test]
    fn the_mods_own_modules_draw_nothing() {
        // The settings and what the mods do with them stay free of the drawing layers, so they can
        // move to taiko-logic; their panels are drawn here.
        let logic = [
            ("multiverse", include_str!("multiverse.rs")),
            ("od_override", include_str!("od_override.rs")),
            ("shapeshift", include_str!("shapeshift.rs")),
            ("tact", include_str!("tact.rs")),
            ("stars", include_str!("stars.rs")),
        ];
        for (name, src) in logic {
            for layer in ["hit", "render", "mobile", "social", "style", "mod_setup"] {
                assert!(
                    !src.contains(&format!("crate::{layer}::")),
                    "{name}.rs names crate::{layer}"
                );
            }
            assert!(!src.contains("Canvas"), "{name}.rs draws");
        }
    }
}
