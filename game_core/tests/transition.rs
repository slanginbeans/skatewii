//! Transition skating: dropping in on foot, coping stalls (lip tricks) and hand plants, on a test-only
//! quarter pipe (the built-in park has none) without imported data, and with it through Skate 3's own
//! graphs:
//!
//! - Standing at the deck's edge with the board in hand, Y sets the board's tail on the coping and the
//!   rider stands on it while Y is held; letting Y go drops him in, rolling down the transition. The stick
//!   pulled back steps away again; away from an edge Y still just gets on.
//! - Coming down onto the coping with the board's middle on it, held there, falls back in fakie: a rock to
//!   fakie (the stick to the side turns it round: a rock and roll). Both trucks on it, stopped, and back
//!   in turned down the ramp: an axle stall. Moving along the coping it is a grind.
//! - Z held riding up the transition finds the coping: the rider plants a hand on it, turns upside down
//!   over it and comes back down the ramp riding forwards (a hand plant), scored with Skate 3's points;
//!   the C-stick and the push buttons pick its variant. No coping, no hand plant.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it):
//!   Skate 3's graphs go through `Mount/Ground/IntoDropIn` into `OnBoard/Grinding/DroppingIn`, play their
//!   grind states on the coping for the stalls, and their `HandPlantGround` and `HandPlantAir` states.

mod common;

use std::path::PathBuf;

use game_core::Game;
use game_core::flick::FlickOut;
use game_core::input::{Pad, PadRaw, btn, flag};
use game_core::math::{Vec3, v3};
use game_core::park::{Axis, Park, Shape};
use game_core::rails::Rail;
use game_core::skater::dropin::Phase;
use game_core::skater::stall::Stall;
use game_core::skater::{DECK_HEIGHT, Event, Skater};

const TICK: f32 = 1.0 / 60.0;
/// The quarter pipe: a 3 m radius transition from the flat at z = 0 up to 75 degrees in 30 strips, its
/// coping along x at [`coping`] (the deck behind it).
const RADIUS: f32 = 3.0;
const TOP: f32 = 75.0;
const STRIPS: usize = 30;

fn coping() -> Vec3 {
    let th = TOP.to_radians();
    v3(0.0, RADIUS * (1.0 - th.cos()), RADIUS * th.sin())
}

/// Flat ground towards -z, the transition, the deck from the coping to z = 9.
fn pipe() -> Park {
    let mut shapes = Vec::new();
    let pt = |k: f32| {
        let th = (k * TOP).to_radians();
        (RADIUS * th.sin(), RADIUS * (1.0 - th.cos()))
    };
    for i in 0..STRIPS {
        let (z0, y0) = pt(i as f32 / STRIPS as f32);
        let (z1, y1) = pt((i + 1) as f32 / STRIPS as f32);
        shapes.push(Shape::Ramp {
            min: v3(-8.0, 0.0, z0),
            max: v3(8.0, 0.0, z1),
            axis: Axis::Z,
            h0: y0,
            h1: y1,
            rgba: 0,
        });
    }
    let c = coping();
    shapes.push(Shape::Block {
        min: v3(-8.0, 0.0, c.z),
        max: v3(8.0, c.y, 9.0),
        rgba: 0,
    });
    let mut park = Park {
        shapes,
        half_size: 30.0,
        wall_height: 6.0,
        spawn: v3(0.0, 0.0, -10.0),
        spawn_yaw: 0.0,
        rails: Vec::new(),
    };
    let rail = Rail::new(vec![v3(-8.0, c.y, c.z), v3(8.0, c.y, c.z)]).classified(&park);
    park.rails = vec![rail];
    park
}

#[derive(Clone, Copy, Default)]
struct In {
    stick: [i8; 2],
    cstick: [i8; 2],
    buttons: u32,
}

fn pad(i: In) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        stick_x: i.stick[0],
        stick_y: i.stick[1],
        cstick_x: i.cstick[0],
        cstick_y: i.cstick[1],
        buttons: i.buttons,
        ..Default::default()
    }
}

fn run(g: &mut Game, ticks: usize, i: In, ev: &mut Vec<Event>) {
    for _ in 0..ticks {
        g.update(&pad(i), TICK);
        ev.extend(g.events.iter().cloned());
    }
}

