//! The camera's polish, as Skate 3 does it (`camera::look`, `camera::shake`, `camera::slow_motion`, the
//! High / Low camera):
//!
//! - free look: Z + C-stick on foot tilts the camera (Skate 3's 30 degrees) and lets go of it after; on
//!   the board the same chord does nothing at all (no flick, no dark catch, no look);
//! - the pause menu's camera row switches to Skate 3's high camera (further back, higher) and back;
//! - a landing shakes the camera (Skate 3's one-shot impulse), a bail fades the shake out;
//! - a long air slows the simulation clock (Skate 3's curve, down to 35 ticks a second), and the
//!   simulation stays the same tick by tick as without slow motion; the replay records it slowed.
//!
//! On the built-in park and shots; the `imported_` tests also run Skate 3's camera graph and shots from
//! the player's imported data (skipped without it).

use game_core::Game;
use game_core::camera::select;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::{Vec3, atan2f, v3};
use game_core::menu::CameraAngle;
use game_core::park::Park;
use game_core::skater::Event;

const TICK: f32 = 1.0 / 60.0;
const LANE: Vec3 = Vec3 {
    x: -7.0,
    y: 0.0,
    z: -18.0,
};

fn pad(buttons: u32, c: (i8, i8)) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        buttons,
        cstick_x: c.0,
        cstick_y: c.1,
        ..Default::default()
    }
}

fn run(g: &mut Game, ticks: usize, buttons: u32, c: (i8, i8)) -> Vec<Event> {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        g.update(&pad(buttons, c), TICK);
        seen.extend(g.events.iter().cloned());
    }
    seen
}

/// The drawn camera's angle above the rider (degrees).
fn elevation(g: &Game) -> f32 {
    let c = g.camera().steady();
    let d = c.eye - g.skater.pos;
    atan2f(d.y, d.horizontal().length()).to_degrees()
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
    Box::leak(std::fs::read(path).unwrap().into_boxed_slice())
}

fn imported(dir: &std::path::Path) -> Game {
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
    g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
    g
}

/// On foot at the lane, standing, the camera settled.
fn on_foot(mut g: Game) -> Option<Game> {
    g.place_rider(LANE, 0.0, 0.0);
    run(&mut g, 30, 0, (0, 0));
    run(&mut g, 1, btn::Y, (0, 0));
    run(&mut g, 150, 0, (0, 0));
    (g.skater.walking()).then_some(g)
}

/// Z + C-stick up on foot: the camera rises to look down on the rider by Skate 3's 30 degrees, the rider
/// does nothing, and letting go gives the camera back.
fn free_look(mut g: Game, what: &str) {
    assert!(
        g.camera().look_allowed(),
        "{what}: {}",
        g.camera().shot_name
    );
    let before = elevation(&g);
    let pos = g.skater.pos;
    run(&mut g, 1, btn::Z, (0, 0));
    let seen = run(&mut g, 150, btn::Z, (0, 80));
    assert!(g.camera().looking(), "{what}");
    let look = g.camera().look.elevation.to_degrees();
    // The C-stick 80 of 92 is 0.89 of the way: 0.89 x 30 degrees, closed on at 2/s.
    assert!(look > 23.0 && look < 30.0, "{what}: look {look}");
    let up = elevation(&g);
    assert!(up > before + 12.0, "{what}: {before} -> {up} degrees");
    assert!(g.skater.walking(), "{what}: {seen:?}");
    assert!(
        (g.skater.pos - pos).length() < 0.05,
        "{what}: the rider stays"
    );
    // Let go: the camera eases back to the shot.
    let seen = run(&mut g, 240, 0, (0, 0));
    assert!(!g.camera().looking());
    assert!(g.camera().look.elevation.to_degrees().abs() < 1.0, "{what}");
    let back = elevation(&g);
    assert!(
        (back - before).abs() < 3.0,
        "{what}: back to {back} from {before} degrees"
    );
    assert!(!seen.contains(&Event::SteppedOff), "{what}");
}

#[test]
fn free_look_on_foot_tilts_the_camera_and_gives_it_back() {
    let g = on_foot(Game::new()).expect("Y gets off the board");
    free_look(g, "built-in");
}

