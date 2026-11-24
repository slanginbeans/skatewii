//! Grind balance and grind grabs (Skate 3's rigid board held on the rail by its balance forces,
//! `skater::balance`), on synthetic rails and without imported data:
//!
//! - A 50-50 that comes onto the rail drifting sideways is pulled back over it by the pin and rides to
//!   the end.
//! - A nose slide: the main stick pushed the wrong way (outwards) slides the rail off the nose's end and
//!   the board falls off the rail; pushed the other way the board stays on it (in past the truck it is
//!   a boardslide).
//! - A boardslide on a ledge pushed off the low side by the sticks falls off it.
//! - Coming down hard onto a rail with the board rolled against it (its closing speed across the
//!   rider's ground plane over `Wipeout_GrindXZDeck`) wipes out; the same, slower, grinds.
//! - A trigger fully in during a 50-50 is a grind grab, scored with the grind.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it):
//!   Skate 3's graphs go into their grind-grab states and the grind scores the grab.

use std::path::PathBuf;

use game_core::Game;
use game_core::flick::FlickOut;
use game_core::input::{Pad, PadRaw, flag};
use game_core::math::{Vec3, v3};
use game_core::park::Park;
use game_core::rails::Rail;
use game_core::skater::{DECK_HEIGHT, Event, Family, Mode, Skater};

const TICK: f32 = 1.0 / 60.0;

/// Flat ground with a thin rail 0.5 m up along +z from z = 0 to `len` (and on through `then`).
fn rail_park(len: f32, then: Option<Vec3>) -> Park {
    let mut points = vec![v3(0.0, 0.5, 0.0), v3(0.0, 0.5, len)];
    points.extend(then);
    Park {
        shapes: Vec::new(),
        half_size: 60.0,
        wall_height: 2.0,
        spawn: v3(0.0, 0.0, -3.0),
        spawn_yaw: 0.0,
        rails: vec![Rail::new(points)],
    }
}

/// A skater in the air with its deck's middle at `deck`, its up `up` and nose `fwd`, moving at `vel`.
fn in_air(p: &Park, deck: Vec3, up: Vec3, fwd: Vec3, vel: Vec3) -> Skater {
    let mut s = Skater::spawn(p);
    s.since_spawn = 10.0;
    s.pos = deck - up * DECK_HEIGHT;
    s.normal = up;
    s.contact_n = up;
    s.fwd = fwd;
    s.heading = game_core::math::atan2f(fwd.x, fwd.z);
    s.vel = vel;
    s.mode = Mode::Air { time: 0.5 };
    s
}

fn step(s: &mut Skater, p: &Park, pad: &Pad, ticks: usize, ev: &mut Vec<Event>) {
    for _ in 0..ticks {
        s.step(p, pad, &FlickOut::default(), TICK, ev);
    }
}

