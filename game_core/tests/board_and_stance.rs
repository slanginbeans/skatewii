//! The board's height and the rider's stance changes.
//!
//! - Riding fakie, the stick turns the travel its own way (Skate 3's `FakieTurn`).
//! - Turning round from fakie to switch turns nothing in the world: the board's frame turns end for end,
//!   and what is drawn (PAL50's blend between ticks included) stays where it was.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; the tests that need it
//!   say so and pass without it): the imported board's wheels rest on the ground (flat, a bank, a ramp's
//!   kink, landing) and its trucks' hangers on a rail in a 50-50; through the auto switch and a push from fakie
//!   the drawn board and the rider's chest turn no more than a few degrees a tick.

use std::path::PathBuf;

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::{Vec3, Xform, atan2f, v3, wrap_angle};
use game_core::park::Terrain;
use game_core::skater::{DECK_HEIGHT, Event, Mode};

const TICK: f32 = 1.0 / 60.0;

fn pad(stick_x: i8, buttons: u32) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        stick_x,
        buttons,
        ..Default::default()
    }
}

/// Degrees between two directions seen from above.
fn yaw_step(a: Vec3, b: Vec3) -> f32 {
    wrap_angle(atan2f(b.x, b.z) - atan2f(a.x, a.z))
        .to_degrees()
        .abs()
}

/// Degrees between two lines seen from above (a board's long axis: nose and tail look alike).
fn axis_step(a: Vec3, b: Vec3) -> f32 {
    let d = yaw_step(a, b);
    d.min(180.0 - d)
}

/// How far the travel turns (rad, + left) riding at `speed` with the stick at `stick_x` for a second.
fn travel_turn(speed: f32, stick_x: i8) -> (f32, bool) {
    let mut g = Game::new();
    g.place_rider(v3(0.0, 0.0, -8.0), 0.0, speed);
    for _ in 0..20 {
        g.skater.speed = speed;
        g.update(&pad(0, 0), TICK);
    }
    let fakie = g.skater.fakie;
    let y0 = g.skater.travel_yaw();
    for _ in 0..60 {
        g.update(&pad(stick_x, 0), TICK);
    }
    (wrap_angle(g.skater.travel_yaw() - y0), fakie)
}

#[test]
fn riding_fakie_the_stick_turns_the_travel_its_own_way() {
    // Positive yaw turns towards +x: to the left, seen from behind the travel.
    let (forward, _) = travel_turn(3.0, -100);
    let (fakie, was_fakie) = travel_turn(-3.0, -100);
    assert!(was_fakie);
    assert!(
        forward > 0.1,
        "stick left turns left rolling forwards: {forward}"
    );
    assert!(
        fakie > 0.1,
        "stick left turns the travel left riding fakie too: {fakie}"
    );
    let (fakie_right, _) = travel_turn(-3.0, 100);
    assert!(fakie_right < -0.1, "{fakie_right}");
}

/// The drawn rider's chest (toe side, mirrored with the stance) and the board's nose, as drawn.
fn drawn_placeholder(g: &Game) -> (Vec3, Vec3) {
    let x = g.imported_xform();
    let side = x.rot.cols[0];
    let chest = if g.skater.mirrored() { -side } else { side };
    (chest, x.rot.cols[2])
}

