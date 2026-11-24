//! Combos and lines (Skate 3's scoring: `game_core::scoring`) through the real input path on flat ground,
//! without imported data:
//!
//! - Three flips in a row make a line: the first is scored at x1 and starts the multiplier (x1.5 once
//!   its reward is in the combo timer), the next two are multiplied, and the line's score is more than
//!   their rewards.
//! - A bail (a grab still tweaked into the landing) scores nothing for its trick and ends the line: the
//!   multiplier is back to x1 and what the line had is banked.
//! - Rolling on with nothing done, the line timer runs out and the line ends.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it):
//!   the rules pack's trick catalogue loads and scores a kickflip at the disc's points.

use std::path::PathBuf;

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::v3;
use game_core::park::Park;
use game_core::scoring::catalog;
use game_core::skater::{Event, Mode};

const TICK: f32 = 1.0 / 60.0;
/// Raw C-stick: pulled back (the wind-up), flicked forward-right and forward-left (a kickflip and a
/// heelflip for the built-in shapes), pushed to the side (a grab's tweak).
const BACK: (i8, i8) = (0, -92);
const UP_RIGHT: (i8, i8) = (50, 62);
const UP_LEFT: (i8, i8) = (-50, 62);
const TWEAK: (i8, i8) = (80, 20);

fn flat() -> Park {
    Park {
        shapes: Vec::new(),
        half_size: 200.0,
        wall_height: 2.0,
        spawn: v3(0.0, 0.0, -150.0),
        spawn_yaw: 0.0,
        rails: Vec::new(),
    }
}

/// A game on flat ground, rolling along +z.
fn game() -> Game {
    let mut g = Game::new();
    g.park = flat();
    g.place_rider(v3(0.0, 0.0, -150.0), 0.0, 7.0);
    for _ in 0..30 {
        g.update(&pad(), TICK);
    }
    g
}

fn pad() -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        ..Default::default()
    }
}

/// Runs one frame, keeping the landings (name, raw reward).
fn frame(g: &mut Game, p: &PadRaw, landed: &mut Vec<(String, u32)>, bailed: &mut bool) {
    g.update(p, TICK);
    for e in &g.events {
        match e {
            Event::Landed { name, points, .. } => landed.push((name.clone(), *points)),
            Event::Bailed => *bailed = true,
            _ => {}
        }
    }
}

/// Winds up, flicks `end` (grabbing and tweaking in the air when `tweak`), and rides until back on the
/// ground and settled (pushing back up to speed).
fn flip(g: &mut Game, end: (i8, i8), tweak: bool, landed: &mut Vec<(String, u32)>) -> bool {
    let mut bailed = false;
    for _ in 0..20 {
        let p = PadRaw {
            cstick_x: BACK.0,
            cstick_y: BACK.1,
            ..pad()
        };
        frame(g, &p, landed, &mut bailed);
    }
    let p = PadRaw {
        cstick_x: end.0,
        cstick_y: end.1,
        ..pad()
    };
    frame(g, &p, landed, &mut bailed);
    let mut ticks = 0;
    while ticks < 200 {
        ticks += 1;
        let air = g.skater.is_airborne();
        let p = if tweak && air {
            PadRaw {
                trig_l: 255,
                cstick_x: TWEAK.0,
                cstick_y: TWEAK.1,
                ..pad()
            }
        } else {
            pad()
        };
        frame(g, &p, landed, &mut bailed);
        if ticks > 10 && !air && g.skater.mode == Mode::Rolling {
            break;
        }
        if bailed {
            break;
        }
    }
    // Roll a moment, pushing if it slowed (three ticks publish the sequence).
    for _ in 0..12 {
        let p = PadRaw {
            buttons: if g.skater.speed < 6.0 && g.skater.mode == Mode::Rolling {
                btn::A
            } else {
                0
            },
            ..pad()
        };
        frame(g, &p, landed, &mut bailed);
    }
    bailed
}

