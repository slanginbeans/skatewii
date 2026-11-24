//! Foot plants (Skate 3's boneless, fastplant and no-comply on the ground, the foot plant in the air off
//! a wall: `skater::plant`), on synthetic parks and without imported data:
//!
//! - A trigger fully in on the ground (Skate 3's ground grab) and a push button plant a foot: A (the
//!   right foot's push) a boneless, B a fastplant. The rider jumps off the planted foot on the clips'
//!   timing with Skate 3's FootJump launch (3 m/s up on flat), higher than the board alone would go, and
//!   the landing scores the plant.
//! - A flick while a push's foot is still coming down is a no-comply.
//! - In the air, a grab and a push button (the one-foot grab) reach the free foot for what the flight is
//!   about to meet: against a wall it plants on the wall's face, the rider swings up over the foot and
//!   flies off it (a wallplant), scored as Skate 3 names it.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it):
//!   Skate 3's graphs play their boneless and no-comply states and the physics jumps off the planted foot
//!   on their clips' `FOOTJUMP`; a one-foot grab against a wall plays their foot plant states.

use std::path::PathBuf;

use game_core::Game;
use game_core::flick::FlickOut;
use game_core::gesture::Flick;
use game_core::input::{Pad, PadRaw, btn, flag};
use game_core::math::{Vec3, v3};
use game_core::park::{Park, Shape};
use game_core::skater::plant::PlantKind;
use game_core::skater::{DECK_HEIGHT, Event, Mode, Skater};
use game_core::tuning::plant::PlantTuning;

const TICK: f32 = 1.0 / 60.0;

fn flat() -> Park {
    Park {
        shapes: Vec::new(),
        half_size: 60.0,
        wall_height: 2.0,
        spawn: v3(0.0, 0.0, -30.0),
        spawn_yaw: 0.0,
        rails: Vec::new(),
    }
}

/// Flat ground with a wall 2.5 m high across the way from z = `at`.
fn wall_park(at: f32) -> Park {
    Park {
        shapes: vec![Shape::Block {
            min: v3(-6.0, 0.0, at),
            max: v3(6.0, 2.5, at + 3.0),
            rgba: 0,
        }],
        ..flat()
    }
}

/// Rolling along +z at `speed`, settled.
fn rolling(p: &Park, speed: f32) -> Skater {
    let mut s = Skater::spawn(p);
    s.since_spawn = 10.0;
    s.put_at(p, v3(0.0, 0.0, -30.0), 0.0, speed);
    let mut ev = Vec::new();
    step(
        &mut s,
        p,
        &Pad::default(),
        &FlickOut::default(),
        20,
        &mut ev,
    );
    s
}

fn step(s: &mut Skater, p: &Park, pad: &Pad, flick: &FlickOut, ticks: usize, ev: &mut Vec<Event>) {
    for _ in 0..ticks {
        s.step(p, pad, flick, TICK, ev);
    }
}

