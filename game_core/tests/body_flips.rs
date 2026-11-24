//! Body flips turn end over end, as in Skate 3: the drawn rider's up sweeps through the direction of
//! travel (forwards for a front flip, backwards for a back flip) and never out to the side, for
//! regular and goofy riders, riding natural, switch or fakie (SK8 BeginBodyFlip: the axis is the
//! reckoning frame's right, across the board).
//!
//! - Without imported data, the state graphs' flip request is handed to the physics directly.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; passes without it),
//!   a back flip asked for through the pad (a grab and the left stick's `BackFlip` gesture) turns the
//!   drawn rider's head (the posed clips included) back over the travel.

use std::path::PathBuf;

use game_core::Game;
use game_core::input::{PadRaw, flag};
use game_core::math::{Vec3, v3};
use game_core::skater::{GraphControl, Mode, Stance};

const TICK: f32 = 1.0 / 60.0;

fn idle() -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        ..Default::default()
    }
}

/// How a scripted flip went: the most the drawn up leaned along the travel (signed, + forwards) and
/// out to the side (unsigned), and how far the flip turned (rad).
#[derive(Debug)]
struct Sweep {
    along: (f32, f32),
    side: f32,
    nose_side: f32,
    turned: f32,
}

/// A long flight off flat ground in the demo park with the graphs asking for a body flip and the grab
/// held: `fakie` rolls tail first.
fn scripted_flip(stance: Stance, switch: bool, fakie: bool, front: bool) -> Sweep {
    let mut g = Game::new();
    let speed = if fakie { -2.0 } else { 2.0 };
    g.place_rider(v3(0.0, 0.0, -8.0), 0.0, speed);
    g.skater.stance = stance;
    g.skater.switch = switch;
    g.skater.fakie = fakie;
    for _ in 0..5 {
        g.skater.speed = speed;
        g.update(&idle(), TICK);
    }
    let s = &mut g.skater;
    let travel = (s.fwd * speed.signum())
        .horizontal()
        .normalize_or(Vec3::ZERO);
    let side = Vec3::UP.cross(travel);
    s.pos.y += 0.05;
    s.vel = travel * 2.0 + v3(0.0, 6.5, 0.0);
    s.mode = Mode::Air { time: 0.0 };
    let mut sweep = Sweep {
        along: (0.0, 0.0),
        side: 0.0,
        nose_side: 0.0,
        turned: 0.0,
    };
    let mut k = 0;
    while matches!(g.skater.mode, Mode::Air { .. }) && k < 300 {
        g.skater.graph = Some(GraphControl {
            body_flip: (k < 20).then_some(front),
            grabbing: true,
            ..GraphControl::default()
        });
        g.update(&idle(), TICK);
        let Some(f) = g.skater.body_flip else {
            k += 1;
            continue;
        };
        sweep.turned = sweep.turned.max(f.angle.abs());
        let x = g.imported_xform();
        let (up, nose) = (x.rot.cols[1], x.rot.cols[2]);
        let a = up.dot(travel);
        sweep.along = (sweep.along.0.min(a), sweep.along.1.max(a));
        sweep.side = sweep.side.max(up.dot(side).abs());
        sweep.nose_side = sweep.nose_side.max(nose.dot(side).abs());
        k += 1;
    }
    sweep
}

#[test]
fn front_and_back_flips_turn_end_over_end_for_every_stance() {
    for stance in [Stance::Regular, Stance::Goofy] {
        for switch in [false, true] {
            for fakie in [false, true] {
                for front in [true, false] {
                    let what = format!(
                        "{} {stance:?} switch {switch} fakie {fakie}",
                        if front { "front flip" } else { "back flip" }
                    );
                    let s = scripted_flip(stance, switch, fakie, front);
                    assert!(s.turned > 2.0, "{what}: the flip turned only {s:?}");
                    // The head goes over through the travel: forwards for a front flip, back for a
                    // back flip (well past a quarter turn either way, so both signs show up).
                    if front {
                        assert!(s.along.1 > 0.9, "{what}: head not forwards {s:?}");
                    } else {
                        assert!(s.along.0 < -0.9, "{what}: head not backwards {s:?}");
                    }
                    // Never out to the side (a roll), and the board's nose stays in the travel's plane.
                    assert!(s.side < 0.1, "{what}: rolled sideways {s:?}");
                    assert!(s.nose_side < 0.1, "{what}: nose swung sideways {s:?}");
                }
            }
        }
    }
}

// ---- With the player's imported data ----

fn data_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FG_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist/apps/flatground/data")
        });
    ["skater.fgpak", "motion.fgpak", "rules.fgpak"]
        .iter()
        .all(|f| dir.join(f).is_file())
        .then_some(dir)
}

fn leak(path: PathBuf) -> &'static [u8] {
    Box::leak(std::fs::read(path).expect("read pack").into_boxed_slice())
}

