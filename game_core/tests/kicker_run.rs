//! End-to-end: real input path (raw pad -> flick reader -> physics) through the demo park.

use game_core::Game;
use game_core::autoplay::KickerRun;
use game_core::skater::Event;

#[test]
fn kickflip_off_the_kicker_lands() {
    let mut game = Game::new();
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
    // Skate 3's kickflip: 100 points, x1.1 landed clean, and the air's own reward.
    assert!(points >= 110, "{points}");
    assert_eq!(game.score, points, "the line banked");
    assert!(!seen.contains(&Event::Bailed), "{seen:?}");
}
