//! Skate 3's speed wobble and the ways a manual ends.
//!
//! - Riding fast and straight, the board starts to wobble above Skate 3's onset (15 m/s in Normal, plus
//!   the trucks' tightness and the crouch) and stops once it slows below it.
//! - Without the state graphs, flatground's own balance meter tips a manual held badly (the stick left
//!   past its sweet spot) back onto four wheels.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it):
//!   in Skate 3's own graphs, dragging the tail holds the manual in Normal (the corrective force is on),
//!   and ends it when the physics mode has no corrective force (Easy: `PhysicsWantsManualExit`).

use std::path::PathBuf;

use game_core::Game;
use game_core::flick::FlickOut;
use game_core::input::{Pad, PadRaw, btn, flag};
use game_core::math::{Vec3, v3};
use game_core::park::Park;
use game_core::skater::wobble::SpeedWobble;
use game_core::skater::{Event, Mode, Skater};

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

#[test]
fn a_fast_straight_ride_wobbles_above_the_onset_and_settles_below_it() {
    let park = flat(400.0);
    let mut s = Skater::spawn(&park);
    s.put_at(&park, v3(0.0, 0.0, -350.0), 0.0, 23.0);
    let onset = SpeedWobble::onset(&s.tuning.wobble, 0.96, s.tuning.truck_tightness);
    assert!((18.0..19.0).contains(&onset), "onset {onset} m/s");
    let mut ev = Vec::new();
    let coast = Pad::default();
    let mut wobbled = 0;
    let mut tilt = (f32::MAX, f32::MIN);
    for _ in 0..120 {
        s.step(&park, &coast, &FlickOut::default(), TICK, &mut ev);
        assert_eq!(s.mode, Mode::Rolling);
        if s.wobble.amplitude > 0.0 {
            wobbled += 1;
            tilt = (tilt.0.min(s.deck_tilt), tilt.1.max(s.deck_tilt));
        }
    }
    assert!(s.speed > onset, "still fast: {} m/s", s.speed);
    assert!(wobbled > 100, "wobbled {wobbled} ticks");
    // No steering, yet the deck rocks both ways.
    assert!(tilt.0 < -0.0005 && tilt.1 > 0.0005, "deck tilt {tilt:?}");
    // The foot brake down below the onset: the wobble stops while the board still rolls.
    let brake = Pad {
        held: btn::X,
        ..Pad::default()
    };
    let mut below = None;
    for i in 0..600 {
        s.step(&park, &brake, &FlickOut::default(), TICK, &mut ev);
        let onset = SpeedWobble::onset(&s.tuning.wobble, s.riding_com(), 0.7);
        if s.speed < onset - 0.5 {
            below = Some(i);
            break;
        }
    }
    assert!(below.is_some(), "braked below the onset: {} m/s", s.speed);
    for _ in 0..30 {
        s.step(&park, &coast, &FlickOut::default(), TICK, &mut ev);
        assert!(!s.wobble.active && s.wobble.amplitude == 0.0);
    }
    assert!(s.speed > 5.0 && s.mode == Mode::Rolling);
}

#[test]
fn without_the_graphs_a_manual_held_past_its_sweet_spot_tips_back_onto_four_wheels() {
    let park = flat(100.0);
    let mut s = Skater::spawn(&park);
    s.put_at(&park, v3(0.0, 0.0, -80.0), 0.0, 5.0);
    let mut ev = Vec::new();
    // The C-stick held most of the way back (towards the player), never corrected.
    let back = FlickOut {
        stick: [0.0, 0.8],
        ..FlickOut::default()
    };
    let mut manualled = 0;
    for _ in 0..(5 * 60) {
        s.speed = s.speed.max(4.0);
        s.step(&park, &Pad::default(), &back, TICK, &mut ev);
        manualled += s.manual.is_some() as u32;
        if ev.contains(&Event::ManualLost) {
            break;
        }
    }
    assert!(manualled > 10, "it was a manual");
    assert!(ev.contains(&Event::ManualLost), "{ev:?}");
    assert!(s.manual.is_none() && s.mode == Mode::Rolling);
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

fn imported(dir: &std::path::Path) -> Game {
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
    g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
    assert!(g.graph.is_some(), "state graphs on");
    g
}

fn cstick(y: i8) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        cstick_y: y,
        ..Default::default()
    }
}

/// Into a tail manual with the C-stick lightly back, then all the way back (the tail drags) for a
/// second: how many of those ticks were still a manual, and the motion graph's state at the end.
fn tail_drag(g: &mut Game) -> Option<(u32, String)> {
    let state = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    g.place_rider(v3(-16.0, 0.0, -17.0), 0.0, 6.0);
    for _ in 0..30 {
        g.update(&cstick(0), TICK);
    }
    for _ in 0..40 {
        g.update(&cstick(-50), TICK);
    }
    if !state(g).contains("/TailManual/") {
        eprintln!(
            "manual_wobble: no tail manual in this motion graph ({}): skipped",
            state(g)
        );
        return None;
    }
    let mut manual = 0;
    for _ in 0..60 {
        g.update(&cstick(-127), TICK);
        manual += state(g).contains("/Manual/") as u32;
    }
    Some((manual, state(g)))
}

#[test]
fn imported_graphs_end_a_dragged_manual_only_without_the_corrective_force() {
    let Some(dir) = data_dir() else {
        eprintln!("manual_wobble: no imported data (FG_DATA): skipped");
        return;
    };
    // Skate 3's Normal: the tail drags and the manual goes on.
    let mut g = imported(&dir);
    assert!(g.skater.tuning.manual_corrective);
    let Some((manual, state)) = tail_drag(&mut g) else {
        return;
    };
    assert_eq!(manual, 60, "{state}");
    assert!(state.contains("/TailManual/Brake"), "{state}");
    // No corrective force (Skate 3's Easy): the physics wants out and the graph leaves the manual.
    let mut g = imported(&dir);
    g.skater.tuning.manual_corrective = false;
    let (manual, state) = tail_drag(&mut g).unwrap();
    assert!(manual < 10, "still manualling {manual} ticks: {state}");
    assert!(!state.contains("/Manual/"), "{state}");
    assert!(g.skater.manual.is_none());
}