fn landed(ev: &[Event]) -> Vec<String> {
    ev.iter()
        .filter_map(|e| match e {
            Event::Landed { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_50_50_drifting_sideways_onto_the_rail_is_pulled_back_over_it() {
    let p = rail_park(12.0, None);
    // 3 cm off the rail, drifting 0.8 m/s across it (an air entry keeps 60% of that).
    let mut s = in_air(
        &p,
        v3(0.03, 0.56, 1.0),
        Vec3::UP,
        v3(0.0, 0.0, 1.0),
        v3(0.8, -1.0, 5.0),
    );
    let mut ev = Vec::new();
    step(&mut s, &p, &Pad::default(), 2, &mut ev);
    assert_eq!(s.mode, Mode::Grind, "{ev:?}");
    let g = s.grind.as_ref().unwrap();
    assert_eq!(g.family, Family::FiftyFifty);
    assert!(
        g.lateral_vel.abs() > 0.3,
        "still drifting: {}",
        g.lateral_vel
    );
    let mut widest = 0.0f32;
    for _ in 0..60 {
        step(&mut s, &p, &Pad::default(), 1, &mut ev);
        let g = s.grind.as_ref().expect("still on the rail");
        widest = widest.max(g.lateral.abs());
    }
    let g = s.grind.as_ref().unwrap();
    // The pin caught it within the truck's width and settled it back over the rail.
    assert!(widest > 0.03 && widest < 0.095, "widest {widest}");
    assert!(g.lateral.abs() < 0.01, "settled {}", g.lateral);
    assert!(s.pos.x.abs() < 0.02, "{:?}", s.pos);
    step(&mut s, &p, &Pad::default(), 120, &mut ev);
    assert_eq!(s.grind_exit, Some("off the end"));
    let names = landed(&ev);
    assert!(names.iter().any(|n| n.ends_with("50-50")), "{ev:?}");
    assert!(!ev.contains(&Event::Bailed), "{ev:?}");
}

/// A nose slide on the rail: the board across it (nose +x), its middle 0.33 m short of the rail, so
/// the rail is on the nose's tip.
fn nose_slide(p: &Park) -> Skater {
    let mut s = in_air(
        p,
        v3(-0.33, 0.6, 1.0),
        Vec3::UP,
        v3(1.0, 0.0, 0.0),
        v3(0.0, -1.0, 5.0),
    );
    let mut ev = Vec::new();
    step(&mut s, p, &Pad::default(), 1, &mut ev);
    assert_eq!(
        s.grind.as_ref().map(|g| g.family),
        Some(Family::Tipslide),
        "{ev:?}"
    );
    s
}

#[test]
fn leaning_the_wrong_way_out_of_a_nose_slide_falls_off_the_rail() {
    let p = rail_park(20.0, None);
    // Left alone the nose slide holds.
    let mut s = nose_slide(&p);
    let mut ev = Vec::new();
    step(&mut s, &p, &Pad::default(), 40, &mut ev);
    assert_eq!(s.mode, Mode::Grind, "{ev:?}");
    assert_eq!(s.grind.as_ref().unwrap().family, Family::Tipslide);
    // The main stick right pushes the board to the rider's right of the travel (-x here): out
    // along the nose, off its tip.
    let mut s = nose_slide(&p);
    let mut ev = Vec::new();
    let out = Pad {
        stick: [1.0, 0.0],
        ..Pad::default()
    };
    let mut ticks = 0;
    while s.mode == Mode::Grind && ticks < 120 {
        step(&mut s, &p, &out, 1, &mut ev);
        ticks += 1;
    }
    assert_eq!(s.grind_exit, Some("fell off"), "{ev:?}");
    assert!(ticks < 60, "off in {ticks} ticks");
    assert!(s.pos.z < 10.0 && s.vel.x < 0.0, "{:?} {:?}", s.pos, s.vel);
    assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    // The other way the board stays on: pushed in past the truck, the rail is under the deck.
    let mut s = nose_slide(&p);
    let mut ev = Vec::new();
    let inward = Pad {
        stick: [-1.0, 0.0],
        ..Pad::default()
    };
    step(&mut s, &p, &inward, 60, &mut ev);
    assert_eq!(s.mode, Mode::Grind, "{ev:?}");
    assert_eq!(s.grind.as_ref().unwrap().family, Family::Boardslide);
}

#[test]
fn a_boardslide_on_a_ledge_pushed_off_its_side_falls_off() {
    // A 0.4 m ledge (its top x 0..2), its edge at x = 0 a rail.
    let mut tris: Vec<([[f32; 3]; 3], u16, u16)> = Vec::new();
    let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
        tris.push(([a, b, c], 0, 0));
        tris.push(([a, c, d], 0, 0));
    };
    quad(
        [-20.0, 0.0, -30.0],
        [-20.0, 0.0, 30.0],
        [20.0, 0.0, 30.0],
        [20.0, 0.0, -30.0],
    );
    quad(
        [0.0, 0.4, -10.0],
        [0.0, 0.4, 10.0],
        [2.0, 0.4, 10.0],
        [2.0, 0.4, -10.0],
    );
    quad(
        [0.0, 0.0, -10.0],
        [0.0, 0.4, -10.0],
        [0.0, 0.4, 10.0],
        [0.0, 0.0, 10.0],
    );
    let mut m = game_core::collision::TriMesh::parse(
        &game_core::collision::encode(&tris),
        (v3(-0.6, 0.0, -9.0), 0.0),
    )
    .unwrap();
    let rail = Rail::new(vec![v3(0.0, 0.4, -10.0), v3(0.0, 0.4, 10.0)]).classified(&m);
    m.rails = vec![rail];
    let slide = |stick: f32| {
        let mut s = Skater::spawn(&m);
        s.since_spawn = 10.0;
        let up = Vec3::UP;
        s.pos = v3(0.0, 0.43, -8.0) - up * DECK_HEIGHT;
        s.fwd = v3(1.0, 0.0, 0.0);
        s.heading = core::f32::consts::FRAC_PI_2;
        s.vel = v3(0.0, -1.0, 4.0);
        s.mode = Mode::Air { time: 0.5 };
        let mut ev = Vec::new();
        s.step(&m, &Pad::default(), &FlickOut::default(), TICK, &mut ev);
        assert_eq!(
            s.grind.as_ref().map(|g| (g.family, g.class.is_ledge())),
            Some((Family::Boardslide, true)),
            "{ev:?}"
        );
        let pad = Pad {
            stick: [stick, 0.0],
            ..Pad::default()
        };
        let mut ticks = 0;
        while s.mode == Mode::Grind && ticks < 90 {
            s.step(&m, &pad, &FlickOut::default(), TICK, &mut ev);
            ticks += 1;
        }
        (s, ticks, ev)
    };
    // Untouched it slides on (the ledge leans it off only after its 2 s delay).
    let (s, ticks, ev) = slide(0.0);
    assert!(s.mode == Mode::Grind && ticks == 90, "{ev:?}");
    // The stick right pushes it to -x: off the ledge's low side.
    let (s, ticks, ev) = slide(1.0);
    assert_eq!(s.grind_exit, Some("fell off"), "{ev:?}");
    assert!(ticks < 60 && s.vel.x < 0.0, "{ticks} {:?}", s.vel);
}

#[test]
fn slamming_down_onto_a_rail_with_the_board_rolled_wipes_out() {
    let p = rail_park(12.0, None);
    // The board rolled 45 degrees about its nose (along the rail), falling straight down: after this
    // tick's fall the rail is 12 cm under the deck along its up, in both truck boxes.
    let s2 = core::f32::consts::FRAC_1_SQRT_2;
    let up = v3(s2, s2, 0.0);
    let drop = |speed: f32| {
        let fall = speed * TICK + 9.81 * TICK * TICK;
        let deck = v3(0.0, 0.5, 4.0) + up * 0.12 + v3(0.0, fall, 0.0);
        let mut s = in_air(&p, deck, up, v3(0.0, 0.0, 1.0), v3(0.0, -speed, 4.0));
        let mut ev = Vec::new();
        step(&mut s, &p, &Pad::default(), 1, &mut ev);
        (s, ev)
    };
    // 9.5 m/s down: 6.7 m/s of it across the rolled rider's ground plane, over Skate 3's 6.
    let (s, ev) = drop(9.5);
    assert!(ev.contains(&Event::Bailed), "{:?} {ev:?}", s.mode);
    assert!(matches!(s.mode, Mode::Bailed { .. }));
    assert_eq!(s.bail_reason, Some("slammed into the rail"));
    // 5 m/s: 3.5 m/s across, a grind.
    let (s, ev) = drop(5.0);
    assert_eq!(s.mode, Mode::Grind, "{ev:?}");
    assert!(!ev.contains(&Event::Bailed), "{ev:?}");
}

#[test]
fn a_trigger_in_on_a_50_50_is_a_grind_grab_scored_with_it() {
    let p = rail_park(12.0, None);
    let mut s = in_air(
        &p,
        v3(0.0, 0.56, 1.0),
        Vec3::UP,
        v3(0.0, 0.0, 1.0),
        v3(0.0, -1.0, 5.0),
    );
    let mut ev = Vec::new();
    step(&mut s, &p, &Pad::default(), 2, &mut ev);
    assert_eq!(s.grind.as_ref().map(|g| g.family), Some(Family::FiftyFifty));
    // The left trigger fully in: the frontside hand (the rider isn't mirrored goofy; mirrored, it is
    // the backside one), held half a second.
    let pad = Pad {
        trig_l: 1.0,
        ..Pad::default()
    };
    step(&mut s, &p, &pad, 30, &mut ev);
    let g = s.grind.as_ref().expect("grinding");
    let want = if s.mirrored() { "BSGRAB" } else { "FSGRAB" };
    assert_eq!(g.grab.map(|(n, _)| n), Some(want));
    step(&mut s, &p, &Pad::default(), 200, &mut ev);
    let names = landed(&ev);
    let shown = if s.mirrored() { "BS GRAB" } else { "FS GRAB" };
    assert!(
        names.iter().any(|n| n.ends_with(&format!("50-50 {shown}"))),
        "{names:?}"
    );
    // A trigger only partly in is no grind grab.
    let mut s = in_air(
        &p,
        v3(0.0, 0.56, 1.0),
        Vec3::UP,
        v3(0.0, 0.0, 1.0),
        v3(0.0, -1.0, 5.0),
    );
    let mut ev = Vec::new();
    step(&mut s, &p, &Pad::default(), 2, &mut ev);
    let half = Pad {
        trig_l: 0.5,
        ..Pad::default()
    };
    step(&mut s, &p, &half, 20, &mut ev);
    assert!(s.grind.as_ref().is_some_and(|g| g.grab.is_none()));
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
        .expect("rules pack");
    g.load_skater(leak(dir.join("skater.fgpak")))
        .expect("skater pack");
    g.load_motion(leak(dir.join("motion.fgpak")))
        .expect("motion pack");
    assert!(g.graph.is_some(), "state graphs on");
    g
}

/// With the player's imported data: onto the built-in park's flat bar in a 50-50, the left trigger
/// held fully in. Skate 3's action graph turns it into the grind grab's intent (`GrindFSGrab`, or
/// `GrindBSGrab` mirrored) and its motion graph plays the grind's grab (`.../FS_50_50_FSGRAB/...`); with
/// the C-stick back the 5-0's grab; the grind is scored with the grab.
#[test]
fn imported_a_trigger_in_on_a_rail_plays_skate_3s_grind_grab() {
    let Some(dir) = data_dir() else {
        eprintln!("grind grab: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = imported(&dir);
    g.place_rider(v3(4.0, 0.0, 8.0), 0.0, 4.0);
    let idle = PadRaw {
        flags: flag::CONNECTED,
        ..Default::default()
    };
    for _ in 0..5 {
        g.update(&idle, TICK);
    }
    let s = &mut g.skater;
    s.pos = v3(4.0, 0.55, 10.3);
    s.vel = v3(0.0, -0.5, 4.0);
    s.mode = Mode::Air { time: 0.5 };
    let mut states = Vec::new();
    let mut grabs = Vec::new();
    let mut names = Vec::new();
    let mut min_height = 0.0f32;
    for i in 0..150 {
        let pad = PadRaw {
            trig_l: if i >= 10 { 255 } else { 0 },
            // The C-stick back: onto the back truck (a 5-0) in the grab.
            cstick_y: if (30..50).contains(&i) { -70 } else { 0 },
            ..idle
        };
        g.update(&pad, TICK);
        let st = g.graph.as_ref().unwrap().runner.motion_state();
        if states.last() != Some(&st) {
            states.push(st);
        }
        if let Some(gr) = g.skater.grind.as_ref().and_then(|x| x.grab) {
            grabs.push(gr.0);
        }
        if let Some(c) = g.skater.graph {
            min_height = min_height.max(c.grind_grab_min_height);
        }
        names.extend(g.events.iter().filter_map(|e| match e {
            Event::Landed { name, .. } => Some(name.clone()),
            _ => None,
        }));
    }
    let grab = if g.skater.mirrored() {
        "BSGRAB"
    } else {
        "FSGRAB"
    };
    assert!(
        states
            .iter()
            .any(|s| s.starts_with("OnBoard/Grinding/") && s.contains("GRAB/")),
        "never in a grind grab: {states:?}"
    );
    assert!(
        states
            .iter()
            .any(|s| s.contains("_5_O_") && s.ends_with("/GrabCycle")),
        "never in a 5-0 grab's cycle: {states:?}"
    );
    // The 5-0 grab's cycle names the grab for the physics (its ScoringTrick), and its free end is held
    // up (`PhysGrindGrabMinHeight`).
    assert!(
        grabs.contains(&grab) || grabs.contains(&"NOSEGRAB"),
        "{grabs:?}"
    );
    assert!((min_height - 0.2).abs() < 1e-4, "{min_height}");
    assert!(
        names
            .iter()
            .any(|n| n.contains(" 5-0 ") && n.contains("GRAB")),
        "{names:?} {grabs:?} {states:?}"
    );
}
