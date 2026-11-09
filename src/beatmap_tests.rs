//! Maps (taiko-logic) as the client plays them.

use std::sync::Arc;

#[test]
fn a_slider_trillions_of_pixels_long_makes_a_game_to_preview() {
    // Like osu!team's FM Synthesis Experiment (set 2608578): NaN slider velocities and sliders
    // 3.6e12 px long, which crashed the game on picking the map (the rest of the check:
    // beatmap::tests::a_slider_trillions_of_pixels_long_is_a_roll_that_can_be_played).
    let text = "osu file format v14\n\n[General]\nAudioFilename: audio.mp3\nMode: 0\n\n[Difficulty]\nOverallDifficulty:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n0,NaN,4,1,0,100,0,0\n2000,-0.0001,4,1,0,100,0,0\n\n[HitObjects]\n256,192,1000,2,0,L|31457196:20,1,3617087291392\n256,192,3000,2,0,L|31457196:20,1,3617087291392\n256,192,5000,12,0,1e300,0:0:0:0:\n";
    let _ = crate::game::Game::new(
        crate::beatmap::parse_raw(text),
        Arc::new(crate::audio::Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
}
