//! Off the board (Skate 3's biped, `skater::biped`), on the built-in park without imported data (a
//! synthetic state graph times getting off and on, as the graphs' dismount and mount clips do):
//!
//! - Getting off the board on the ground and back on it on foot; the board's nose then points the way
//!   the rider faced, rolling at his speed.
//! - The stick ahead walks and runs at Skate 3's speeds (`physics_biped/SpeedVsInput`: 6 m/s pushed
//!   fully), A sprints a burst towards 10 m/s that fades while held, the stick pulled back turns the
//!   rider round on the spot.
//! - B jumps 0.9 m (`JumpHeight`); walking off a ledge falls; a wall too high to step up stops him.

mod common;

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::{v3, wrap_angle};
use game_core::skater::Event;
use game_core::skater::biped::Loco;

const TICK: f32 = 1.0 / 60.0;
/// A clear lane across the built-in park (nothing in the way along +z for 38 m).
const LANE: game_core::math::Vec3 = game_core::math::Vec3 {
    x: -7.0,
    y: 0.0,
    z: -18.0,
};

fn pad(stick: [i8; 2], buttons: u32) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        stick_x: stick[0],
        stick_y: stick[1],
        buttons,
        ..Default::default()
    }
}

fn run(g: &mut Game, ticks: usize, stick: [i8; 2], buttons: u32) {
    for _ in 0..ticks {
        g.update(&pad(stick, buttons), TICK);
    }
}

/// On foot at `pos` facing `heading`, the camera settled behind.
fn on_foot(pos: game_core::math::Vec3, heading: f32) -> Game {
    let mut g = Game::new();
    g.place_rider(pos, heading, 0.0);
    run(&mut g, 30, [0, 0], 0);
    common::get_off(&mut g);
    assert!(g.skater.walking(), "Y gets off the board standing");
    run(&mut g, 60, [0, 0], 0);
    g
}

#[test]
fn y_gets_off_the_board_and_back_on() {
    let mut g = Game::new();
    g.place_rider(LANE, 0.0, 3.0);
    run(&mut g, 20, [0, 0], 0);
    let speed = g.skater.speed;
    common::get_off(&mut g);
    assert!(g.skater.walking());
    // Off at the board's speed, facing its way, slowing to a stop with the stick let go.
    assert!(
        (g.skater.biped.speed - speed).abs() < 0.3,
        "{}",
        g.skater.biped.speed
    );
    assert!(wrap_angle(g.skater.biped.heading).abs() < 0.05);
    run(&mut g, 90, [0, 0], 0);
    assert!(
        g.skater.biped.speed < 0.01,
        "stops: {}",
        g.skater.biped.speed
    );
    assert_eq!(g.skater.biped.loco, Loco::Stand);

    // Walking: back on facing the way he walks, at his speed.
    run(&mut g, 60, [0, 100], 0);
    let heading = g.skater.biped.heading;
    let walking = g.skater.biped.speed;
    common::get_on_with(&mut g, [0, 100]);
    assert!(g.skater.rolling(), "Y gets back on");
    assert!(wrap_angle(g.skater.heading - heading).abs() < 0.05);
    assert!(
        (g.skater.speed - walking).abs() < 0.5,
        "{} vs {walking}",
        g.skater.speed
    );
    // And rides on: A pushes.
    run(&mut g, 60, [0, 0], btn::A);
    assert!(g.skater.rolling());
}

/// Getting off while riding switch and back on rides on switch: the stance isn't set back to natural
/// (which turned the rider round a moment after getting on).
#[test]
fn getting_back_on_keeps_the_stance() {
    for switch in [false, true] {
        let mut g = Game::new();
        g.place_rider(LANE, 0.0, 3.0);
        g.skater.switch = switch;
        run(&mut g, 20, [0, 0], 0);
        common::get_off(&mut g);
        assert!(g.skater.walking());
        assert_eq!(g.skater.switch, switch, "off the board");
        run(&mut g, 60, [0, 0], 0);
        common::get_on(&mut g);
        assert!(g.skater.rolling());
        assert_eq!(g.skater.switch, switch, "back on");
        run(&mut g, 120, [0, 0], 0);
        assert_eq!(g.skater.switch, switch, "and riding on");
    }
}