const NONE: In = In {
    stick: [0, 0],
    cstick: [0, 0],
    buttons: 0,
};
const Y: In = In {
    stick: [0, 0],
    cstick: [0, 0],
    buttons: btn::Y,
};
const Z: In = In {
    stick: [0, 0],
    cstick: [0, 0],
    buttons: btn::Z,
};

fn landed(ev: &[Event]) -> Vec<String> {
    ev.iter()
        .filter_map(|e| match e {
            Event::Landed { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// On foot on the deck `back` metres behind the coping, facing it, the board in hand.
fn at_the_edge(g: &mut Game, back: f32) {
    let c = coping();
    g.park = pipe();
    g.place_rider(v3(0.0, c.y, c.z + back), std::f32::consts::PI, 0.0);
    let mut ev = Vec::new();
    run(g, 30, NONE, &mut ev);
    if g.graph.is_some() {
        run(g, 1, Y, &mut ev);
    } else {
        common::get_off(g);
    }
    run(g, 120, NONE, &mut ev);
    assert!(g.skater.walking(), "off the board: {ev:?}");
    assert!(g.skater.possession.held(), "the board in hand");
}

#[test]
fn y_at_the_edge_sets_the_board_on_the_coping_and_letting_go_drops_in() {
    let mut g = Game::new();
    at_the_edge(&mut g, 1.2);
    let e = g.skater.edge.expect("the edge ahead");
    assert!(
        (e.dist - 1.2).abs() < 0.15 && e.transition && e.rail == Some(0),
        "{e:?}"
    );
    let mut ev = Vec::new();
    // Y held: up to the edge and standing on the board there (the motion graph's `IntoDropIn/Cycle`).
    g.skater.graph.drop_in_stand = true;
    run(&mut g, 90, Y, &mut ev);
    g.skater.graph.drop_in_stand = false;
    assert!(ev.contains(&Event::DropInSet), "{ev:?}");
    let d = g.skater.drop_in.expect("dropping in");
    assert_eq!(d.phase, Phase::Stand);
    let c = coping();
    assert!(
        (g.skater.pos.z - c.z).abs() < 0.3,
        "at the edge: {:?}",
        g.skater.pos
    );
    assert!(g.skater.walking());
    // Let go: on the board with its tail on the coping, then down the transition.
    let (mut on_coping, mut riding) = (false, false);
    for _ in 0..240 {
        run(&mut g, 1, NONE, &mut ev);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        on_coping |= g.skater.drop_in.is_some_and(|d| d.dropping())
            && g.skater.rolling()
            && g.skater.speed == 0.0;
        riding |= g.skater.rolling() && g.skater.drop_in.is_none();
    }
    assert!(on_coping, "held on the coping first");
    assert!(ev.contains(&Event::DroppedIn), "{ev:?}");
    assert!(riding, "rolled away");
    assert!(
        g.skater.pos.y < 0.5 && g.skater.speed > 3.0,
        "down in the flat, riding forwards: {:?} {}",
        g.skater.pos,
        g.skater.speed
    );
}

#[test]
fn the_stick_pulled_back_at_the_edge_steps_away() {
    let mut g = Game::new();
    at_the_edge(&mut g, 0.8);
    let mut ev = Vec::new();
    g.skater.graph.drop_in_stand = true;
    run(&mut g, 60, Y, &mut ev);
    g.skater.graph.drop_in_stand = false;
    assert!(ev.contains(&Event::DropInSet), "{ev:?}");
    // The camera looks along the rider's facing: the stick pulled back is away from the edge.
    let back = In {
        stick: [0, -100],
        ..Y
    };
    run(&mut g, 2, back, &mut ev);
    assert!(ev.contains(&Event::DropInCancelled), "{ev:?}");
    assert!(g.skater.drop_in.is_none());
    run(&mut g, 60, back, &mut ev);
    assert!(g.skater.walking(), "still on foot");
    assert!(
        g.skater.pos.z > coping().z + 0.5,
        "walked back from the edge: {:?}",
        g.skater.pos
    );
    assert!(!ev.contains(&Event::DroppedIn));
}

#[test]
fn y_away_from_an_edge_still_gets_on_the_board() {
    let mut g = Game::new();
    at_the_edge(&mut g, 3.0);
    assert!(g.skater.edge.is_none_or(|e| e.dist > 1.5));
    let ev: Vec<Event> = Vec::new();
    common::get_on(&mut g);
    assert!(g.skater.drop_in.is_none());
    assert!(g.skater.rolling(), "{ev:?}");
}

/// A skater in the air with its deck's middle at `deck`, nose `fwd`, moving at `vel`.
fn in_air(p: &Park, deck: Vec3, fwd: Vec3, vel: Vec3) -> Skater {
    let mut s = Skater::spawn(p);
    s.since_spawn = 10.0;
    s.pos = deck - Vec3::UP * DECK_HEIGHT;
    s.normal = Vec3::UP;
    s.contact_n = Vec3::UP;
    s.fwd = fwd;
    s.heading = game_core::math::atan2f(fwd.x, fwd.z);
    s.vel = vel;
    s.force_state(game_core::skater::State::PhysicsAir);
    s.air_time = 0.5;
    s.states.time = 0.5;
    s
}

/// Steps until rolling in the flat again (or `ticks` run out), the stick at `stick` while on the coping.
fn ride_out(s: &mut Skater, p: &Park, stick: [f32; 2], ticks: usize) -> Vec<Event> {
    let mut ev = Vec::new();
    for _ in 0..ticks {
        let pad = Pad {
            stick: if s.grinding() { stick } else { [0.0; 2] },
            ..Pad::default()
        };
        s.step(p, &pad, &FlickOut::default(), TICK, &mut ev);
        if s.rolling() && s.pos.y < 0.3 {
            break;
        }
    }
    ev
}

#[test]
fn the_boards_middle_held_on_the_coping_rocks_back_in_fakie() {
    let p = pipe();
    let c = coping();
    // Coming down onto the coping with the nose over the deck, barely moving along it.
    let rock = || {
        in_air(
            &p,
            v3(0.0, c.y + 0.08, c.z),
            v3(0.0, 0.0, 1.0),
            v3(0.0, -0.8, 0.3),
        )
    };
    let mut s = rock();
    let mut ev = Vec::new();
    s.step(&p, &Pad::default(), &FlickOut::default(), TICK, &mut ev);
    let g = s.grind.clone().expect("on the coping");
    assert!(g.coping, "the rail is a coping: {g:?}");
    ev.extend(ride_out(&mut s, &p, [0.0; 2], 300));
    assert_eq!(s.stalled, Some(Stall::RockToFakie));
    assert_eq!(landed(&ev), ["ROCK TO FAKIE"], "{ev:?}");
    assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    assert!(
        s.rolling() && s.pos.y < 0.3 && s.speed < -3.0,
        "back down the ramp tail first: {:?} {:?} {}",
        s.state(),
        s.pos,
        s.speed
    );
    // The stick held to the side as it falls back turns it round: a rock and roll, riding forwards.
    let mut s = rock();
    let ev = ride_out(&mut s, &p, [1.0, 0.0], 300);
    assert_eq!(s.stalled, Some(Stall::RockAndRoll));
    let names = landed(&ev);
    assert!(
        names.len() == 1 && names[0].ends_with(" ROCK N ROLL"),
        "{ev:?}"
    );
    assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    assert!(s.speed > 3.0 && s.pos.y < 0.3, "{:?} {}", s.pos, s.speed);
}

#[test]
fn both_trucks_held_on_the_coping_are_an_axle_stall_back_down_the_ramp() {
    let p = pipe();
    let c = coping();
    // Along the coping, both trucks coming down onto it.
    let mut s = in_air(
        &p,
        v3(0.0, c.y + 0.06, c.z),
        v3(1.0, 0.0, 0.0),
        v3(0.1, -0.8, 0.0),
    );
    let ev = ride_out(&mut s, &p, [0.0; 2], 300);
    assert_eq!(s.stalled, Some(Stall::Axle));
    let names = landed(&ev);
    assert!(
        names.len() == 1 && names[0].ends_with(" AXLE STALL"),
        "{ev:?}"
    );
    assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    assert!(
        s.rolling() && s.pos.y < 0.3 && s.speed > 3.0 && !s.fakie,
        "turned back down and riding forwards: {:?} {:?} {}",
        s.state(),
        s.pos,
        s.speed
    );
    // Moving along it fast is a grind, not a stall.
    let mut s = in_air(
        &p,
        v3(-6.0, c.y + 0.06, c.z),
        v3(1.0, 0.0, 0.0),
        v3(4.0, -0.8, 0.0),
    );
    let ev = ride_out(&mut s, &p, [0.0; 2], 300);
    assert_eq!(s.stalled, None, "{ev:?}");
    assert!(landed(&ev).iter().any(|n| n.ends_with("50-50")), "{ev:?}");
}

#[test]
fn z_riding_up_the_ramp_plants_a_hand_on_the_coping_and_comes_back_down() {
    let mut g = Game::new();
    g.park = pipe();
    g.place_rider(v3(0.0, 0.0, -3.0), 0.0, 8.0);
    let c = coping();
    let mut ev = Vec::new();
    let (mut found, mut upside_down, mut top) = (false, false, f32::MIN);
    // The motion graph attaching the world grab (Z) and committing its hand plant.
    g.skater.graph.grab_world = true;
    g.skater.graph.handplant_committed = true;
    for _ in 0..360 {
        run(&mut g, 1, Z, &mut ev);
        found |= g.skater.handplant.found;
        if g.skater.hand_plant.is_some() {
            upside_down |= g.skater.normal.y < 0.0;
            top = top.max(g.skater.pos.y);
            // The free look and the markers leave Z to it.
            assert!(g.skater.handplanting());
        }
        if ev.contains(&Event::HandReleased) && g.skater.rolling() && g.skater.pos.y < 0.3 {
            break;
        }
    }
    assert!(found && ev.contains(&Event::HandPlanted), "{ev:?}");
    assert!(upside_down, "inverted over the hand");
    assert!(
        top > c.y + 0.8,
        "the board over the rider over the coping: {top}"
    );
    assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    let names = landed(&ev);
    assert!(
        names
            .first()
            .is_some_and(|n| n == "BS HANDPLANT" || n == "FS HANDPLANT"),
        "{ev:?}"
    );
    // Skate 3's points for it (100, held past its 0.2 s completion delay).
    let points = ev.iter().find_map(|e| match e {
        Event::Landed { name, points, .. } if name.ends_with("HANDPLANT") => Some(*points),
        _ => None,
    });
    assert_eq!(points, Some(100), "{ev:?}");
    assert!(
        g.skater.rolling() && g.skater.speed > 3.0 && !g.skater.fakie,
        "riding back down forwards: {:?} {}",
        g.skater.state(),
        g.skater.speed
    );
}

#[test]
fn z_up_a_ramp_without_a_coping_is_no_hand_plant() {
    let mut p = pipe();
    p.rails.clear();
    let mut s = Skater::spawn(&p);
    s.since_spawn = 10.0;
    s.pos = v3(0.0, 0.0, -3.0);
    s.speed = 8.0;
    s.vel = v3(0.0, 0.0, 8.0);
    let mut ev = Vec::new();
    let z = Pad {
        held: btn::Z,
        ..Pad::default()
    };
    for _ in 0..120 {
        s.step(&p, &z, &FlickOut::default(), TICK, &mut ev);
        assert!(!s.handplant.found && s.hand_plant.is_none());
    }
    assert!(!ev.contains(&Event::HandPlanted));
}

// ---- With the player's imported data.

fn data_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FG_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist/apps/flatground/data")
        });
    ["rules.fgpak", "skater.fgpak", "motion.fgpak"]
        .iter()
        .all(|f| dir.join(f).is_file())
        .then_some(dir)
}

fn leak(p: PathBuf) -> &'static [u8] {
    Box::leak(std::fs::read(p).unwrap().into_boxed_slice())
}

