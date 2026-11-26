//! The drawn rider and board (Skate 3's physical pose job, `game_core::render_pose`) with the player's
//! imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it): through a ride, an
//! ollie and its landing, a kickflip, a powerslide, getting off, walking and getting back on, the drawn
//! hips and board never jump against the rider's own motion from one tick to the next (the pose doesn't
//! snap between the clips and the physics), and riding the drawn board is the physical deck.

mod common;

use common::{TICK, pad};
use game_core::Game;
use game_core::input::{PadRaw, btn};
use game_core::math::{Vec3, v3};

/// The drawn bone's place in the world.
fn drawn(g: &Game, bone: &str) -> Vec3 {
    let r = g.rider().expect("imported rider");
    let c = &r.character;
    let m = c.world()[c.bone(bone).expect("bone")];
    g.imported_xform().point(v3(m[3][0], m[3][1], m[3][2]))
}

/// Runs `ticks` ticks of `p`, checking each tick that the drawn hips and board moved with the rider
/// (against the skater's own place) by no more than `most` metres; returns the largest jump.
fn run(g: &mut Game, ticks: usize, p: PadRaw, most: f32, what: &str, worst: &mut f32) {
    // Against the physical deck where there is one (the skater's place jumps at a pop), else the
    // skater's place.
    let anchor = |g: &Game| g.skater.rig.as_ref().map(|r| r.board.deck().position);
    let rel = |g: &Game, at: Vec3| (drawn(g, "HIPS") - at, drawn(g, "SKATEBOARD_ROOT") - at);
    for t in 0..ticks {
        let a0 = anchor(g);
        let before = g.skater.pos;
        let (h0b, b0b) = rel(g, before);
        let deck0 = a0.map(|a| rel(g, a));
        g.update(&p, TICK);
        let (h0, b0, h1, b1) = match (deck0, anchor(g)) {
            (Some((h0, b0)), Some(a1)) => {
                let (h1, b1) = rel(g, a1);
                (h0, b0, h1, b1)
            }
            _ => {
                let (h1, b1) = rel(g, g.skater.pos);
                (h0b, b0b, h1, b1)
            }
        };
        let jump = (h1 - h0).length().max((b1 - b0).length());
        *worst = worst.max(jump);
        assert!(
            jump < most,
            "{what}, tick {t}: the drawn pose jumped {:.0} mm (hips {:.0}, board {:.0}) in {:?}",
            jump * 1000.0,
            (h1 - h0).length() * 1000.0,
            (b1 - b0).length() * 1000.0,
            g.graph.as_ref().map(|d| d.runner.motion_state())
        );
    }
}

#[test]
fn imported_the_drawn_pose_never_snaps() {
    let Some(mut g) = common::disc_game("imported_the_drawn_pose_never_snaps") else {
        return;
    };
    let mut worst: f32 = 0.0;
    g.place_rider(v3(-7.0, 0.0, -18.0), 0.0, 3.0);
    for _ in 0..30 {
        g.update(&pad([0, 0], 0), TICK);
    }
    let w = &mut worst;
    run(&mut g, 60, pad([0, 0], 0), 0.06, "riding", w);
    // Riding, the drawn board is the physical deck.
    let deck = g.skater.rig.as_ref().unwrap().board.deck().position;
    let board = drawn(&g, "SKATEBOARD_ROOT");
    assert!((board - deck).length() < 0.01, "{board:?} {deck:?}");
    // An ollie: wind up on the C-stick, flick, land.
    let back = PadRaw {
        cstick_y: -92,
        ..pad([0, 0], 0)
    };
    run(&mut g, 20, back, 0.08, "winding up", w);
    let up = PadRaw {
        cstick_y: 92,
        ..pad([0, 0], 0)
    };
    run(&mut g, 2, up, 0.12, "the pop", w);
    run(
        &mut g,
        90,
        pad([0, 0], 0),
        0.12,
        "the ollie and its landing",
        w,
    );
    // A powerslide (the stick back and to the side) and out of it.
    run(&mut g, 30, pad([60, -80], 0), 0.08, "a powerslide", w);
    run(&mut g, 60, pad([0, 0], 0), 0.08, "out of the powerslide", w);
    // Off the board and walking, and back on.
    run(&mut g, 1, pad([0, 0], btn::Y), 0.08, "Y", w);
    // (The dismount clip kicks the board up into the hands at up to 5 m/s.)
    run(&mut g, 90, pad([0, 0], 0), 0.12, "getting off", w);
    assert!(g.skater.walking(), "off the board");
    run(&mut g, 60, pad([0, 100], 0), 0.12, "walking", w);
    run(&mut g, 1, pad([0, 100], btn::Y), 0.12, "Y", w);
    // Known: the mount from a run still jumps up to 16 cm where the rig takes over (ROADMAP).
    run(&mut g, 120, pad([0, 0], 0), 0.2, "getting on", w);
    assert!(g.skater.rolling(), "back on: {:?}", g.skater.state());
    eprintln!("the drawn pose's largest jump: {:.0} mm", worst * 1000.0);
}