#[test]
fn pushing_from_fakie_turns_nothing_in_the_world_at_50_hz() {
    // flatground's own rules (no state graphs): A riding fakie turns the rider round to push switch.
    let mut g = Game::new();
    g.place_rider(v3(0.0, 0.0, -8.0), 0.0, -3.0);
    let dt = 1.0 / 50.0;
    for _ in 0..30 {
        g.update(&pad(0, 0), dt);
    }
    assert!(g.skater.fakie);
    let (mut chest, mut nose) = drawn_placeholder(&g);
    let mut switched = false;
    for i in 0..40 {
        g.update(&pad(0, if i < 5 { btn::A } else { 0 }), dt);
        switched |= g.events.contains(&Event::Switched);
        let (c, n) = drawn_placeholder(&g);
        assert!(
            yaw_step(chest, c) < 3.0,
            "tick {i}: chest {chest:?} -> {c:?}"
        );
        assert!(
            axis_step(nose, n) < 3.0,
            "tick {i}: board {nose:?} -> {n:?}"
        );
        (chest, nose) = (c, n);
    }
    assert!(switched && g.skater.switch && !g.skater.fakie);
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

/// The imported rider's part `name` as drawn, in the world.
fn part_points(g: &Game, name: &str) -> Vec<Vec3> {
    let r = g.rider().expect("imported rider");
    let x: Xform = g.imported_xform();
    r.character
        .parts
        .iter()
        .filter(|p| p.name == name)
        .flat_map(|p| p.out.iter())
        .map(|v| x.point(v3(v.pos[0], v.pos[1], v.pos[2])))
        .collect()
}

/// The lowest wheel's distance over the ground under it (m, along the ground's normal).
fn wheel_gap(g: &Game) -> f32 {
    let t: &dyn Terrain = g.terrain();
    part_points(g, "skatewheel")
        .into_iter()
        .map(|p| {
            let h = t.ground(p.x, p.z, p.y + 0.3);
            (p - v3(p.x, h.height, p.z)).dot(h.normal)
        })
        .fold(f32::MAX, f32::min)
}

/// The lowest wheel touches the ground (within 3 mm; a wheel's rim rolling over a ramp's kink dips a
/// few millimetres into the slope ahead, as a round wheel does).
fn assert_on_ground(g: &Game, what: &str) {
    let gap = wheel_gap(g);
    assert!(
        (-0.004..0.003).contains(&gap),
        "{what}: wheels {:.1} mm over the ground",
        gap * 1000.0
    );
}

#[test]
fn imported_wheels_rest_on_the_ground_and_trucks_on_the_rail() {
    let Some(mut g) = imported_game("wheels") else {
        return;
    };
    // Flat ground.
    g.place_rider(v3(0.0, 0.0, -12.0), 0.0, 3.0);
    for _ in 0..30 {
        g.update(&pad(0, 0), TICK);
    }
    assert_on_ground(&g, "flat ground");
    // The clips' board (the character's origin) is the deck's centre, about Skate 3's deck height over
    // the contact point (the imported wheels are a little smaller than Skate 3's physical ones).
    let deck = (g.imported_xform().pos - g.skater.pos).dot(g.skater.normal);
    assert!((deck - DECK_HEIGHT).abs() < 0.006, "deck {deck}");
    // Up the bank (16 degrees) and on the funbox's ramp, its kink onto the deck.
    g.place_rider(v3(14.8, 0.0, 0.0), core::f32::consts::FRAC_PI_2, 2.5);
    for _ in 0..12 {
        g.update(&pad(0, 0), TICK);
    }
    assert_eq!(g.skater.mode, Mode::Rolling);
    assert_on_ground(&g, "the bank");
    g.place_rider(v3(8.0, 0.0, -1.0), 0.0, 3.0);
    for i in 0..50 {
        g.update(&pad(0, 0), TICK);
        if g.skater.mode == Mode::Rolling && i % 5 == 0 {
            assert_on_ground(&g, &format!("the funbox, tick {i}"));
        }
    }
    // A manual: the clips pitch the board, the deck rises about the wheels left on the ground.
    g.place_rider(v3(0.0, 0.0, -14.0), 0.0, 4.0);
    let manual_pad = PadRaw {
        cstick_y: -((0.38 * 0.7 + 0.25) * game_core::input::C_RANGE) as i8,
        ..pad(0, 0)
    };
    let mut manualing = 0;
    for _ in 0..90 {
        g.update(&manual_pad, TICK);
        if g.skater.manual.is_some() {
            manualing += 1;
            if manualing > 20 {
                assert_on_ground(&g, &format!("a manual, tick {manualing}"));
            }
        }
    }
    assert!(manualing > 30, "manualing {manualing}");
    // Dropped from 0.6 m onto flat ground: on the ground the tick it lands and after.
    g.place_rider(v3(0.0, 0.0, -12.0), 0.0, 3.0);
    for _ in 0..10 {
        g.update(&pad(0, 0), TICK);
    }
    g.skater.pos.y += 0.6;
    g.skater.mode = Mode::Air { time: 0.3 };
    let mut landed = None;
    for i in 0..60 {
        g.update(&pad(0, 0), TICK);
        if landed.is_none() && g.skater.mode == Mode::Rolling {
            landed = Some(i);
        }
        if landed.is_some_and(|l| i - l < 20) {
            assert_on_ground(&g, &format!("landing, tick {i}"));
        }
    }
    assert!(landed.is_some());
    // A 50-50 on the flat bar (rail top 0.35 m): the trucks' hangers on the rail.
    g.place_rider(v3(4.0, 0.0, 8.0), 0.0, 4.0);
    let s = &mut g.skater;
    s.pos = v3(4.0, 0.55, 10.3);
    s.vel = v3(0.0, -0.5, 4.0);
    s.mode = Mode::Air { time: 0.5 };
    let mut grinding = 0;
    for _ in 0..90 {
        g.update(&pad(0, 0), TICK);
        if g.skater.mode == Mode::Grind {
            grinding += 1;
            if grinding > 30 {
                break;
            }
        }
    }
    assert!(grinding > 30, "grinding: {:?}", g.skater.mode);
    let lowest = part_points(&g, "skatetruck")
        .into_iter()
        .map(|p| p.y - 0.35)
        .fold(f32::MAX, f32::min);
    assert!(
        lowest.abs() < 0.004,
        "trucks {:.1} mm over the rail",
        lowest * 1000.0
    );
}

/// The drawn rider's chest (the line between the shoulders turned a quarter, in the world) and the
/// drawn board's long axis.
fn drawn_rider(g: &Game) -> (Vec3, Vec3) {
    let r = g.rider().expect("imported rider");
    let c = &r.character;
    let w = c.world();
    let x = g.imported_xform();
    let at = |name: &str| {
        let m = w[c.bone(name).expect("bone")];
        x.point(v3(m[3][0], m[3][1], m[3][2]))
    };
    let across = at("RIGHTARM") - at("LEFTARM");
    let chest = v3(across.z, 0.0, -across.x);
    let b = w[c.bone("SKATEBOARD_ROOT").expect("board bone")];
    let axis = x.point(v3(b[2][0], b[2][1], b[2][2])) - x.pos;
    (chest, axis)
}

/// Rides fakie, does `act` until the rider has turned round to switch, then 60 ticks more, checking the
/// drawn rider and board each frame (at `dt`); returns the motion graph states passed through.
fn turn_round_steadily(
    g: &mut Game,
    dt: f32,
    act: impl Fn(u32) -> PadRaw,
    want_state: &str,
) -> Vec<String> {
    g.place_rider(v3(0.0, 0.0, 8.0), core::f32::consts::PI, -3.5);
    for _ in 0..20 {
        g.update(&pad(0, 0), dt);
    }
    assert!(g.skater.fakie, "riding fakie");
    let mut states = Vec::new();
    let (mut chest, mut axis) = drawn_rider(g);
    let mut switched_at = None;
    let mut worst = (0.0f32, 0.0f32);
    for i in 0..400u32 {
        g.update(&act(i), dt);
        for l in g.log.drain(..) {
            if let Some(s) = l.strip_prefix("graph: ") {
                states.push(s.to_string());
            }
        }
        if g.events.contains(&Event::Switched) {
            switched_at.get_or_insert(i);
        }
        let (c, a) = drawn_rider(g);
        let (dc, da) = (yaw_step(chest, c), axis_step(axis, a));
        worst = (worst.0.max(dc), worst.1.max(da));
        assert!(dc < 10.0, "tick {i}: the rider's chest turned {dc:.1} deg");
        assert!(da < 3.0, "tick {i}: the board turned {da:.1} deg");
        (chest, axis) = (c, a);
        if switched_at.is_some_and(|s| i > s + 60) {
            break;
        }
    }
    eprintln!(
        "{want_state}: worst chest step {:.1}, board {:.1}",
        worst.0, worst.1
    );
    assert!(switched_at.is_some(), "turned round: {states:?}");
    assert!(g.skater.switch && !g.skater.fakie && g.skater.speed > 0.0);
    assert!(
        states.iter().any(|s| s.contains(want_state)),
        "{want_state}: {states:?}"
    );
    states
}

#[test]
fn the_auto_switch_turns_nothing_in_the_world() {
    let Some(mut g) = imported_game("auto switch") else {
        return;
    };
    turn_round_steadily(&mut g, TICK, |_| pad(0, 0), "/Turning/Switch/");
    // PAL50: a frame runs one or two ticks and is drawn between the last two.
    let mut g = imported_game("auto switch").unwrap();
    turn_round_steadily(&mut g, 1.0 / 50.0, |_| pad(0, 0), "/Turning/Switch/");
}

#[test]
fn pushing_from_fakie_with_the_graphs_turns_nothing_in_the_world() {
    let Some(mut g) = imported_game("push from fakie") else {
        return;
    };
    let push = |i: u32| pad(0, if (2..8).contains(&i) { btn::A } else { 0 });
    turn_round_steadily(&mut g, TICK, push, "/PushFromFakie/");
}

#[test]
fn riding_fakie_with_the_graphs_the_stick_turns_the_travel_its_own_way() {
    let Some(mut g) = imported_game("fakie steering") else {
        return;
    };
    let mut turn = |speed: f32| {
        g.place_rider(v3(0.0, 0.0, -10.0), 0.0, speed);
        for _ in 0..10 {
            g.update(&pad(0, 0), TICK);
        }
        let fakie = g.skater.fakie;
        let y0 = g.skater.travel_yaw();
        for _ in 0..25 {
            g.update(&pad(-100, 0), TICK);
        }
        (wrap_angle(g.skater.travel_yaw() - y0), fakie)
    };
    let (forward, _) = turn(3.5);
    let (fakie, was_fakie) = turn(-3.5);
    assert!(was_fakie);
    assert!(forward > 0.05 && fakie > 0.05, "{forward} {fakie}");
}