fn imported(dir: &std::path::Path) -> Game {
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
    g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
    g
}

fn path(g: &Game) -> String {
    g.graph.as_ref().unwrap().runner.motion_state()
}

#[test]
fn imported_graphs_drop_in_through_their_drop_in_states() {
    let Some(dir) = data_dir() else {
        eprintln!("transition: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = imported(&dir);
    at_the_edge(&mut g, 1.2);
    let mut ev = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for i in 0..400 {
        run(&mut g, 1, if i < 120 { Y } else { NONE }, &mut ev);
        let p = path(&g);
        assert!(!p.starts_with("WipeOut"), "{p}");
        if seen.last() != Some(&p) {
            seen.push(p);
        }
    }
    let at = |part: &str| seen.iter().position(|p| p.contains(part));
    let (into, out, grinding) = (
        at("Mount/Ground/IntoDropIn/Into"),
        at("Mount/Ground/OutOfDropIn"),
        at("OnBoard/Grinding/DroppingIn"),
    );
    assert!(
        into.is_some() && out > into && grinding > out,
        "Skate 3's drop-in states in order: {seen:?}"
    );
    assert!(at("IntoDropIn/Cycle").is_some(), "{seen:?}");
    assert!(
        ev.contains(&Event::DroppedIn) && !ev.contains(&Event::Bailed),
        "{ev:?}"
    );
    assert!(
        g.skater.rolling() && g.skater.pos.y < 0.3,
        "{:?} {:?}",
        g.skater.state(),
        g.skater.pos
    );
}

#[test]
fn imported_graphs_play_their_grind_states_for_the_stalls() {
    let Some(dir) = data_dir() else {
        eprintln!("transition: no imported data (FG_DATA): skipped");
        return;
    };
    let c = coping();
    for (stall, fwd, vel, states) in [
        (
            Stall::RockToFakie,
            v3(0.0, 0.0, 1.0),
            v3(0.0, -0.8, 0.3),
            ["_LIP", "_BOARD"],
        ),
        (
            Stall::Axle,
            v3(1.0, 0.0, 0.0),
            v3(0.1, -0.8, 0.0),
            ["_50_50", "_50_50"],
        ),
    ] {
        let mut g = imported(&dir);
        g.park = pipe();
        g.place_rider(v3(0.0, 0.0, -6.0), 0.0, 0.0);
        let mut ev = Vec::new();
        run(&mut g, 20, NONE, &mut ev);
        let s = &mut g.skater;
        s.pos = v3(0.0, c.y + 0.06, c.z) - Vec3::UP * DECK_HEIGHT;
        s.normal = Vec3::UP;
        s.contact_n = Vec3::UP;
        s.fwd = fwd;
        s.heading = game_core::math::atan2f(fwd.x, fwd.z);
        s.vel = vel;
        s.force_state(game_core::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        let mut grinding = Vec::new();
        for _ in 0..150 {
            run(&mut g, 1, NONE, &mut ev);
            let p = path(&g);
            assert!(!p.starts_with("WipeOut"), "{p}");
            if g.skater.grinding() {
                grinding.push(p);
            }
        }
        assert_eq!(g.skater.stalled, Some(stall), "{ev:?}");
        assert!(
            grinding.len() > 2
                && grinding.iter().skip(2).all(|p| {
                    p.starts_with("OnBoard/Grinding/") && states.iter().any(|s| p.contains(s))
                }),
            "{stall:?}: {grinding:?}"
        );
        let points = ev.iter().find_map(|e| match e {
            Event::Landed { points, .. } => Some(*points),
            _ => None,
        });
        assert!(points.is_some_and(|p| p > 0), "scored as its grind: {ev:?}");
    }
}

#[test]
fn imported_graphs_play_the_hand_plant() {
    let Some(dir) = data_dir() else {
        eprintln!("transition: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = imported(&dir);
    g.park = pipe();
    g.place_rider(v3(0.0, 0.0, -3.0), 0.0, 8.0);
    let mut ev = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..300 {
        run(&mut g, 1, Z, &mut ev);
        let p = path(&g);
        assert!(!p.starts_with("WipeOut"), "{p}");
        if seen.last() != Some(&p) {
            seen.push(p);
        }
    }
    let at = |part: &str| seen.iter().position(|p| p.contains(part));
    let (ground, air, cyc) = (
        at("Riding/HandPlantGround/"),
        at("OnBoard/HandPlantAir/"),
        at("HandPlantAir/FS/Cyc").or(at("HandPlantAir/BS/Cyc")),
    );
    assert!(
        ground.is_some() && air > ground && cyc > air,
        "Skate 3's hand plant states: {seen:?}"
    );
    let scored = ev.iter().find_map(|e| match e {
        Event::Landed { name, points, .. } if name.contains("HANDPLANT") => Some(*points),
        _ => None,
    });
    assert!(scored.is_some_and(|p| p >= 100), "Skate 3's points: {ev:?}");
    assert!(!ev.contains(&Event::Bailed), "{ev:?}");
}
