//! Riding with the player's state graphs, pinned to SK8's simulation of the same pad (SK8-ENGINE/
//! skate-3-rust-engine run headless on the stock data; docs/graph-runner.md, "Measured against SK8").
//!
//! Needs the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`); without it the
//! tests pass and say so.

use std::path::PathBuf;

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::{atan2f, v3, wrap_angle};

const TICK: f32 = 1.0 / 60.0;

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

fn held(buttons: u32) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        buttons,
        ..Default::default()
    }
}

#[test]
fn the_foot_brake_drags_once_the_brake_clip_puts_the_foot_down() {
    // SK8: holding the brake at 6.65 m/s, the board keeps its speed through the brake's `INTO` clip
    // (`BRAKE_CONTACT` 0) and slows 4.08 m/s^2 from its `CYC` (contact 1), 0.23 s after the press.
    let Some(mut g) = imported_game("foot brake") else {
        return;
    };
    g.place_rider(v3(-16.0, 0.0, -17.0), 0.0, 6.65);
    for _ in 0..60 {
        g.update(&held(0), TICK);
    }
    let v0 = g.skater.speed;
    let mut first = None;
    let mut speeds = Vec::new();
    for i in 0..60 {
        g.update(&held(btn::X), TICK);
        speeds.push(g.skater.speed);
        if first.is_none() && g.skater.speed < v0 - 0.03 {
            first = Some(i);
        }
    }
    let first = first.expect("the brake slows the board") as f32 * TICK;
    assert!((0.15..0.3).contains(&first), "foot down after {first} s");
    let decel = (speeds[29] - speeds[59]) * 2.0;
    assert!((decel - 4.08).abs() < 0.25, "{decel} m/s^2");
}

#[test]
fn a_turn_let_go_just_before_the_pop_does_not_spin_it() {
    // SK8 at 6 m/s: the main stick held left while winding up an ollie and let go a tick before the
    // board leaves the ground. Known air's landing assist owns the spin from take-off, so the body
    // turns about nothing in the air (-7 degrees); the let-go copy of the stick used to spin it on
    // (46 degrees) while the take-off's landing was still being looked for.
    let Some(mut g) = imported_game("let-go spin") else {
        return;
    };
    g.place_rider(v3(-16.0, 0.0, -17.0), 0.0, 6.0);
    for _ in 0..30 {
        g.update(&held(0), TICK);
    }
    let yaw = |g: &Game| atan2f(g.skater.fwd.x, g.skater.fwd.z);
    let mut takeoff = None;
    let mut landed = None;
    for p in 0..150 {
        let cstick_y = if p < 40 {
            -(game_core::input::C_RANGE as i8)
        } else if p < 45 {
            game_core::input::C_RANGE as i8
        } else {
            0
        };
        let stick_x = if (30..52).contains(&p) { -100 } else { 0 };
        g.update(
            &PadRaw {
                flags: flag::CONNECTED,
                stick_x,
                cstick_y,
                ..Default::default()
            },
            TICK,
        );
        let air = g.skater.is_airborne();
        if air && takeoff.is_none() {
            takeoff = Some(yaw(&g));
        }
        if takeoff.is_some() && !air && landed.is_none() {
            landed = Some(yaw(&g));
        }
    }
    let (Some(a), Some(b)) = (takeoff, landed) else {
        panic!("no ollie: {takeoff:?} {landed:?}");
    };
    let turned = wrap_angle(b - a).abs().to_degrees();
    assert!(turned < 20.0, "{turned} degrees in the air");
}

#[test]
fn held_pushes_chase_the_posed_trajectory_speed() {
    // SK8 holding the push from a standstill: the board chases the posed push's trajectory speed while
    // the foot is down (0.06, 0.39, 1.16, 2.24, 3.18, 3.43, 2.70, 3.96 m/s on its eight contact ticks),
    // so the first push ends at 3.22 m/s, the second at 6.09, the third at 8.07, and 8 m/s takes
    // 2.63 s. Chasing the push sets' end speed gave 4.5 m/s and 1.7 s.
    let Some(mut g) = imported_game("held push") else {
        return;
    };
    g.place_rider(v3(-16.0, 0.0, -17.0), 0.0, 0.0);
    for _ in 0..60 {
        g.update(&held(0), TICK);
    }
    let mut plateaus = Vec::new();
    let mut last = 0.0f32;
    let mut still = 0;
    let mut to_8 = None;
    for i in 0..300 {
        g.update(&held(btn::A), TICK);
        let v = g.skater.speed;
        if (v - last).abs() < 1e-3 && v > 0.5 {
            still += 1;
            if still == 10 && plateaus.last().is_none_or(|p: &f32| (v - p).abs() > 0.5) {
                plateaus.push(v);
            }
        } else {
            still = 0;
        }
        if to_8.is_none() && v >= 8.0 {
            to_8 = Some((i + 1) as f32 * TICK);
        }
        last = v;
    }
    assert!(plateaus.len() >= 3, "plateaus {plateaus:?}");
    for (got, sk8) in plateaus.iter().zip([3.22, 6.09, 8.07]) {
        assert!(
            (got - sk8).abs() < 0.12,
            "plateaus {plateaus:?} (SK8 3.22, 6.09, 8.07)"
        );
    }
    let t = to_8.expect("reaches 8 m/s");
    assert!((t - 2.63).abs() < 0.1, "{t} s to 8 m/s (SK8 2.63)");
}
