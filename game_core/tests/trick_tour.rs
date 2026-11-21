//! End-to-end: the whole move set through the real input path in the demo park (flatground's own
//! riding rules: no imported data).

use game_core::Game;
use game_core::autoplay::{MANUAL_TICKS, TrickTour};
use game_core::skater::{Event, Mode};

#[test]
fn trick_tour_does_every_move() {
    let mut game = Game::new();
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
            game.place_rider(p.pos, p.heading, p.speed);
        }
        game.update(&pad, 1.0 / 60.0);
        seen.extend(game.events.iter().cloned());
        let s = &game.skater;
        manual_frames += s.manual.is_some() as u32;
        grab_frames += s.grab.is_some() as u32;
        slide_frames += s.slide.is_some() as u32;
        fakie_frames += (s.fakie && s.mode == Mode::Rolling) as u32;
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
    let has = |part: &str| names.iter().any(|n| n.contains(part));
    assert!(script.done(), "{names:?} {seen:?}");
    assert_eq!(names.first(), Some(&"KICKFLIP"), "{seen:?}");
    assert!(leaned.0 && leaned.1, "carved both ways");
    assert!(manual_frames >= MANUAL_TICKS, "{manual_frames}");
    assert!(script.seen.manual >= MANUAL_TICKS, "{:?}", script.seen);
    assert!(has("MANUAL"), "{names:?}");
    assert!(script.seen.kick_turn, "{:?}", script.seen);
    assert!(grab_frames > 10, "{grab_frames}");
    // The C-stick down picks the indy's variant (flatground's table: a stalefish).
    assert!(
        has("OLLIE STALEFISH") && script.seen.grab_variant,
        "{names:?}"
    );
    assert!(has("REVERT") && script.seen.revert, "{names:?}");
    assert!(has("360 FLIP") || has("LASER FLIP"), "{names:?}");
    // The 180's stick is let go early: the spin carries on and the landing assist finishes it.
    assert!(has("180 OLLIE"), "{names:?}");
    assert!(
        !names
            .iter()
            .any(|n| n.contains("SKETCHY") && n.contains("180")),
        "{names:?}"
    );
    assert!(fakie_frames > 10, "{fakie_frames}");
    assert!(seen.contains(&Event::Switched), "{names:?}");
    // The held kickflip (a kickflip or, ridden switch, the mirrored flick's heelflip).
    assert!(
        names
            .iter()
            .filter(|n| n.contains("KICKFLIP") || n.contains("HEELFLIP"))
            .count()
            >= 2,
        "{names:?}"
    );
    assert!(slide_frames > 10, "{slide_frames}");
    assert!(has("HIPPIE JUMP"), "{names:?}");
    // Grinds on the demo ledge: an ollie onto it into a 50-50 tipped into a 5-0 with the C-stick, and
    // an ollie along its line with a quarter turn into a slide (the demo park has no coping).
    assert!(
        script.seen.ledge && script.seen.five_o && script.seen.slide,
        "{:?} {names:?}",
        script.seen
    );
    assert!(has(" 5-0"), "{names:?}");
    assert_eq!(seen.last(), Some(&Event::Bailed), "{seen:?}");
}
