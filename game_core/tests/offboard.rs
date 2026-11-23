//! Off the board (Skate 3's biped, `skater::biped`), on the built-in park without imported data:
//!
//! - Y gets off the board on the ground and back on it on foot; the board's nose then points the way the
//!   rider faced, rolling at his speed.
//! - The stick ahead walks and runs at Skate 3's speeds (`physics_biped/SpeedVsInput`: 6 m/s pushed
//!   fully), A sprints a burst towards 10 m/s that fades while held, the stick pulled back turns the
//!   rider round on the spot.
//! - B jumps 0.9 m (`JumpHeight`); walking off a ledge falls; a wall too high to step up stops him.

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::{v3, wrap_angle};
use game_core::skater::biped::Loco;
use game_core::skater::{Event, Mode};

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
    run(&mut g, 1, [0, 0], btn::Y);
    assert_eq!(g.skater.mode, Mode::Walk, "Y gets off the board standing");
    assert!(g.events.contains(&Event::SteppedOff));
    run(&mut g, 60, [0, 0], 0);
    g
}

#[test]
fn y_gets_off_the_board_and_back_on() {
    let mut g = Game::new();
    g.place_rider(LANE, 0.0, 3.0);
    run(&mut g, 20, [0, 0], 0);
    let speed = g.skater.speed;
    run(&mut g, 1, [0, 0], btn::Y);
    assert_eq!(g.skater.mode, Mode::Walk);
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
    run(&mut g, 1, [0, 100], btn::Y);
    assert_eq!(g.skater.mode, Mode::Rolling, "Y gets back on");
    assert!(g.events.contains(&Event::SteppedOn));
    assert!(wrap_angle(g.skater.heading - heading).abs() < 0.05);
    assert!(
        (g.skater.speed - walking).abs() < 0.5,
        "{} vs {walking}",
        g.skater.speed
    );
    // And rides on: A pushes.
    run(&mut g, 60, [0, 0], btn::A);
    assert_eq!(g.skater.mode, Mode::Rolling);
}

#[test]
fn no_getting_off_in_the_air() {
    let mut g = Game::new();
    g.place_rider(v3(0.0, 0.0, -8.0), 0.0, 4.0);
    run(&mut g, 10, [0, 0], 0);
    g.skater.mode = Mode::Air { time: 0.0 };
    g.skater.vel.y = 3.0;
    run(&mut g, 1, [0, 0], btn::Y);
    assert_ne!(g.skater.mode, Mode::Walk);
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
    assert_eq!(g.skater.mode, Mode::Walk);
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
    assert_eq!(
        g.skater.mode,
        Mode::Rolling,
        "the dismount clip plays first"
    );
    let (mut dismount, mut off) = (None, None);
    for i in 0..90 {
        if dismount.is_none() && path(&g).starts_with("Dismount/") {
            dismount = Some(i);
        }
        if g.skater.mode == Mode::Walk {
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
        on |= g.skater.mode == Mode::Rolling;
    }
    assert!(on, "back on the board: {}", path(&g));
    assert!(path(&g).starts_with("OnBoard/"), "{}", path(&g));
}