#[test]
fn no_getting_off_in_the_air() {
    let mut g = Game::new();
    g.place_rider(v3(0.0, 0.0, -8.0), 0.0, 4.0);
    run(&mut g, 10, [0, 0], 0);
    g.skater.force_state(game_core::skater::State::PhysicsAir);
    g.skater.air_time = 0.0;
    g.skater.states.time = 0.0;
    g.skater.vel.y = 3.0;
    run(&mut g, 1, [0, 0], btn::Y);
    assert!(!g.skater.walking());
}

#[test]
fn walking_running_and_the_sprint_burst() {
    let mut g = on_foot(LANE, 0.0);
    // Half a stick: Skate 3's jog.
    run(&mut g, 90, [0, 60], 0);
    let jog = g.skater.biped.speed;
    assert!((2.0..3.0).contains(&jog), "half a stick jogs at {jog}");
    assert_eq!(g.skater.biped.loco, Loco::Walk);
    // The whole stick runs at 6 m/s.
    run(&mut g, 60, [0, 100], 0);
    let full = g.skater.biped.speed;
    assert!((full - 6.0).abs() < 0.1, "full stick runs at {full}");
    assert_eq!(g.skater.biped.loco, Loco::Run);
    // A sprints: 10 m/s at once...
    g.skater.pos = LANE;
    let mut burst: f32 = 0.0;
    for _ in 0..45 {
        run(&mut g, 1, [0, 100], btn::A);
        burst = burst.max(g.skater.biped.speed);
    }
    assert!(burst > 9.9, "sprint burst {burst}");
    assert_eq!(g.skater.biped.loco, Loco::Sprint);
    // ...fading back to the run while it is held (Skate 3's blend is gone by 3.8 s).
    for _ in 0..4 {
        g.skater.pos = LANE;
        run(&mut g, 60, [0, 100], btn::A);
    }
    let held = g.skater.biped.speed;
    assert!(held < 7.0, "the sprint fades while held: {held}");
}

#[test]
fn the_stick_pulled_back_turns_round_on_the_spot() {
    let mut g = on_foot(LANE, 0.0);
    let start = g.skater.pos;
    let h0 = g.skater.biped.heading;
    // Back and to the right (straight back has no side to turn to: Skate 3 turns right). The stick is read
    // against the chase camera, which sits a few degrees to one side after getting off (so the stick's side
    // has to be clear of that) and follows him round, so he may carry on past facing back: the most he
    // turned counts.
    let mut turned: f32 = 0.0;
    let mut right = 0.0;
    for _ in 0..40 {
        run(&mut g, 1, [30, -100], 0);
        let d = wrap_angle(g.skater.biped.heading - h0);
        if turned < 0.3 {
            right = d;
        }
        turned = turned.max(d.abs());
    }
    // Turning right is towards -x when facing +z: the yaw goes down.
    assert!(right < 0.0, "turned the wrong way: {right}");
    assert!(turned > 2.5, "turned {:.0} deg", turned.to_degrees());
    let moved = (g.skater.pos - start).horizontal().length();
    // (The stick stays where it points while he turns: towards the end it is ahead and he walks.)
    assert!(moved < 1.5, "turning round moved {moved} m");
}

#[test]
fn b_jumps_skate_3s_height() {
    let mut g = on_foot(LANE, 0.0);
    let y0 = g.skater.pos.y;
    run(&mut g, 1, [0, 0], btn::B);
    assert!(g.skater.biped.air.is_some(), "B jumps");
    let mut top: f32 = 0.0;
    let mut ticks = 0;
    while g.skater.biped.air.is_some() && ticks < 120 {
        run(&mut g, 1, [0, 0], 0);
        top = top.max(g.skater.pos.y - y0);
        ticks += 1;
    }
    assert!((top - 0.9).abs() < 0.05, "jumped {top} m");
    assert!(g.skater.biped.air.is_none(), "lands");
    assert!(g.skater.walking());
    // About 2 sqrt(2 h / g) in the air.
    let t = ticks as f32 * TICK;
    assert!((t - 0.857).abs() < 0.05, "{t} s in the air");
}

