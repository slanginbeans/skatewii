//! Riding with the player's state graphs, pinned to SK8's simulation of the same pad (SK8-ENGINE/
//! skate-3-rust-engine run headless on the stock data; docs/graph-runner.md, "Measured against SK8").
//!
//! Needs the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`); without it the
//! tests pass and say so.

use std::path::PathBuf;

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::v3;

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