fn pad(trig_l: f32, held: u32, pressed: u32) -> Pad {
    Pad {
        trig_l,
        held,
        pressed,
        ..Pad::default()
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

/// Rides until landed (or `ticks` run out): the highest the board got (over the flat ground at 0).
fn ride_out(s: &mut Skater, p: &Park, pad: &Pad, ticks: usize, ev: &mut Vec<Event>) -> f32 {
    let mut top = s.pos.y;
    for _ in 0..ticks {
        s.step(p, pad, &FlickOut::default(), TICK, ev);
        top = top.max(s.pos.y);
        if s.mode == Mode::Rolling && ev.iter().any(|e| matches!(e, Event::Landed { .. })) {
            break;
        }
    }
    top
}

#[test]
fn a_ground_grab_and_a_push_is_a_boneless_that_jumps_off_the_planted_foot() {
    let p = flat();
    let mut s = rolling(&p, 5.0);
    let mut ev = Vec::new();
    // The left trigger fully in and A: the goofy rider's front hand, the back foot's push.
    let held = pad(1.0, btn::A, btn::A);
    step(&mut s, &p, &held, &FlickOut::default(), 1, &mut ev);
    assert!(s.ground_plant.is_some(), "{ev:?}");
    assert!(
        s.push.is_none(),
        "the push button planted the foot instead of pushing"
    );
    let hold = pad(1.0, 0, 0);
    let mut ticks = 1;
    while !ev.iter().any(|e| matches!(e, Event::FootJump(_))) {
        step(&mut s, &p, &hold, &FlickOut::default(), 1, &mut ev);
        ticks += 1;
        assert!(ticks < 60, "{ev:?}");
        if s.mode == Mode::Rolling {
            assert!(s.speed > 4.5, "rolls on while planted: {}", s.speed);
        }
    }
    // On Skate 3's timing: its graphs' boneless from the press to the clips' FOOTJUMP, 0.47 s.
    assert!((27..=29).contains(&ticks), "{ticks}");
    assert!(
        ev.contains(&Event::FootJump(PlantKind::FsBoneless)),
        "{ev:?}"
    );
    assert!(matches!(s.mode, Mode::Air { .. }));
    // Skate 3's FootJump on flat: 3 m/s up the normal, the speed along the ground scaled.
    assert!((s.vel.y - 3.0).abs() < 0.05, "{:?}", s.vel);
    assert!(s.vel.z > 3.5 && s.vel.z < 5.0, "{:?}", s.vel);
    let top = ride_out(&mut s, &p, &Pad::default(), 120, &mut ev);
    // 3 m/s up climbs about 0.46 m: the plant, not the board's pop.
    assert!(top > 0.4 && top < 0.55, "{top}");
    assert_eq!(s.mode, Mode::Rolling, "{ev:?}");
    assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    let names = landed(&ev);
    assert!(names.iter().any(|n| n == "FS BONELESS"), "{names:?}");
}

#[test]
fn b_with_the_ground_grab_is_a_fastplant_and_without_it_a_push() {
    let p = flat();
    let mut s = rolling(&p, 4.0);
    let mut ev = Vec::new();
    step(
        &mut s,
        &p,
        &pad(1.0, btn::B, btn::B),
        &FlickOut::default(),
        1,
        &mut ev,
    );
    for _ in 0..40 {
        if s.is_airborne() {
            break;
        }
        step(
            &mut s,
            &p,
            &pad(1.0, 0, 0),
            &FlickOut::default(),
            1,
            &mut ev,
        );
    }
    assert!(
        ev.contains(&Event::FootJump(PlantKind::FsFastplant)),
        "{ev:?}"
    );
    ride_out(&mut s, &p, &Pad::default(), 120, &mut ev);
    assert!(landed(&ev).iter().any(|n| n == "FS FASTPLANT"), "{ev:?}");

    // The trigger only partly in is a crouch, not a ground grab: B pushes.
    let mut s = rolling(&p, 4.0);
    let mut ev = Vec::new();
    step(
        &mut s,
        &p,
        &pad(0.5, btn::B, btn::B),
        &FlickOut::default(),
        1,
        &mut ev,
    );
    assert!(s.ground_plant.is_none());
    step(
        &mut s,
        &p,
        &pad(0.5, btn::B, 0),
        &FlickOut::default(),
        40,
        &mut ev,
    );
    assert!(
        !ev.iter().any(|e| matches!(e, Event::FootJump(_))),
        "{ev:?}"
    );
    assert_eq!(s.mode, Mode::Rolling);
}

#[test]
fn a_flick_while_the_push_foot_comes_down_is_a_no_comply() {
    let p = flat();
    let mut s = rolling(&p, 4.0);
    let mut ev = Vec::new();
    step(
        &mut s,
        &p,
        &pad(0.0, btn::A, btn::A),
        &FlickOut::default(),
        1,
        &mut ev,
    );
    assert!(s.push.is_some_and(|p| p.stage().0 == 0), "{:?}", s.push);
    let flick = FlickOut {
        flick: Some(Flick::OLLIE),
        strength: 1.0,
        ..FlickOut::default()
    };
    step(&mut s, &p, &pad(0.0, btn::A, 0), &flick, 1, &mut ev);
    assert!(
        !ev.iter().any(|e| matches!(e, Event::Popped(_))),
        "a no-comply, not an ollie: {ev:?}"
    );
    assert_eq!(s.ground_plant.map(|g| g.kind), Some(PlantKind::NoComply));
    step(
        &mut s,
        &p,
        &Pad::default(),
        &FlickOut::default(),
        20,
        &mut ev,
    );
    assert!(ev.contains(&Event::FootJump(PlantKind::NoComply)), "{ev:?}");
    let top = ride_out(&mut s, &p, &Pad::default(), 120, &mut ev);
    assert!(top > 0.4, "{top}");
    assert!(landed(&ev).iter().any(|n| n == "NO COMPLY"), "{ev:?}");
}

/// In the air at `deck` moving `vel`, the board level and its nose along +z.
fn in_air(p: &Park, deck: Vec3, vel: Vec3) -> Skater {
    let mut s = Skater::spawn(p);
    s.since_spawn = 10.0;
    s.pos = deck - Vec3::UP * DECK_HEIGHT;
    s.normal = Vec3::UP;
    s.contact_n = Vec3::UP;
    s.fwd = v3(0.0, 0.0, 1.0);
    s.heading = 0.0;
    s.vel = vel;
    s.mode = Mode::Air { time: 0.3 };
    s
}

#[test]
fn a_one_foot_grab_at_a_wall_plants_on_it_and_flies_off_it() {
    let p = wall_park(3.0);
    // Coming up at the wall at 5 m/s, rising.
    let mut s = in_air(&p, v3(0.0, 0.6, 1.6), v3(0.0, 5.0, 5.0));
    let mut ev = Vec::new();
    // Grab (left trigger) and A: the right foot off the board.
    step(
        &mut s,
        &p,
        &pad(0.6, btn::A, btn::A),
        &FlickOut::default(),
        1,
        &mut ev,
    );
    let one_foot = pad(0.6, btn::A, 0);
    let mut planted = None;
    for _ in 0..60 {
        let before = s.vel;
        step(&mut s, &p, &one_foot, &FlickOut::default(), 1, &mut ev);
        if planted.is_none()
            && let Some(a) = s.air_plant
        {
            planted = Some((before, a));
        }
        if ev.contains(&Event::FootReleased) {
            break;
        }
    }
    let (at, plant) = planted.unwrap_or_else(|| panic!("planted: {ev:?} {:?}", s.footplant));
    assert!(ev.contains(&Event::FootPlanted), "{ev:?}");
    // On the wall's face, below the rider.
    assert!(plant.normal.z < -0.9, "{plant:?}");
    assert!((plant.anchor.z - 3.0).abs() < 0.05, "{plant:?}");
    assert!(ev.contains(&Event::FootReleased), "{ev:?}");
    // The swing carried the rider up over the foot, and he flies off it up and back away from the wall.
    assert!(plant.curve[3].y > plant.curve[0].y + 0.1, "{plant:?}");
    assert!(s.vel.y > 0.3, "{:?} (planted at {at:?})", s.vel);
    assert!(s.vel.z < -1.0, "{:?} (planted at {at:?})", s.vel);
    assert!(s.pos.z < 3.0, "{:?}", s.pos);
    let names = {
        ride_out(&mut s, &p, &Pad::default(), 240, &mut ev);
        landed(&ev)
    };
    assert!(!ev.contains(&Event::Bailed), "{ev:?} {:?}", s.bail_reason);
    assert!(
        names.iter().any(|n| n.contains("FSGRAB FS FOOTPLANT")),
        "{names:?}"
    );
}

// ---- With the player's imported data.

fn data_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FG_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist/apps/flatground/data")
        });
    dir.join("motion.fgpak").exists().then_some(dir)
}