#[test]
fn walls_stop_and_ledges_drop() {
    // The funbox's deck (0.6 m) is too high to step up onto from its side.
    let mut g = on_foot(v3(4.0, 0.0, 3.5), core::f32::consts::FRAC_PI_2);
    run(&mut g, 120, [0, 100], 0);
    assert!(
        g.skater.pos.x < 6.0,
        "the side stops him: x {}",
        g.skater.pos.x
    );
    assert!(g.skater.pos.y.abs() < 0.01);

    // On its deck, walking off its edge: a fall, then on his feet on the ground.
    let mut g = on_foot(v3(7.0, 0.6, 3.5), -core::f32::consts::FRAC_PI_2);
    assert!((g.skater.pos.y - 0.6).abs() < 0.01, "stands on the deck");
    let mut fell = false;
    for _ in 0..120 {
        run(&mut g, 1, [0, 100], 0);
        fell |= g.skater.biped.air.is_some();
    }
    assert!(fell, "walks off the edge into the air");
    assert!(g.skater.biped.air.is_none());
    assert!(
        g.skater.pos.x < 6.0 && g.skater.pos.y.abs() < 0.01,
        "{:?}",
        g.skater.pos
    );
}

fn data_dir() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("FG_DATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../dist/apps/flatground/data")
        });
    ["skater.fgpak", "motion.fgpak", "rules.fgpak"]
        .iter()
        .all(|f| dir.join(f).is_file())
        .then_some(dir)
}

fn leak(path: std::path::PathBuf) -> &'static [u8] {
    Box::leak(std::fs::read(path).expect("read pack").into_boxed_slice())
}