#[test]
fn imported_free_look_on_foot_with_skate3s_shots() {
    let Some(dir) = data_dir() else {
        eprintln!("camera_polish: no imported data: skipped");
        return;
    };
    let Some(g) = on_foot(imported(&dir)) else {
        eprintln!("camera_polish: the motion pack has no off-board clips: skipped");
        return;
    };
    if !g.camera().look_allowed() {
        eprintln!(
            "camera_polish: the imported shots predate OptionUseFreeCamStick (re-import): skipped"
        );
        return;
    }
    free_look(g, "imported");
}

/// On the board Z + C-stick is nothing: the game is exactly the one that pressed nothing (no ollie, no
/// manual, no dark catch, no look), and the C-stick alone still flicks.
#[test]
fn z_and_the_c_stick_on_the_board_do_nothing() {
    let start = |g: &mut Game| {
        g.place_rider(LANE, 0.0, 5.0);
        run(g, 30, 0, (0, 0));
    };
    let (mut chord, mut idle) = (Game::new(), Game::new());
    start(&mut chord);
    start(&mut idle);
    run(&mut chord, 1, btn::Z, (0, 0));
    run(&mut idle, 1, 0, (0, 0));
    // A wind-up and a flick, with Z held.
    let mut seen = run(&mut chord, 20, btn::Z, (0, -92));
    seen.extend(run(&mut chord, 3, btn::Z, (0, 92)));
    seen.extend(run(&mut chord, 60, btn::Z, (0, 0)));
    run(&mut idle, 83, 0, (0, 0));
    assert!(
        !seen.iter().any(|e| matches!(e, Event::Popped(_))),
        "{seen:?}"
    );
    assert_eq!(chord.skater.pos, idle.skater.pos);
    assert!(chord.skater.rolling());
    assert_eq!(chord.camera().camera().eye, idle.camera().camera().eye);
    assert_eq!(chord.camera().look.elevation, 0.0);
    // The C-stick alone pops an ollie.
    let mut g = Game::new();
    start(&mut g);
    run(&mut g, 20, 0, (0, -92));
    let seen = run(&mut g, 3, 0, (0, 92));
    assert!(
        seen.iter().any(|e| matches!(e, Event::Popped(_))),
        "{seen:?}"
    );
}

/// Rides straight on, returning the camera's distance and elevation once settled.
fn settled(g: &mut Game) -> (f32, f32, String) {
    run(g, 240, 0, (0, 0));
    (
        g.camera().distance(),
        elevation(g),
        g.camera().shot_name.clone(),
    )
}

fn high_and_low(mut g: Game, what: &str) {
    g.place_rider(LANE, 0.0, 4.0);
    let (low_d, low_e, low_shot) = settled(&mut g);
    assert_eq!(g.camera_angle(), CameraAngle::Low, "Skate 3's default");
    // The pause menu: START, down to the camera row, right.
    run(&mut g, 1, btn::START, (0, 0));
    run(&mut g, 1, 0, (0, 0));
    for _ in 0..3 {
        run(&mut g, 1, btn::DPAD_DOWN, (0, 0));
        run(&mut g, 1, 0, (0, 0));
    }
    run(&mut g, 1, btn::DPAD_RIGHT, (0, 0));
    run(&mut g, 1, 0, (0, 0));
    assert_eq!(g.camera_angle(), CameraAngle::High, "{what}");
    assert!(g.take_settings_changed(), "saved when the menu closes");
    run(&mut g, 1, btn::START, (0, 0));
    run(&mut g, 1, 0, (0, 0));
    assert!(g.menu.is_none());
    let (high_d, high_e, high_shot) = settled(&mut g);
    assert_eq!(high_shot, "bl_high_chase", "{what}");
    assert_ne!(low_shot, high_shot, "{what}");
    assert!(
        high_d > low_d + 0.5 && high_e > low_e + 10.0,
        "{what}: low {low_d:.2} m {low_e:.1} deg, high {high_d:.2} m {high_e:.1} deg"
    );
    g.set_camera_angle(CameraAngle::Low);
    let (d, e, shot) = settled(&mut g);
    assert_eq!(shot, low_shot, "{what}");
    assert!(
        (d - low_d).abs() < 0.3 && (e - low_e).abs() < 3.0,
        "{what}: {d} {e}"
    );
    eprintln!(
        "{what}: low {low_shot} {low_d:.2} m {low_e:.1} deg, high {high_shot} {high_d:.2} m {high_e:.1} deg"
    );
}

