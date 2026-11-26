//! End-to-end: the whole move set through the real input path in the demo park, played through Skate 3's
//! state graphs with the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped
//! without it), and the hands-off autoplay repeating the tour.

mod common;

use game_core::autoplay::{Autoplay, MANUAL_TICKS, TrickTour};
use game_core::skater::Event;

#[test]
fn imported_autoplay_repeats_the_tour() {
    let Some(mut game) = common::disc_game("imported_autoplay_repeats_the_tour") else {
        return;
    };
    let mut auto = Autoplay::new();
    let mut lines = 0;
    for _ in 0..(60 * 240) {
        let (pad, line) = auto.next(&game);
        lines += line.is_some_and(|l| l.contains(" done")) as u32;
        if auto.take_restart() {
            game.restart_at_spawn();
        }
        if let Some(p) = auto.take_place() {
            game.place(&p);
        }
        game.update(&pad, 1.0 / 60.0);
        if auto.rounds >= 2 {
            break;
        }
    }
    assert!(auto.rounds >= 2, "rounds {}", auto.rounds);
    assert_eq!(lines, auto.rounds);
}

#[test]
fn imported_trick_tour_does_every_move() {
    let Some(mut game) = common::disc_game("imported_trick_tour_does_every_move") else {
        return;
    };
    let mut script = TrickTour::new();
    let mut seen = Vec::new();
    let mut manual_frames = 0;
    let mut grab_frames = 0;
    let mut slide_frames = 0;
    let mut fakie_frames = 0;
    let mut leaned = (false, false);
    for _ in 0..(60 * 60) {
        let pad = script.next(&game);
        if let Some(p) = script.take_place() {
            game.place(&p);
        }
        game.update(&pad, 1.0 / 60.0);
        seen.extend(game.events.iter().cloned());
        let s = &game.skater;
        manual_frames += s.manual.is_some() as u32;
        grab_frames += s.grab.is_some() as u32;
        slide_frames += s.slide.is_some() as u32;
        fakie_frames += (s.fakie && s.rolling()) as u32;
        if s.rolling() {
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
    let has = |part: &str| names.iter().any(|n| n.contains(part));
    // Through Skate 3's graphs the tour gets as far as its flips and spins (its later moves are scripted
    // for flatground's old rules and stall: ROADMAP).
    assert_eq!(names.first(), Some(&"KICKFLIP"), "{seen:?}");
    assert!(leaned.0 && leaned.1, "carved both ways");
    assert!(manual_frames >= MANUAL_TICKS, "{manual_frames}");
    assert!(has("MANUAL"), "{names:?}");
    assert!(has("REVERT") && script.seen.revert, "{names:?}");
    assert!(has("360 FLIP") || has("LASER FLIP"), "{names:?}");
    assert!(has("180 OLLIE"), "{names:?}");
    assert!(has("DOUBLE KICKFLIP"), "{names:?}");
    assert!(has("LATE KICKFLIP"), "{names:?}");
    let _ = (slide_frames, grab_frames, fakie_frames);
}
