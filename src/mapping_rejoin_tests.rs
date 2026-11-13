//! Going back into a mapping session left in the editor: the menus' bar and the join again.

use super::*;
use editor::view::collab::LeftSession;
use hit::Hit;

/// Song select, logged in to a server that runs mapping sessions (one nobody answers at: a test
/// never reaches a real server).
fn menus() -> State {
    let mut s = crate::layout_tests::state();
    // Stay in song select once the library is scanned, however the tests were started.
    s.play_when_scanned = false;
    s.mapping_sessions = true;
    s.cfg.online_token = "t".into();
    s.cfg.online_server = "127.0.0.1:9".into();
    s.scene = SongSelect::scene();
    s
}

fn frame(s: &mut State) -> Canvas {
    let mut c = Canvas {
        audit: true,
        ..Default::default()
    };
    s.build_frame(&mut c, Instant::now(), 853.0);
    c
}

#[test]
fn the_menus_offer_the_session_left_and_a_click_goes_back_into_it() {
    let mut s = menus();
    frame(&mut s);
    assert!(!s.hits.iter().any(|h| h.hit == Hit::BackToSession));
    let dir = std::env::temp_dir().join(format!("taiko-rs-rejoin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    s.left_mapping = Some(LeftSession {
        session: 9,
        title: "USAO - Dynamite".into(),
        dir,
    });
    let c = frame(&mut s);
    assert!(
        c.texts
            .iter()
            .any(|(_, t)| t.contains("You're in the mapping session USAO - Dynamite")),
        "the bar says so; drawn: {:?}, mapping_ok {}, select {}, splash {}, changelog {}, \
         player {}, left {}",
        c.texts.iter().map(|(_, t)| t).collect::<Vec<_>>(),
        s.mapping_ok(),
        s.scene.is_select(),
        s.splash.is_some(),
        s.changelog.is_some(),
        s.player.is_some(),
        s.left_mapping.is_some(),
    );
    let hits: Vec<Hit> = s.hits.iter().map(|h| h.hit).collect();
    assert!(hits.contains(&Hit::BackToSession) && hits.contains(&Hit::ForgetSession));
    s.click(Hit::BackToSession, None);
    assert!(s.mapping_join.is_some(), "joining again");
    assert!(s.left_mapping.is_none());
    // The server can't be reached: the session is offered again for later.
    let until = Instant::now() + std::time::Duration::from_secs(10);
    while s.mapping_join.is_some() && Instant::now() < until {
        s.poll_mapping();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(s.mapping_join.is_none());
    assert_eq!(s.left_mapping.as_ref().map(|l| l.session), Some(9));
    s.click(Hit::ForgetSession, None);
    assert!(s.left_mapping.is_none());
    frame(&mut s);
    assert!(!s.hits.iter().any(|h| h.hit == Hit::BackToSession));
}

#[test]
fn a_session_that_closes_or_another_one_takes_the_offer_away() {
    let mut s = menus();
    let left = || {
        Some(LeftSession {
            session: 9,
            title: "t".into(),
            dir: std::env::temp_dir(),
        })
    };
    s.left_mapping = left();
    s.mapping_closed(8);
    assert!(s.left_mapping.is_some(), "another session's end");
    s.mapping_closed(9);
    assert!(s.left_mapping.is_none());
    // Accepting an invite to another session.
    s.left_mapping = left();
    s.join_mapping(12);
    assert!(s.left_mapping.is_none());
    // Without a server that runs sessions, nothing is offered.
    let mut s = menus();
    s.left_mapping = left();
    s.mapping_sessions = false;
    frame(&mut s);
    assert!(!s.hits.iter().any(|h| h.hit == Hit::BackToSession));
}
