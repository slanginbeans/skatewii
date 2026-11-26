//! Skate 3's difficulty (Easy, Normal, Hardcore: the physics mode collection the Play Mode row selects)
//! and the truck tightness, as the pause menu sets them.
//!
//! - The pause menu's DIFFICULTY row switches the physics mode at once, without resetting the rider, and
//!   asks the platform to save it (`settings.txt`).
//! - Each mode rides with its own values: Easy's speed wobble starts later than Normal's (onset 22 against
//!   15 m/s), Hardcore's sooner (12).
//! - Tighter trucks hold off the wobble (`TruckTightnessSpeedMaxAdjust`) and steer less.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it):
//!   the disc's three collections are Skate 3's built-in ones, and in Skate 3's own graphs a nose drag
//!   ends the manual on Easy (no corrective force: `PhysicsWantsManualExit`) and holds it on Normal.

use std::path::PathBuf;

use game_core::Game;
use game_core::flick::FlickOut;
use game_core::input::{Pad, PadRaw, btn, flag};
use game_core::math::{Vec3, v3};
use game_core::park::Park;
use game_core::skater::Skater;
use game_core::tuning::{Difficulty, ModeTuning, Tuning};

const TICK: f32 = 1.0 / 60.0;

fn flat(half_size: f32) -> Park {
    Park {
        shapes: Vec::new(),
        half_size,
        wall_height: 2.0,
        spawn: Vec3::ZERO,
        spawn_yaw: 0.0,
        rails: Vec::new(),
    }
}

/// Ticks of speed wobble in two seconds of coasting straight at `speed` with `tuning`.
fn wobble_ticks(tuning: Tuning, speed: f32) -> u32 {
    let park = flat(400.0);
    let mut s = Skater::spawn(&park);
    s.tuning = tuning;
    s.put_at(&park, v3(0.0, 0.0, -350.0), 0.0, speed);
    let mut ev = Vec::new();
    let mut wobbled = 0;
    for _ in 0..120 {
        s.step(&park, &Pad::default(), &FlickOut::default(), TICK, &mut ev);
        assert!(s.rolling());
        wobbled += (s.wobble.amplitude > 0.0) as u32;
    }
    wobbled
}

fn with(d: Difficulty) -> Tuning {
    let mut t = Tuning::BUILT_IN;
    t.set_difficulty(d);
    t
}

#[test]
fn each_mode_rides_with_its_own_speed_wobble() {
    // Normal's onset with 0.7 trucks standing is about 18.5 m/s; Easy's about 25.5; Hardcore's 15.5.
    assert!(wobble_ticks(with(Difficulty::Normal), 23.0) > 100);
    assert_eq!(wobble_ticks(with(Difficulty::Easy), 23.0), 0, "Easy: later");
    assert_eq!(wobble_ticks(with(Difficulty::Normal), 19.0), 0);
    assert!(
        wobble_ticks(with(Difficulty::Hardcore), 19.0) > 100,
        "Hardcore: sooner"
    );
}

#[test]
fn tighter_trucks_hold_off_the_wobble_and_steer_less() {
    let mut loose = Tuning::BUILT_IN;
    loose.truck_tightness = 0.0;
    let mut tight = Tuning::BUILT_IN;
    tight.truck_tightness = 1.0;
    // Normal's onset is 15 m/s on loose trucks and 20 on tight ones.
    assert!(wobble_ticks(loose, 18.0) > 100);
    assert_eq!(wobble_ticks(tight, 18.0), 0);
    let (l, t) = (
        loose.steer_tilt(1.0, 5.0, 0.0, 1.0),
        tight.steer_tilt(1.0, 5.0, 0.0, 1.0),
    );
    assert!(
        t.abs() < l.abs(),
        "tight trucks tilt the deck less: {t} against {l}"
    );
}

fn tap(g: &mut Game, b: u32) {
    let pad = |buttons| PadRaw {
        flags: flag::CONNECTED,
        buttons,
        ..Default::default()
    };
    g.update(&pad(b), TICK);
    g.update(&pad(0), TICK);
}

