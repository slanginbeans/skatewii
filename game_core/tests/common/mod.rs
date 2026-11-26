//! Helpers shared by the scripted runs: the player's imported data when it is there, and a synthetic
//! state graph (`GraphControl`, what the game's graphs hand the physics each tick) for the runs that go
//! without it. A `Game` without a graph driver leaves the skater's `graph` as the test sets it.
#![allow(dead_code)]

use std::path::PathBuf;

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::skater::GraphControl;

pub const TICK: f32 = 1.0 / 60.0;

/// The player's imported data (`FG_DATA`, else `dist/apps/flatground/data`), when all of it is there.
pub fn data_dir() -> Option<PathBuf> {
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

pub fn leak(path: PathBuf) -> &'static [u8] {
    Box::leak(std::fs::read(path).expect("read pack").into_boxed_slice())
}

/// The demo park with the player's rules, rider and state graphs; `None` (the test passes) without them.
pub fn disc_game(test: &str) -> Option<Game> {
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
    assert!(g.require_disc_data(), "disc data complete");
    Some(g)
}

pub fn pad(stick: [i8; 2], buttons: u32) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        stick_x: stick[0],
        stick_y: stick[1],
        buttons,
        ..Default::default()
    }
}

/// Y off the board as Skate 3's graphs time it (the dismount clip walking the rider: `OB_TRAJ` in the
/// `Dismount` states), with a synthetic graph; the graph is cleared after.
pub fn get_off(g: &mut Game) {
    g.skater.graph = GraphControl {
        ob_traj: true,
        dismounting: true,
        ..GraphControl::default()
    };
    g.update(&pad([0, 0], btn::Y), TICK);
    for _ in 0..10 {
        if g.skater.walking() {
            break;
        }
        g.update(&pad([0, 0], 0), TICK);
    }
    g.skater.graph = GraphControl::default();
    assert!(g.skater.walking(), "off the board");
}

/// Y back on the board as Skate 3's graphs time it (the mount clip's `OB_TRAJ`, then its end), with a
/// synthetic graph; the graph is cleared after.
pub fn get_on(g: &mut Game) {
    get_on_with(g, [0, 0]);
}

/// [`get_on`] with the stick held at `stick`.
pub fn get_on_with(g: &mut Game, stick: [i8; 2]) {
    g.skater.graph = GraphControl {
        ob_traj: true,
        mounting: true,
        ..GraphControl::default()
    };
    g.update(&pad(stick, btn::Y), TICK);
    g.skater.graph.ob_traj = false;
    for _ in 0..10 {
        if !g.skater.walking() {
            break;
        }
        g.update(&pad(stick, 0), TICK);
    }
    g.skater.graph = GraphControl::default();
}