#[test]
fn a_line_of_three_flips_is_multiplied() {
    let mut g = game();
    let mut landed = Vec::new();
    assert!(!flip(&mut g, UP_RIGHT, false, &mut landed));
    assert_eq!(landed.len(), 1, "{landed:?}");
    let first = landed[0].1 as f32;
    assert!(first >= 100.0, "{landed:?}");
    assert!(g.scoring.in_line(), "the first flip starts the line");
    assert_eq!(g.scoring.multiplier(), 1.5);
    assert!((g.scoring.line_score() - first).abs() <= 1.0);
    assert!(!flip(&mut g, UP_LEFT, false, &mut landed));
    assert!(!flip(&mut g, UP_RIGHT, false, &mut landed));
    assert_eq!(landed.len(), 3, "{landed:?}");
    let raw: f32 = landed.iter().map(|l| l.1 as f32).sum();
    let line = g.scoring.line_score();
    // The second and third at x1.5.
    let expect = first + 1.5 * (landed[1].1 + landed[2].1) as f32;
    assert!((line - expect).abs() <= 3.0, "{line} {expect} {landed:?}");
    assert!(line > raw + 10.0, "{line} {raw}");
    assert!(g.scoring.in_line());
    assert_eq!(g.scoring.names().count(), 3);
    // The same flip again in the line scores half (Skate 3's repetition penalty).
    assert!(
        (landed[2].1 as f32 - first * 0.5).abs() <= 1.0,
        "{landed:?}"
    );
    assert_eq!(
        g.score,
        libm::roundf(line) as u32,
        "the total counts the line"
    );
}

#[test]
fn a_bail_loses_the_line() {
    let mut g = game();
    let mut landed = Vec::new();
    assert!(!flip(&mut g, UP_RIGHT, false, &mut landed));
    assert!(g.scoring.in_line() && g.scoring.multiplier() > 1.0);
    let line = g.scoring.line_score();
    assert!(
        flip(&mut g, UP_LEFT, true, &mut landed),
        "tweaked into the landing"
    );
    assert_eq!(landed.len(), 1, "the bailed flip isn't scored: {landed:?}");
    assert!(!g.scoring.in_line(), "the line is over");
    assert_eq!(g.scoring.multiplier(), 1.0);
    assert_eq!(g.scoring.line_score(), 0.0);
    assert_eq!(g.scoring.names().count(), 0);
    assert!(
        (g.scoring.total() - line).abs() < 0.01,
        "banked before the bail"
    );
}

#[test]
fn the_line_ends_when_its_timer_runs_out() {
    let mut g = game();
    let mut landed = Vec::new();
    assert!(!flip(&mut g, UP_RIGHT, false, &mut landed));
    let line = g.scoring.line_score();
    assert!(line > 0.0 && g.scoring.line_seconds() > 1.0);
    let mut ticks = 0;
    let mut bailed = false;
    while g.scoring.in_line() && ticks < 60 * 20 {
        frame(&mut g, &pad(), &mut landed, &mut bailed);
        ticks += 1;
    }
    // At most 8 s (Skate 3's 400-point line timer drained at 50 a second); about 2.2 s for a kickflip.
    assert!(ticks > 60 && ticks < 60 * 8, "{ticks}");
    assert_eq!(g.scoring.multiplier(), 1.0);
    assert_eq!(g.scoring.line_score(), 0.0);
    assert_eq!(g.score, libm::roundf(line) as u32, "banked");
    // The next flip starts a new line at x1 and full points.
    let before = landed.len();
    assert!(!flip(&mut g, UP_RIGHT, false, &mut landed));
    assert!((landed[before].1 as f32 - landed[0].1 as f32).abs() <= 1.0);
}

/// The player's imported data, if there is any.
fn imported() -> Option<PathBuf> {
    let dir = std::env::var_os("FG_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist/apps/flatground/data")
        });
    dir.join("rules.fgpak").exists().then_some(dir)
}

#[test]
fn imported_the_disc_trick_catalogue_scores_a_kickflip() {
    let Some(dir) = imported() else {
        eprintln!("skipped: no imported data");
        return;
    };
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    let c = &g.scoring.catalog;
    assert!(
        c.imported,
        "the rules pack has a trick catalogue (re-import)"
    );
    assert!(c.len() > 200, "{}", c.len());
    let kickflip = catalog::id("kickflip").unwrap();
    let (points, _) = c.get(kickflip).expect("a kickflip record");
    assert!(points > 0);
    assert!(c.get(catalog::id("fs_50_50").unwrap()).is_some());
    // The disc's session numbers came through the rules pack's tuning.
    assert!(g.scoring.rules.line_capacity > 0.0 && g.scoring.rules.line_drain > 0.0);
    // A kickflip landed clean scores the disc's points, at the disc's clean landing factor.
    let frame = game_core::scoring::Frame {
        collector: game_core::scoring::Collector::Ground,
        speed: 6.0,
        ..Default::default()
    };
    let r = g.scoring.credit(
        "KICKFLIP",
        &game_core::scoring::Scored::Air(game_core::scoring::Air {
            trick: Some(kickflip),
            landing: game_core::scoring::Landing::Clean,
            ..Default::default()
        }),
        &frame,
    );
    let k = g.scoring.rules.land_clean;
    assert!((r - points as f32 * k).abs() < 0.01, "{r} {points} {k}");
}