#[test]
fn the_pause_menu_switches_difficulty_and_trucks_without_resetting_the_rider() {
    let mut g = Game::new();
    assert_eq!(g.difficulty(), Difficulty::Normal, "Skate 3's default");
    assert_eq!(g.truck_tightness(), 7);
    g.place_rider(v3(0.0, 0.0, 0.0), 0.0, 4.0);
    for _ in 0..10 {
        g.update(&PadRaw::default(), TICK);
    }
    tap(&mut g, btn::START);
    assert!(g.menu.is_some());
    let before = (g.skater.pos, g.skater.speed);
    for _ in 0..5 {
        tap(&mut g, btn::DPAD_DOWN);
    }
    tap(&mut g, btn::DPAD_LEFT);
    assert_eq!(g.difficulty(), Difficulty::Easy);
    assert_eq!(g.skater.tuning.difficulty, Difficulty::Easy);
    assert_eq!(ModeTuning::of(&g.skater.tuning), ModeTuning::EASY);
    assert!(!g.skater.tuning.manual_corrective);
    assert!(g.take_settings_changed(), "the platform saves it");
    // The trucks row, one down.
    tap(&mut g, btn::DPAD_DOWN);
    tap(&mut g, btn::DPAD_RIGHT);
    assert_eq!(g.truck_tightness(), 8);
    assert!((g.skater.tuning.truck_tightness - 0.8).abs() < 1e-6);
    assert!(g.take_settings_changed());
    assert_eq!((g.skater.pos, g.skater.speed), before, "the rider stays");
    // Back to Normal: everything as built in.
    g.set_difficulty(Difficulty::Normal);
    g.set_truck_tightness(7);
    assert_eq!(g.skater.tuning, Tuning::BUILT_IN);
}

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

fn imported(dir: &std::path::Path, d: Difficulty) -> Game {
    let mut g = Game::new();
    g.set_difficulty(d);
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
    g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
    assert!(g.graph.is_some(), "state graphs on");
    g
}

#[test]
fn the_discs_modes_are_the_built_in_ones() {
    let Some(dir) = data_dir() else {
        eprintln!("difficulty: no imported data (FG_DATA): skipped");
        return;
    };
    let rules = std::fs::read(dir.join("rules.fgpak")).unwrap();
    let pack = game_core::pack::Pack::parse(&rules).unwrap();
    let values = game_core::tuning::parse(pack.find("rules/tuning").unwrap().data).unwrap();
    let carried = |d: Difficulty| {
        values
            .iter()
            .filter(|(n, _)| n.starts_with(&format!("physics_mode/{}/", d.key())))
            .count()
    };
    let g = imported(&dir, Difficulty::Hardcore);
    let fields = game_core::tuning::mode::FIELDS.len();
    if carried(Difficulty::Easy) == 0 || carried(Difficulty::Normal) < fields {
        eprintln!(
            "difficulty: rules pack from before every mode field (re-import): partly skipped"
        );
        return;
    }
    assert_eq!(carried(Difficulty::Normal), fields);
    assert_eq!(g.tuning.modes[1], ModeTuning::NORMAL, "the disc's Normal");
    assert_eq!(
        (carried(Difficulty::Easy), carried(Difficulty::Hardcore)),
        (fields, fields)
    );
    assert_eq!(
        g.tuning.modes,
        ModeTuning::BUILT_IN,
        "the disc's three modes"
    );
    assert_eq!(ModeTuning::of(&g.skater.tuning), ModeTuning::HARDCORE);
}

fn cstick(y: i8) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        cstick_y: y,
        ..Default::default()
    }
}

/// Into a nose manual with the C-stick lightly forward, then all the way forward (the nose drags)
/// for a second: how many of those ticks were still a manual, and the motion graph's state at the end.
fn nose_drag(g: &mut Game) -> Option<(u32, String)> {
    let state = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    g.place_rider(v3(-16.0, 0.0, -17.0), 0.0, 6.0);
    for _ in 0..30 {
        g.update(&cstick(0), TICK);
    }
    for _ in 0..40 {
        g.update(&cstick(50), TICK);
    }
    if !state(g).contains("/NoseManual/") {
        eprintln!("difficulty: no nose manual in this motion graph: skipped");
        return None;
    }
    let mut manual = 0;
    for _ in 0..60 {
        g.update(&cstick(127), TICK);
        manual += state(g).contains("/Manual/") as u32;
    }
    Some((manual, state(g)))
}

#[test]
fn a_nose_drag_ends_the_manual_on_easy_and_holds_it_on_normal() {
    let Some(dir) = data_dir() else {
        eprintln!("difficulty: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = imported(&dir, Difficulty::Normal);
    assert!(g.skater.tuning.manual_corrective);
    let Some((manual, state)) = nose_drag(&mut g) else {
        return;
    };
    assert_eq!(manual, 60, "Normal holds it: {state}");
    let mut g = imported(&dir, Difficulty::Easy);
    assert!(
        !g.skater.tuning.manual_corrective,
        "Easy: no corrective force"
    );
    let (manual, state) = nose_drag(&mut g).unwrap();
    assert!(
        manual < 30,
        "Easy: still manualling {manual} ticks: {state}"
    );
    assert!(!state.contains("/Manual/"), "{state}");
    assert!(g.skater.manual.is_none());
}