#[test]
fn the_high_camera_setting_rides_further_back_and_higher() {
    high_and_low(Game::new(), "built-in");
}

#[test]
fn imported_the_high_camera_runs_skate3s_high_graph() {
    let Some(dir) = data_dir() else {
        eprintln!("camera_polish: no imported data: skipped");
        return;
    };
    let g = imported(&dir);
    if g.camera().data.graph.is_none() {
        eprintln!("camera_polish: no camera graph in the rules: skipped");
        return;
    }
    high_and_low(g, "imported");
}

/// Flat ground all round, the rider rolling along +z.
fn flat_game() -> Game {
    let mut g = Game::new();
    g.park = Park {
        shapes: Vec::new(),
        half_size: 200.0,
        wall_height: 2.0,
        spawn: v3(0.0, 0.0, -150.0),
        spawn_yaw: 0.0,
        rails: Vec::new(),
    };
    g.place_rider(v3(0.0, 0.0, -150.0), 0.0, 6.0);
    run(&mut g, 60, 0, (0, 0));
    g
}

/// Lifts the rolling rider `height` m into the air (as off a drop).
fn launch(g: &mut Game, height: f32) {
    g.skater.pos.y += height;
    g.skater.force_state(game_core::skater::State::PhysicsAir);
    g.skater.air_time = 0.0;
    g.skater.states.time = 0.0;
}

/// The view's shake this tick: its rotation's size (radians).
fn shaking(g: &Game) -> f32 {
    g.camera()
        .shake
        .offset
        .rotation
        .iter()
        .map(|r| r.abs())
        .sum()
}

#[test]
fn a_hard_landing_shakes_the_camera_and_a_bail_fades_it_out() {
    let mut g = flat_game();
    let mut calm = 0.0;
    for _ in 0..60 {
        run(&mut g, 1, 0, (0, 0));
        calm += shaking(&g);
        assert_eq!(g.camera().shake.impulse, 0.0);
    }
    // Rolling, the camera sways a little with the speed (Skate 3's amplitude curve).
    assert!(calm > 0.0);
    launch(&mut g, 2.5);
    let mut landed = None;
    for t in 0..120 {
        run(&mut g, 1, 0, (0, 0));
        if g.camera().shake.impulse > 0.0 {
            landed = Some(t);
            break;
        }
    }
    let landed = landed.expect("the landing shakes");
    assert!(landed > 20, "only once down ({landed} ticks)");
    // Skate 3's OneShotMagnitude: 0.5, a tick's decay in.
    let impulse = g.camera().shake.impulse;
    assert!((impulse - 0.5 * 0.98).abs() < 1e-4, "{impulse}");
    let mut hit = 0.0;
    for _ in 0..60 {
        run(&mut g, 1, 0, (0, 0));
        hit += shaking(&g);
    }
    // The impulse (0.5) adds to the sway's amplitude at this speed (about 1.5) for a while.
    assert!(hit > calm * 1.2, "after the landing {hit} against {calm}");
    // It decays away (2% a tick, gone under 0.005).
    run(&mut g, 300, 0, (0, 0));
    assert_eq!(g.camera().shake.impulse, 0.0);
    // The drawn camera is the shaken one; what the walker reads against is the steady one.
    assert_ne!(g.camera().camera().eye, g.camera().steady().eye);

    // A bail: the shake fades out (Skate 3 shakes a wipeout only in its broken-bone slow motion).
    let mut g = flat_game();
    launch(&mut g, 2.0);
    let mut bailed = false;
    for _ in 0..200 {
        // Grabbing and tweaking into the landing wipes out.
        let p = PadRaw {
            trig_l: 255,
            ..pad(0, (80, 20))
        };
        g.update(&p, TICK);
        bailed |= g.events.contains(&Event::Bailed);
        if bailed {
            break;
        }
    }
    if !bailed {
        eprintln!("camera_polish: no bail from the tweaked landing: bail part skipped");
        return;
    }
    run(&mut g, 13, 0, (0, 0));
    if g.skater.bailed() {
        assert_eq!(g.camera().shake.weight, 0.0);
        assert_eq!(shaking(&g), 0.0);
    }
}