fn leak(p: PathBuf) -> &'static [u8] {
    Box::leak(std::fs::read(p).unwrap().into_boxed_slice())
}

fn imported(dir: &std::path::Path) -> Game {
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .expect("rules pack");
    g.load_skater(leak(dir.join("skater.fgpak")))
        .expect("skater pack");
    g.load_motion(leak(dir.join("motion.fgpak")))
        .expect("motion pack");
    g
}

fn raw(stick: [i8; 2], trig_l: u8, buttons: u32) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        stick_x: stick[0],
        stick_y: stick[1],
        trig_l,
        buttons,
        ..Default::default()
    }
}

const LANE: Vec3 = Vec3 {
    x: -7.0,
    y: 0.0,
    z: -18.0,
};

#[test]
fn imported_graphs_play_the_boneless_and_jump_off_the_planted_foot() {
    let Some(dir) = data_dir() else {
        eprintln!("foot plants: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = imported(&dir);
    // The built-in foot plant tuning is Skate 3's own (the rules pack carries the same numbers).
    assert_eq!(g.skater.tuning.plant, PlantTuning::BUILT_IN);
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    g.place_rider(LANE, 0.0, 4.0);
    for _ in 0..40 {
        g.update(&raw([0, 0], 0, 0), TICK);
    }
    // The left trigger fully in (a ground grab: crouched into it), then A.
    for _ in 0..40 {
        g.update(&raw([0, 0], 255, 0), TICK);
    }
    let mut states = Vec::new();
    let mut jumped = false;
    let mut lifted = false;
    let mut names = Vec::new();
    for i in 0..180 {
        let b = if i < 6 { btn::A } else { 0 };
        // Let go of the trigger once off the ground.
        let t = if jumped { 0 } else { 255 };
        g.update(&raw([0, 0], t, b), TICK);
        names.extend(landed(&g.events));
        let p = path(&g);
        if states.last() != Some(&p) {
            states.push(p);
        }
        lifted |= g.skater.ground_plant.is_some_and(|p| p.lifted);
        jumped |= g.events.iter().any(|e| matches!(e, Event::FootJump(_)));
        if jumped && g.skater.mode == Mode::Rolling {
            break;
        }
    }
    assert!(
        states
            .iter()
            .any(|s| s.contains("/PushTrick/FSBoneless/") || s.contains("/PushTrick/BSBoneless/")),
        "{states:#?}"
    );
    assert!(lifted, "the clips' ANIMSKATEBOARD: {states:#?}");
    assert!(jumped, "the clips' FOOTJUMP: {states:#?}");
    assert!(
        names.iter().any(|n| n.contains("BONELESS")),
        "{names:?} {states:#?}"
    );
}

#[test]
fn imported_graphs_play_the_no_comply() {
    let Some(dir) = data_dir() else {
        eprintln!("foot plants: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = imported(&dir);
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    g.place_rider(LANE, 0.0, 4.0);
    for _ in 0..40 {
        g.update(&raw([0, 0], 0, 0), TICK);
    }
    let mut states = Vec::new();
    let mut kinds = Vec::new();
    for i in 0..120 {
        // A push, and an ollie's flick (C-stick back, then forward) while its foot comes down.
        let b = if i < 4 { btn::A } else { 0 };
        let c = match i {
            2..=5 => -92,
            6..=9 => 92,
            _ => 0,
        };
        let mut r = raw([0, 0], 0, b);
        r.cstick_y = c;
        g.update(&r, TICK);
        let p = path(&g);
        if states.last() != Some(&p) {
            states.push(p);
        }
        for e in &g.events {
            if let Event::FootJump(k) = e {
                kinds.push(*k);
            }
        }
    }
    assert!(
        states.iter().any(|s| s.contains("/PushTrick/NoComply/")),
        "{states:#?}"
    );
    assert_eq!(kinds, vec![PlantKind::NoComply], "{states:#?}");
}

#[test]
fn imported_graphs_play_the_foot_plant_against_a_wall() {
    let Some(dir) = data_dir() else {
        eprintln!("foot plants: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = imported(&dir);
    g.park = wall_park(3.0);
    g.place_rider(v3(0.0, 0.0, -6.0), 0.0, 4.0);
    for _ in 0..20 {
        g.update(&raw([0, 0], 0, 0), TICK);
    }
    let s = &mut g.skater;
    s.pos = v3(0.0, 0.3, 0.6);
    s.vel = v3(0.0, 5.0, 5.0);
    s.mode = Mode::Air { time: 0.3 };
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    let mut states = Vec::new();
    let mut planted = false;
    let mut released = false;
    let mut names = Vec::new();
    for i in 0..180 {
        // A grab (the left trigger), and A for the one-foot grab.
        let b = if (6..60).contains(&i) { btn::A } else { 0 };
        g.update(&raw([0, 0], if i < 60 { 200 } else { 0 }, b), TICK);
        let p = path(&g);
        if states.last() != Some(&p) {
            states.push(p);
        }
        planted |= g.events.contains(&Event::FootPlanted);
        released |= g.events.contains(&Event::FootReleased);
        names.extend(landed(&g.events));
        if released && g.skater.mode == Mode::Rolling {
            break;
        }
    }
    assert!(
        states.iter().any(|s| s.contains("/OneFootAir/")),
        "{states:#?}"
    );
    assert!(
        states.iter().any(|s| s.contains("/Footplant/Into")),
        "{states:#?}"
    );
    assert!(planted && released, "{states:#?}");
    assert!(
        states.iter().any(|s| s.contains("/Footplant/Absorb")),
        "IsFootPlanting: {states:#?}"
    );
    // Scored by the graph's foot plant name (`ScoringGrabs`).
    assert!(
        names.iter().any(|n| n.contains("FOOTPLANT")),
        "{names:?} {:?} {states:#?}",
        g.skater.bail_reason
    );
}
