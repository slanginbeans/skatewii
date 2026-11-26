//! End-to-end: the real input path (raw pad -> Skate 3's state graphs -> physics) through the demo park,
//! with the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it).

mod common;

use game_core::autoplay::KickerRun;
use game_core::skater::Event;

#[test]
fn kickflip_off_the_kicker_lands() {
    let Some(mut game) = common::disc_game("kickflip_off_the_kicker_lands") else {
        return;
    };
    let mut script = KickerRun::new();
    let mut seen = Vec::new();
    for _ in 0..(60 * 8) {
        let pad = script.next(&game);
        game.update(&pad, 1.0 / 60.0);
        seen.extend(game.events.iter().cloned());
    }
    let landed = seen.iter().find_map(|e| match e {
        Event::Landed { name, points, .. } => Some((name.clone(), *points)),
        _ => None,
    });
    let (name, points) = landed.unwrap_or_else(|| panic!("no landing: {seen:?}"));
    assert_eq!(name, "KICKFLIP", "{seen:?}");
    // Scored with the disc's trick catalogue.
    assert!(points > 0, "{points}");
    assert!(!seen.contains(&Event::Bailed), "{seen:?}");
}