/// Every tick's state (the tick count, the rider, the camera), as a run of frames goes.
fn ticks_of(g: &mut Game, ticks: u64, rates: &mut Vec<f32>) -> (Vec<String>, u64) {
    let mut out = Vec::new();
    let mut frames = 0;
    while g.ticks < ticks {
        let before = g.ticks;
        g.update(&pad(0, (0, 0)), TICK);
        frames += 1;
        assert!(g.ticks - before <= 1, "at most a tick a frame at 60 Hz");
        rates.push(g.camera().rate());
        if g.ticks > before {
            let s = &g.skater;
            let c = g.camera().camera();
            out.push(format!(
                "{} {:?} {:?} {:?} {:?} {:?}",
                g.ticks,
                s.pos,
                s.vel,
                s.state(),
                c.eye,
                c.target
            ));
        }
    }
    (out, frames)
}

fn slow_motion(mut slow: Game, mut real: Game, what: &str) {
    for g in [&mut slow, &mut real] {
        g.place_rider(LANE, 0.0, 6.0);
        run(g, 60, 0, (0, 0));
        launch(g, 30.0);
    }
    real.slow_motion = false;
    let end = slow.ticks + 240;
    let mut rates = Vec::new();
    let (a, frames_slow) = ticks_of(&mut slow, end, &mut rates);
    let (b, frames_real) = ticks_of(&mut real, end, &mut Vec::new());
    // The same simulation, tick by tick.
    assert_eq!(a.len(), b.len(), "{what}");
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x, y, "{what}");
    }
    // Slowed: more frames for the same ticks; Skate 3's curve reaches 35 ticks a second, and ends back
    // at 60 on landing.
    let least = rates.iter().copied().fold(60.0, f32::min);
    assert_eq!(least, 35.0, "{what}");
    assert!(
        frames_slow > frames_real + 20,
        "{what}: {frames_slow} frames slowed against {frames_real}"
    );
    assert_eq!(frames_real, 240);
    assert_eq!(*rates.last().unwrap(), 60.0, "{what}: back to real time");
    assert!(!slow.camera().slowing());
    // The replay recorded one frame a tick, each for the shown time it took: slow motion replays slowed.
    let rec = &slow.recording;
    let n = rec.len();
    let span: Vec<_> = (n - a.len()..n).map(|i| rec.header(i).unwrap()).collect();
    for w in span.windows(2) {
        assert_eq!(w[1].tick, w[0].tick + 1, "{what}");
    }
    let shown: f32 = span.iter().skip(1).map(|h| h.dt).sum();
    let expected = (frames_slow - 1) as f32 * TICK;
    assert!(
        (shown - expected).abs() < 0.2,
        "{what}: recorded {shown} s of {expected}"
    );
    let longest = span.iter().map(|h| h.dt).fold(0.0, f32::max);
    assert!(longest > 1.5 * TICK, "{what}: {longest}");
    eprintln!("{what}: 240 ticks in {frames_slow} frames slowed ({frames_real} in real time)");
}

#[test]
fn a_long_air_slows_time_and_the_simulation_stays_the_same() {
    slow_motion(Game::new(), Game::new(), "built-in");
}

#[test]
fn the_high_camera_never_slows_and_a_short_air_doesnt() {
    let mut g = Game::new();
    g.set_camera_angle(CameraAngle::High);
    g.place_rider(LANE, 0.0, 6.0);
    run(&mut g, 60, 0, (0, 0));
    launch(&mut g, 30.0);
    let mut rates = Vec::new();
    let end = g.ticks + 200;
    ticks_of(&mut g, end, &mut rates);
    assert!(rates.iter().all(|r| *r == 60.0));
    assert_eq!(g.camera().camera_type, select::HIGH);
    // A low drop (under 2 s of air) on the low camera: real time.
    let mut g = Game::new();
    g.place_rider(LANE, 0.0, 6.0);
    run(&mut g, 60, 0, (0, 0));
    launch(&mut g, 3.0);
    let mut rates = Vec::new();
    let end = g.ticks + 120;
    ticks_of(&mut g, end, &mut rates);
    assert!(rates.iter().all(|r| *r == 60.0));
}

#[test]
fn imported_a_long_air_slows_through_skate3s_graph() {
    let Some(dir) = data_dir() else {
        eprintln!("camera_polish: no imported data: skipped");
        return;
    };
    let g = imported(&dir);
    if g.camera().data.graph.is_none() {
        eprintln!("camera_polish: no camera graph in the rules: skipped");
        return;
    }
    slow_motion(g, imported(&dir), "imported");
}