/// With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; passes without it, or
/// with a motion pack imported before the off-board clips were): Skate 3's own graphs get the rider off
/// (the dismount clip's `OB_TRAJ` hands him to the walker), walk, run and sprint him by their
/// `LocoState` states, jump, and get him back on (the mount clip's `OB_TRAJ` ends on the board).
#[test]
fn imported_graphs_get_off_walk_and_get_back_on() {
    let Some(dir) = data_dir() else {
        eprintln!("offboard: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .expect("rules pack");
    g.load_skater(leak(dir.join("skater.fgpak")))
        .expect("skater pack");
    g.load_motion(leak(dir.join("motion.fgpak")))
        .expect("motion pack");
    let walks = g
        .motion
        .as_ref()
        .and_then(|m| m.db.clip_index("NB_WALK_FWD_CYC"))
        .is_some_and(|i| g.motion.as_ref().unwrap().pose(i).is_some());
    if !walks {
        eprintln!("offboard: the motion pack has no off-board clips (import it again): skipped");
        return;
    }
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    g.place_rider(LANE, 0.0, 3.0);
    run(&mut g, 30, [0, 0], 0);
    run(&mut g, 1, [0, 0], btn::Y);
    assert!(g.skater.rolling(), "the dismount clip plays first");
    let (mut dismount, mut off) = (None, None);
    for i in 0..90 {
        if dismount.is_none() && path(&g).starts_with("Dismount/") {
            dismount = Some(i);
        }
        if g.skater.walking() {
            off = Some(i);
            break;
        }
        run(&mut g, 1, [0, 0], 0);
    }
    let dismount = dismount.expect("the graph plays its dismount");
    let off = off.expect("off the board within a second and a half");
    assert!(
        off > dismount + 3,
        "at the clip's OB_TRAJ, not at once ({dismount}, {off})"
    );
    run(&mut g, 120, [0, 0], 0);
    assert!(
        path(&g).starts_with("OffBoard/OBGround/Locomotion/Stand"),
        "{}",
        path(&g)
    );
    g.skater.pos = LANE;
    run(&mut g, 60, [0, 100], 0);
    assert!(path(&g).contains("/Run_Flat/WithBoard"), "{}", path(&g));
    run(&mut g, 20, [0, 100], btn::A);
    assert!(path(&g).contains("/Sprint_Flat/WithBoard"), "{}", path(&g));
    run(&mut g, 1, [0, 100], btn::B);
    run(&mut g, 2, [0, 100], 0);
    assert!(path(&g).starts_with("OffBoard/OBAir/"), "{}", path(&g));
    for _ in 0..90 {
        run(&mut g, 1, [0, 0], 0);
    }
    assert!(g.skater.biped.air.is_none());
    assert!(path(&g).starts_with("OffBoard/OBGround/"), "{}", path(&g));
    run(&mut g, 60, [0, 0], 0);
    run(&mut g, 1, [0, 0], btn::Y);
    let mut on = false;
    for _ in 0..90 {
        run(&mut g, 1, [0, 0], 0);
        on |= g.skater.rolling();
    }
    assert!(on, "back on the board: {}", path(&g));
    assert!(path(&g).starts_with("OnBoard/"), "{}", path(&g));
}

#[test]
fn y_in_the_air_off_the_board_gets_on_there() {
    let mut g = on_foot(LANE, 0.0);
    run(&mut g, 30, [0, 100], 0);
    run(&mut g, 1, [0, 100], btn::B);
    run(&mut g, 8, [0, 100], 0);
    assert!(g.skater.biped.air.is_some(), "jumping");
    // Y in the air: on the board at once, coming down onto it (Skate 3's air mount)...
    let pad = game_core::input::Pad::from_raw(&pad([0, 0], btn::Y), 0);
    assert_eq!(g.skater.board_toggle_intent(&pad), (true, true));
    run(&mut g, 1, [0, 0], btn::Y);
    assert!(g.events.contains(&Event::SteppedOn));
    assert!(
        g.skater.is_airborne() && g.skater.landing_on_deck(),
        "{:?}",
        g.skater.state()
    );
    // ...and riding once down, still going his way, with no trick scored for it.
    let mut scored = false;
    let mut on = None;
    for i in 0..60 {
        run(&mut g, 1, [0, 0], 0);
        scored |= g.events.iter().any(|e| matches!(e, Event::Landed { .. }));
        if g.skater.rolling() {
            on = Some(i);
            break;
        }
    }
    assert!(on.is_some(), "riding after coming down");
    assert!(!scored);
    assert!(g.skater.speed > 1.0, "{}", g.skater.speed);
    assert!(
        wrap_angle(g.skater.heading).abs() < 0.1,
        "{}",
        g.skater.heading
    );
}

/// With the player's imported data: however Y is pressed around a jump off the board, the physics and the
/// graphs end up agreeing (riding with the graph on the board, or walking with it off), never riding
/// while the graph holds the board in its hands.
#[test]
fn imported_y_around_a_jump_keeps_the_graphs_with_the_physics() {
    let Some(dir) = data_dir() else {
        eprintln!("offboard: no imported data (FG_DATA): skipped");
        return;
    };
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    // When Y goes down after the jump (ticks), and how long it is held.
    for (at, hold) in [(3, 1), (3, 60), (10, 1), (20, 40), (0, 90)] {
        let mut g = Game::new();
        g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
            .unwrap();
        g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
        g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
        g.place_rider(LANE, 0.0, 3.0);
        run(&mut g, 30, [0, 0], 0);
        run(&mut g, 1, [0, 0], btn::Y);
        run(&mut g, 120, [0, 0], 0);
        if !g.skater.walking() {
            eprintln!("offboard: the motion pack has no off-board clips: skipped");
            return;
        }
        g.skater.pos = LANE;
        run(&mut g, 40, [0, 100], 0);
        run(&mut g, 1, [0, 100], btn::B);
        run(&mut g, at, [0, 100], 0);
        run(&mut g, hold, [0, 100], btn::Y);
        let mut mixed = 0;
        for _ in 0..150 {
            run(&mut g, 1, [0, 0], 0);
            let riding = g.skater.rolling() || g.skater.is_airborne();
            mixed = if riding && path(&g).starts_with("OffBoard/") {
                mixed + 1
            } else {
                0
            };
        }
        let p = path(&g);
        match g.skater.state() {
            _ if g.skater.rolling() => {
                assert!(p.starts_with("OnBoard/"), "Y at {at} for {hold}: {p}")
            }
            _ if g.skater.walking() => {
                assert!(p.starts_with("OffBoard/"), "Y at {at} for {hold}: {p}")
            }
            m => panic!("Y at {at} for {hold}: {m:?} {p}"),
        }
        assert_eq!(
            mixed, 0,
            "Y at {at} for {hold}: riding with the graph off the board"
        );
    }
}

/// With the player's imported data: off the board after riding switch, getting back on plays the mount in
/// that stance and rides on in it; the rider's mirror never changes from the press to a second after.
#[test]
fn imported_getting_on_switch_stays_switch() {
    let Some(dir) = data_dir() else {
        eprintln!("offboard: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
    g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
    g.place_rider(LANE, 0.0, 3.0);
    g.skater.switch = true;
    run(&mut g, 30, [0, 0], 0);
    run(&mut g, 1, [0, 0], btn::Y);
    run(&mut g, 120, [0, 0], 0);
    if !g.skater.walking() {
        eprintln!("offboard: the motion pack has no off-board clips: skipped");
        return;
    }
    let mirrored = g.skater.mirrored();
    run(&mut g, 1, [0, 0], btn::Y);
    let mut on = false;
    for i in 0..150 {
        run(&mut g, 1, [0, 0], 0);
        on |= g.skater.rolling();
        assert_eq!(g.skater.mirrored(), mirrored, "tick {i}");
    }
    assert!(on && g.skater.switch, "riding switch");
}

fn imported(dir: &std::path::Path) -> Game {
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
    g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
    g
}

/// With the player's imported data: Y in the air off the board plays Skate 3's air mount (`Mount/Air`:
/// the board from the hand to under the feet) and rides on after coming down, never wiping out.
#[test]
fn imported_y_in_the_air_plays_the_air_mount() {
    let Some(dir) = data_dir() else {
        eprintln!("offboard: no imported data (FG_DATA): skipped");
        return;
    };
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    for at in [3, 15, 30] {
        let mut g = imported(&dir);
        g.place_rider(LANE, 0.0, 3.0);
        run(&mut g, 30, [0, 0], 0);
        run(&mut g, 1, [0, 0], btn::Y);
        run(&mut g, 120, [0, 0], 0);
        if !g.skater.walking() {
            eprintln!("offboard: the motion pack has no off-board clips: skipped");
            return;
        }
        g.skater.pos = LANE;
        run(&mut g, 40, [0, 100], 0);
        run(&mut g, 1, [0, 100], btn::B);
        run(&mut g, at, [0, 0], 0);
        run(&mut g, 1, [0, 0], btn::Y);
        let (mut mounted, mut riding) = (false, false);
        for _ in 0..120 {
            run(&mut g, 1, [0, 0], 0);
            let p = path(&g);
            assert!(!p.starts_with("WipeOut"), "Y {at} ticks up: {p}");
            mounted |= p.starts_with("Mount/Air");
            riding |= g.skater.rolling() && p.starts_with("OnBoard/");
        }
        assert!(mounted && riding, "Y {at} ticks up: {}", path(&g));
    }
}

/// With the player's imported data: walking and running, Skate 3's phase-controlled locomotion cycles
/// play through (they stood frozen on their first frame while the rider slid along).
#[test]
fn imported_the_walk_and_run_cycles_play_through() {
    let Some(dir) = data_dir() else {
        eprintln!("offboard: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = imported(&dir);
    g.place_rider(LANE, 0.0, 0.0);
    run(&mut g, 30, [0, 0], 0);
    run(&mut g, 1, [0, 0], btn::Y);
    run(&mut g, 120, [0, 0], 0);
    if !g.skater.walking() {
        eprintln!("offboard: the motion pack has no off-board clips: skipped");
        return;
    }
    g.skater.pos = LANE;
    for (stick, what) in [(40, "walking"), (100, "running")] {
        run(&mut g, 40, [0, stick], 0);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..60 {
            run(&mut g, 1, [0, stick], 0);
            let f = g.rider().and_then(|r| r.feet()).expect("posed feet");
            // One foot along the facing, against the other.
            let d = f[0][0] - f[1][0];
            lo = lo.min(d);
            hi = hi.max(d);
        }
        assert!(hi - lo > 0.3, "{what}: the feet stay put ({lo}..{hi})");
    }
}

/// With the player's imported data: A and B together wind up Skate 3's own hippie jump (`OnBoard/
/// HippyJump`: its wind-up clips), and letting go jumps the rider off the board while it rolls on along
/// the ground under him (it used to rise with his feet as in an ollie); he comes down onto it riding.
#[test]
fn imported_hippie_jump_winds_up_and_leaves_the_board_rolling() {
    let Some(dir) = data_dir() else {
        eprintln!("offboard: no imported data (FG_DATA): skipped");
        return;
    };
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    let mut g = imported(&dir);
    g.place_rider(LANE, 0.0, 5.0);
    run(&mut g, 40, [0, 0], 0);
    let mut wound = false;
    for _ in 0..40 {
        run(&mut g, 1, [0, 0], btn::A | btn::B);
        wound |= path(&g).starts_with("OnBoard/HippyJump/Antic");
        assert!(g.skater.rolling(), "winding up on the board");
    }
    assert!(wound, "the wind-up plays: {}", path(&g));
    let (mut top, mut board_hi, mut landed) = (0.0f32, f32::MIN, false);
    for _ in 0..90 {
        run(&mut g, 1, [0, 0], 0);
        let p = path(&g);
        assert!(!p.starts_with("WipeOut"), "{p}");
        if g.skater.landing_on_deck() {
            top = top.max(g.skater.pos.y);
            board_hi = board_hi.max(g.drawn_board().y);
        }
        landed |= g
            .events
            .iter()
            .any(|e| matches!(e, Event::Landed { name, .. } if name.contains("HIPPIE")));
    }
    assert!(top > 0.5, "the rider jumps: {top}");
    assert!(board_hi < 0.15, "the board stays on the ground: {board_hi}");
    assert!(landed, "lands the hippie jump");
    assert!(g.skater.rolling());
    assert!(path(&g).starts_with("OnBoard/"), "{}", path(&g));
}

/// With the player's imported data: Y gets on the board promptly standing, running and sprinting (the
/// run and sprint mounts stalled until `TOGGLE_WAIT`: nothing set their `CadenceStartPercent`, Skate 3's
/// MatchCadence, so they never exposed `OB_TRAJ`).
#[test]
fn imported_mounts_from_stand_run_and_sprint_get_on_promptly() {
    let Some(dir) = data_dir() else {
        eprintln!("offboard: no imported data (FG_DATA): skipped");
        return;
    };
    for (what, stick, buttons) in [
        ("stand", [0, 0], 0),
        ("run", [0, 100], 0),
        ("sprint", [0, 100], btn::A),
    ] {
        let mut g = imported(&dir);
        g.place_rider(LANE, 0.0, 3.0);
        run(&mut g, 30, [0, 0], 0);
        run(&mut g, 1, [0, 0], btn::Y);
        run(&mut g, 150, [0, 0], 0);
        if !g.skater.walking() {
            eprintln!("offboard: the motion pack has no off-board clips: skipped");
            return;
        }
        g.skater.pos = LANE;
        run(&mut g, 60, stick, buttons);
        run(&mut g, 1, stick, buttons | btn::Y);
        let on = (0..40).position(|_| {
            run(&mut g, 1, stick, buttons);
            g.skater.rolling()
        });
        assert!(on.is_some(), "{what}: never got on");
    }
}
