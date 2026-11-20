//! End-to-end: the whole move set through the real input path in the demo park.

use game_core::Game;
use game_core::autoplay::TrickTour;
use game_core::skater::{Event, Mode};

#[test]
fn trick_tour_does_every_move() {
    let mut game = Game::new();
    let mut script = TrickTour::new();
    let mut seen = Vec::new();
    let mut manual_frames = 0;
    let mut grab_frames = 0;
    let mut leaned = (false, false);
    for _ in 0..(60 * 20) {
        let pad = script.next(&game);
        game.update(&pad, 1.0 / 60.0);
        seen.extend(game.events.iter().cloned());
        let s = &game.skater;
        manual_frames += s.manual.is_some() as u32;
        grab_frames += s.grab.is_some() as u32;
        if s.mode == Mode::Rolling {
            leaned.0 |= s.steer < -0.5;
            leaned.1 |= s.steer > 0.5;
        }
        if script.done() {
            break;
        }
    }
    let names: Vec<&str> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Landed { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert!(script.done(), "{seen:?}");
    assert_eq!(names.first(), Some(&"KICKFLIP"), "{seen:?}");
    assert!(leaned.0 && leaned.1, "carved both ways");
    assert!(manual_frames > 90, "{manual_frames}");
    assert!(names.contains(&"MANUAL"), "{names:?}");
    assert!(!seen.contains(&Event::ManualLost), "{seen:?}");
    assert!(grab_frames > 10, "{grab_frames}");
    assert!(names.contains(&"OLLIE INDY"), "{names:?}");
    assert_eq!(seen.last(), Some(&Event::Bailed), "{seen:?}");
}
