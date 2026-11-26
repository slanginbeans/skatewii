//! The camera after a hard carve (flatground's own shots and shot choice, the demo park): it is back
//! behind the rider soon after the carve and never more than about 45 degrees to the side at riding
//! speed, through the straight, the wind-up and the ollie.

use game_core::Game;
use game_core::autoplay::CarveOllie;
use game_core::math::{atan2f, wrap_angle};

/// Degrees the camera is off straight behind the travel (signed).
fn off_behind(game: &Game) -> f32 {
    let s = &game.skater;
    let to = (s.pos - game.camera().camera().eye).horizontal();
    wrap_angle(atan2f(to.x, to.z) - s.travel_yaw()).to_degrees()
}

fn run(straight: u32) {
    let mut game = Game::new();
    let (p, h) = CarveOllie::START;
    game.place_rider(p, h, 0.0);
    let mut script = CarveOllie::new(straight);
    let mut most: f32 = 0.0;
    let mut behind_at = None;
    let mut popped = false;
    for _ in 0..(60 * 9) {
        let pad = script.next(&game);
        game.update(&pad, 1.0 / 60.0);
        let off = off_behind(&game);
        let phase = script.phase();
        if game.skater.speed.abs() > 2.0 && phase != "push" {
            most = most.max(off.abs());
        }
        if script.carved_at > 0 && behind_at.is_none() && off.abs() < 15.0 {
            behind_at = Some(script.t - script.carved_at);
        }
        popped |= game.skater.is_airborne() && script.flick_at > 0;
    }
    eprintln!("straight {straight}: most {most:.1}, behind after {behind_at:?} ticks");
    assert!(
        script.flick_at > 0 && popped,
        "no ollie: {}",
        script.phase()
    );
    assert!(most < 45.0, "the camera went {most} degrees to the side");
    let behind = behind_at.expect("never behind after the carve");
    // Skate 3's turning shot hands back to the chase shot over a second; the arm's tracker closes the
    // rest.
    assert!(behind < 90, "{behind} ticks to get behind");
}

#[test]
fn the_camera_is_behind_soon_after_a_carve_and_through_an_ollie() {
    run(30);
    run(90);
}