/// The demo park with the player's rules, rider and state graphs; `None` (the test passes) without them.
fn imported_game(test: &str) -> Option<Game> {
    let Some(dir) = data_dir() else {
        eprintln!("{test}: no imported data (FG_DATA): skipped");
        return None;
    };
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .expect("rules pack");
    g.load_skater(leak(dir.join("skater.fgpak")))
        .expect("skater pack");
    g.load_motion(leak(dir.join("motion.fgpak")))
        .expect("motion pack");
    assert!(g.graph.is_some(), "state graphs on");
    Some(g)
}

/// Centroids of the drawn board (its wheels) and of the drawn rider (every other part), in the world.
fn drawn_board_and_body(g: &Game) -> (Vec3, Vec3) {
    let r = g.rider().expect("imported rider");
    let x = g.imported_xform();
    let mut sums = [(Vec3::ZERO, 0.0f32); 2];
    for p in &r.character.parts {
        let k = if p.name == "skatewheel" {
            0
        } else if p.name.starts_with("skate") {
            continue;
        } else {
            1
        };
        for v in &p.out {
            sums[k].0 += x.point(v3(v.pos[0], v.pos[1], v.pos[2]));
            sums[k].1 += 1.0;
        }
    }
    (
        sums[0].0 * (1.0 / sums[0].1.max(1.0)),
        sums[1].0 * (1.0 / sums[1].1.max(1.0)),
    )
}

/// The left stick for gesture point `p` (as the autoplay replays a gesture: gesture space has y towards
/// the player, the stick's y the other way).
fn stick(p: [f32; 2]) -> (i8, i8) {
    let mag = (p[0] * p[0] + p[1] * p[1]).sqrt();
    if mag < 1e-4 {
        return (0, 0);
    }
    let k = (mag.min(1.0) * 0.7 + 0.25) * game_core::input::MAIN_RANGE / mag;
    (
        (p[0] * k).clamp(-127.0, 127.0) as i8,
        (-p[1] * k).clamp(-127.0, 127.0) as i8,
    )
}

/// A long flight off the demo park's flat ground with the grab held and the left stick drawing the
/// disc's `gesture` (`BackFlip` / `FrontFlip`) straight after take-off, through the state graphs: the
/// most the drawn rider (board to body, the posed clips included) leaned back and forwards along the
/// travel and out to the side, and how far the flip turned.
fn imported_flip(test: &str, gesture: &str) -> Option<Sweep> {
    let mut g = imported_game(test)?;
    g.place_rider(v3(0.0, 0.0, -8.0), 0.0, 3.0);
    for _ in 0..30 {
        g.update(&idle(), TICK);
    }
    let travel = g.skater.vel.horizontal().normalize_or(Vec3::ZERO);
    let side = Vec3::UP.cross(travel);
    let s = &mut g.skater;
    s.pos.y += 0.05;
    s.vel = travel * 2.0 + v3(0.0, 6.5, 0.0);
    s.mode = Mode::Air { time: 0.0 };
    let shape = g
        .gesture_shape("skaterls", gesture)
        .expect("flip gesture")
        .points
        .clone();
    let mut sweep = Sweep {
        along: (0.0, 0.0),
        side: 0.0,
        nose_side: 0.0,
        turned: 0.0,
    };
    for k in 0..300usize {
        let mut pad = idle();
        pad.trig_r = 255;
        if let Some(&p) = k.checked_sub(1).and_then(|i| shape.get(i)) {
            (pad.stick_x, pad.stick_y) = stick(p);
        }
        g.update(&pad, TICK);
        if !matches!(g.skater.mode, Mode::Air { .. }) {
            break;
        }
        let Some(f) = g.skater.body_flip else {
            continue;
        };
        sweep.turned = sweep.turned.max(f.angle.abs());
        let (board, body) = drawn_board_and_body(&g);
        let d = (body - board).normalize_or(Vec3::ZERO);
        let a = d.dot(travel);
        sweep.along = (sweep.along.0.min(a), sweep.along.1.max(a));
        sweep.side = sweep.side.max(d.dot(side).abs());
        // The nose stays in the travel's plane but for the air's body spin about the board's up (Skate 3
        // turns the spin first, then the flip about its fixed axis: SK8 UpdateBodyFlip).
        let nose = g.imported_xform().rot.cols[2];
        let spun = g.skater.spin.sin().abs();
        sweep.nose_side = sweep.nose_side.max(nose.dot(side).abs() - spun);
    }
    Some(sweep)
}

#[test]
fn imported_flips_through_the_pad_turn_end_over_end() {
    for (gesture, front) in [("BackFlip", false), ("FrontFlip", true)] {
        let Some(s) = imported_flip("imported_flips_through_the_pad_turn_end_over_end", gesture)
        else {
            return;
        };
        assert!(s.turned > 2.0, "{gesture}: turned only {s:?}");
        // Head first back (or forwards), well over; the crouched grab pose sits the body a little over
        // the toe side, but it never swings out there as in a roll.
        if front {
            assert!(s.along.1 > 0.8, "{gesture}: head not forwards {s:?}");
        } else {
            assert!(s.along.0 < -0.8, "{gesture}: head not backwards {s:?}");
        }
        assert!(s.side < 0.5, "{gesture}: rolled sideways {s:?}");
        assert!(s.nose_side < 0.1, "{gesture}: nose swung sideways {s:?}");
    }
}
