//! Session markers and the replay through the real input path (`game_core::marker`, `game_core::replay`):
//!
//! - Z + d-pad up puts a marker down riding; ridden away (with a trick's line going), Z + d-pad down held
//!   takes the rider back to it at rest, facing the way he was, and ends the line; a short hold doesn't.
//!   On foot the marker is an on-foot one; mid-bail it can't go down; in the air the chord isn't taken.
//! - Forty seconds recorded, Z + d-pad left opens the replay on the last 30 s: every kept frame shows the
//!   rider and the camera exactly as they were drawn then, and leaving it (X) the game goes on exactly
//!   as a game that never opened it.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it):
//!   the replay poses the recorded skeleton (to the packing's millimetre) and puts the live one back on
//!   leaving, and the state graphs follow the marker's return on the board and on foot.

mod common;

use std::path::PathBuf;

use game_core::Game;
use game_core::gesture::{Flick, Trick};
use game_core::input::{PadRaw, btn, flag};
use game_core::math::v3;
use game_core::park::Park;
use game_core::replay::FRAMES;
use game_core::skater::{Event, GraphControl};

const TICK: f32 = 1.0 / 60.0;

fn flat() -> Park {
    Park {
        shapes: Vec::new(),
        half_size: 200.0,
        wall_height: 2.0,
        spawn: v3(0.0, 0.0, -150.0),
        spawn_yaw: 0.0,
        rails: Vec::new(),
    }
}

fn game() -> Game {
    let mut g = Game::new();
    g.park = flat();
    g.place_rider(v3(0.0, 0.0, -150.0), 0.0, 7.0);
    run(&mut g, 30, pad(0));
    g
}

fn pad(buttons: u32) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        buttons,
        ..Default::default()
    }
}

fn run(g: &mut Game, frames: usize, p: PadRaw) -> Vec<Event> {
    let mut events = Vec::new();
    for _ in 0..frames {
        g.update(&p, TICK);
        events.extend(g.events.iter().cloned());
    }
    events
}

/// Z, then Z with a d-pad direction, then let go: one chord.
fn chord(g: &mut Game, dpad: u32, hold: usize) -> Vec<Event> {
    let mut ev = run(g, 1, pad(btn::Z));
    ev.extend(run(g, hold, pad(btn::Z | dpad)));
    ev.extend(run(g, 1, pad(0)));
    ev
}

/// A kickflip the (synthetic) state graph pops, then rolling on until back on the ground.
fn kickflip(g: &mut Game) {
    run(g, 20, pad(0));
    g.skater.graph.pop = Some((Flick::of(Trick::Kickflip), 1.0));
    run(g, 1, pad(0));
    g.skater.graph.pop = None;
    for _ in 0..200 {
        run(g, 1, pad(0));
        if !g.skater.is_airborne() && g.skater.rolling() {
            break;
        }
    }
    run(g, 12, pad(0));
}

#[test]
fn z_and_d_pad_up_puts_a_marker_down_and_z_and_down_takes_the_rider_back() {
    let mut g = game();
    let ev = chord(&mut g, btn::DPAD_UP, 1);
    assert!(ev.contains(&Event::MarkerSet), "{ev:?}");
    let m = g.markers.marker.expect("a marker");
    assert!(m.on_board);
    assert!((m.pos - g.skater.pos).length() < 0.5);
    let heading = m.heading;
    // Ride away and start a line.
    kickflip(&mut g);
    assert!(g.scoring.in_line(), "the flip starts a line");
    run(&mut g, 60, pad(0));
    let away = (g.skater.pos - m.pos).length();
    assert!(away > 10.0, "{away}");
    // A short hold doesn't go back (Skate 3: 0.2 s within 100 m).
    let ev = chord(&mut g, btn::DPAD_DOWN, 6);
    assert!(!ev.contains(&Event::MarkerReturned));
    assert!((g.skater.pos - m.pos).length() > 10.0);
    // Held: back at it within 0.2 s and a tick.
    let mut ev = run(&mut g, 1, pad(btn::Z));
    let mut held = 0;
    while !ev.contains(&Event::MarkerReturned) && held < 30 {
        ev.extend(run(&mut g, 1, pad(btn::Z | btn::DPAD_DOWN)));
        held += 1;
    }
    assert!(ev.contains(&Event::MarkerReturned), "{ev:?}");
    assert_eq!(held, 13, "Skate 3's 0.2 s hold");
    assert!((g.skater.pos - m.pos).length() < 0.2, "{:?}", g.skater.pos);
    assert!(g.skater.rolling());
    assert!(g.skater.speed.abs() < 0.1, "at rest: {}", g.skater.speed);
    assert!((g.skater.heading - heading).abs() < 1e-3);
    assert!(!g.scoring.in_line(), "going back ends the line");
    // Holding on does nothing more; let go and push away.
    let ev = run(&mut g, 30, pad(btn::Z | btn::DPAD_DOWN));
    assert!(!ev.contains(&Event::MarkerReturned));
    run(&mut g, 1, pad(0));
    g.skater.graph = GraphControl {
        push_contact: true,
        push_target: Some(3.0),
        pushing: true,
        ..GraphControl::default()
    };
    run(&mut g, 1, pad(btn::A));
    g.skater.graph = GraphControl::default();
    run(&mut g, 60, pad(btn::A));
    assert!(g.skater.speed > 1.0, "rides on: {}", g.skater.speed);
}

#[test]
fn a_marker_on_foot_takes_the_rider_back_on_foot() {
    let mut g = Game::new();
    g.place_rider(v3(-7.0, 0.0, -18.0), 0.0, 0.0);
    run(&mut g, 30, pad(0));
    common::get_off(&mut g);
    run(&mut g, 60, pad(0));
    assert!(g.skater.walking());
    let ev = chord(&mut g, btn::DPAD_UP, 1);
    assert!(ev.contains(&Event::MarkerSet), "{ev:?}");
    let m = g.markers.marker.unwrap();
    assert!(!m.on_board);
    // Walk away.
    let walk = PadRaw {
        stick_y: 100,
        ..pad(0)
    };
    run(&mut g, 90, walk);
    assert!((g.skater.pos - m.pos).length() > 2.0);
    let ev = chord(&mut g, btn::DPAD_DOWN, 20);
    assert!(ev.contains(&Event::MarkerReturned), "{ev:?}");
    assert!(g.skater.walking(), "on foot, as when it went down");
    assert!((g.skater.pos - m.pos).length() < 0.2);
}

#[test]
fn no_marker_mid_bail_and_no_chord_in_the_air() {
    let mut g = game();
    g.skater
        .force_state(game_core::skater::State::WipeoutGround);
    g.skater.states.time = 0.0;
    let ev = chord(&mut g, btn::DPAD_UP, 1);
    assert!(ev.contains(&Event::MarkerRefused), "{ev:?}");
    assert!(g.markers.marker.is_none());
    // In the air Z is the dark catch: the chord isn't taken at all.
    let mut g = game();
    g.place(&game_core::autoplay::Placement {
        pos: v3(0.0, 1.5, -100.0),
        heading: 0.0,
        speed: 5.0,
        lift: 3.0,
    });
    run(&mut g, 2, pad(0));
    assert!(g.skater.is_airborne());
    let ev = chord(&mut g, btn::DPAD_UP, 1);
    assert!(!ev.contains(&Event::MarkerSet) && !ev.contains(&Event::MarkerRefused));
    let ev = chord(&mut g, btn::DPAD_LEFT, 1);
    assert!(g.replay.is_none(), "{ev:?}");
}

/// Rides round in circles (pushing to keep the speed up, a kickflip now and then) for `frames`.
fn ride(g: &mut Game, frames: usize, mut each: impl FnMut(&mut Game)) {
    for f in 0..frames {
        let mut p = pad(0);
        p.stick_x = 45;
        if g.skater.speed < 5.0 && g.skater.rolling() && f % 90 < 45 {
            p.buttons |= btn::A;
        }
        match f % 240 {
            100..120 => p.cstick_y = -92,
            120 => {
                p.cstick_x = 50;
                p.cstick_y = 62;
            }
            _ => {}
        }
        g.update(&p, TICK);
        each(g);
    }
}

#[test]
fn the_replay_shows_the_last_30_seconds_as_they_were_drawn() {
    let mut g = game();
    let start = g.ticks;
    let mut seen = Vec::new();
    ride(&mut g, 40 * 60, |g| {
        if g.ticks % 97 == 0 {
            seen.push((g.ticks, g.shown_figure(), g.shown_camera()));
        }
    });
    let end = g.ticks;
    assert_eq!(end - start, 40 * 60);
    assert!(seen.len() > 20);
    chord(&mut g, btn::DPAD_LEFT, 1);
    assert!(g.replay.is_some(), "Z + d-pad left opens the replay");
    let len = g.recording.len();
    assert_eq!(len, FRAMES, "30 s at 60 Hz");
    // The Z press before the chord ran a tick: the newest frame is it.
    assert_eq!(g.recording.header(len - 1).unwrap().tick, end + 1);
    assert_eq!(g.recording.header(0).unwrap().tick, end + 2 - FRAMES as u64);
    let (mut kept, mut gone) = (0, 0);
    for (tick, figure, cam) in &seen {
        let Some(i) = g.recording.find(*tick) else {
            assert!(end + 2 - tick > FRAMES as u64, "{tick} should be kept");
            gone += 1;
            continue;
        };
        g.replay.as_mut().unwrap().seek(i as f32, len);
        g.update(&pad(0), TICK);
        assert_eq!(g.shown_figure(), *figure, "the rider at tick {tick}");
        assert_eq!(g.shown_camera(), *cam, "the camera at tick {tick}");
        kept += 1;
    }
    assert!(kept > 10 && gone > 3, "{kept} {gone}");
    // Played from the start (A at the end), one frame a frame; slowed down, half a frame.
    g.replay.as_mut().unwrap().seek(len as f32, len);
    run(&mut g, 1, pad(btn::A));
    let p = g.replay.unwrap();
    assert!(p.playing && (p.cursor - 1.0).abs() < 1e-3, "{p:?}");
    run(&mut g, 10, pad(0));
    assert!((g.replay.unwrap().cursor - 11.0).abs() < 1e-3);
    run(&mut g, 1, pad(btn::DPAD_DOWN));
    run(&mut g, 4, pad(0));
    assert!((g.replay.unwrap().cursor - 13.5).abs() < 1e-3);
    // The tripod and orbit cameras look at the rider.
    run(&mut g, 1, pad(btn::Y));
    let rider = g.shown_figure().body.pos;
    let c = g.shown_camera();
    assert!((c.target - rider).length() < 1.5);
    run(&mut g, 1, pad(0));
    run(&mut g, 1, pad(btn::Y));
    let c = g.shown_camera();
    assert!(((c.eye - c.target).length() - 3.5).abs() < 0.01);
    // Nothing ran meanwhile; X leaves.
    assert_eq!(g.ticks, end + 1);
    run(&mut g, 1, pad(btn::X));
    assert!(g.replay.is_none());
}

#[test]
fn leaving_the_replay_the_game_goes_on_exactly_where_it_stopped() {
    let (mut a, mut b) = (game(), game());
    ride(&mut a, 600, |_| {});
    ride(&mut b, 600, |_| {});
    // Z down a tick in both; then only `a` opens the replay, plays, scrubs and turns the cameras.
    run(&mut a, 1, pad(btn::Z));
    run(&mut b, 1, pad(btn::Z));
    run(&mut a, 1, pad(btn::Z | btn::DPAD_LEFT));
    assert!(a.replay.is_some());
    run(&mut a, 1, pad(0));
    run(&mut a, 1, pad(btn::A));
    run(&mut a, 120, pad(0));
    let scrub = PadRaw {
        trig_l: 255,
        ..pad(0)
    };
    run(&mut a, 30, scrub);
    run(&mut a, 1, pad(btn::Y));
    run(&mut a, 60, pad(0));
    run(&mut a, 1, pad(btn::DPAD_RIGHT));
    run(&mut a, 1, pad(btn::X));
    assert!(a.replay.is_none());
    assert_eq!(a.ticks, b.ticks);
    assert_eq!(a.skater, b.skater);
    assert_eq!(a.shown_figure(), b.shown_figure());
    assert_eq!(a.shown_camera(), b.shown_camera());
    // And on from there, the same.
    ride(&mut a, 300, |_| {});
    ride(&mut b, 300, |_| {});
    assert_eq!(a.skater, b.skater);
    assert_eq!(a.scoring, b.scoring);
    assert_eq!(a.score, b.score);
    assert_eq!(a.shown_camera(), b.shown_camera());
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
    g
}

fn close(a: &[[[f32; 3]; 4]], b: &[[[f32; 3]; 4]], tol: f32) -> bool {
    a.len() == b.len()
        && a.iter()
            .flatten()
            .flatten()
            .zip(b.iter().flatten().flatten())
            .all(|(x, y)| (x - y).abs() < tol)
}

#[test]
fn imported_the_replay_poses_the_recorded_skeleton_and_puts_the_live_one_back() {
    let Some(dir) = data_dir() else {
        eprintln!("markers_replay: no imported data (FG_DATA): skipped");
        return;
    };
    let mut g = imported(&dir);
    g.park = flat();
    g.place_rider(v3(0.0, 0.0, -150.0), 0.0, 7.0);
    let mut seen = Vec::new();
    ride(&mut g, 12 * 60, |g| {
        if g.ticks % 53 == 0 {
            seen.push((g.ticks, g.shown_figure(), g.rider_bones().unwrap().to_vec()));
        }
    });
    let bones = g.rider_bones().unwrap().len();
    assert!(bones > 0);
    assert_eq!(g.recording.bone_count(), bones);
    // Under 1.5 MB for the 30 s (Skate 3's skater is 36 bones).
    assert!(game_core::replay::Recording::budget(bones) < 1_500_000);
    assert!(g.recording.bytes() <= game_core::replay::Recording::budget(bones));
    run(&mut g, 1, pad(btn::Z));
    let live_z = g.rider_bones().unwrap().to_vec();
    let skater_z = g.skater.clone();
    let path_z = g.graph.as_ref().unwrap().runner.motion_state();
    run(&mut g, 1, pad(btn::Z | btn::DPAD_LEFT));
    assert!(g.replay.is_some());
    let len = g.recording.len();
    for (tick, figure, pose) in &seen {
        let i = g.recording.find(*tick).expect("kept");
        g.replay.as_mut().unwrap().seek(i as f32, len);
        g.update(&pad(0), TICK);
        assert_eq!(g.shown_figure(), *figure, "tick {tick}");
        assert!(
            close(g.rider_bones().unwrap(), pose, 2e-3),
            "the skeleton at tick {tick}"
        );
    }
    run(&mut g, 1, pad(btn::X));
    assert!(g.replay.is_none());
    assert_eq!(g.rider_bones().unwrap(), &live_z[..], "the live pose back");
    assert_eq!(g.skater, skater_z);
    assert_eq!(g.graph.as_ref().unwrap().runner.motion_state(), path_z);
}

#[test]
fn imported_the_graphs_follow_the_marker_back_on_the_board_and_on_foot() {
    let Some(dir) = data_dir() else {
        eprintln!("markers_replay: no imported data (FG_DATA): skipped");
        return;
    };
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    let mut g = imported(&dir);
    g.place_rider(v3(-7.0, 0.0, -18.0), 0.0, 3.0);
    run(&mut g, 30, pad(0));
    let ev = chord(&mut g, btn::DPAD_UP, 1);
    assert!(ev.contains(&Event::MarkerSet), "{ev:?}");
    run(&mut g, 60, pad(btn::A));
    let ev = chord(&mut g, btn::DPAD_DOWN, 20);
    assert!(ev.contains(&Event::MarkerReturned), "{ev:?}");
    run(&mut g, 30, pad(0));
    assert!(g.skater.rolling());
    assert!(path(&g).starts_with("OnBoard/"), "{}", path(&g));
    // On foot.
    run(&mut g, 1, pad(btn::Y));
    run(&mut g, 120, pad(0));
    if !g.skater.walking() {
        eprintln!("markers_replay: the motion pack has no off-board clips: skipped");
        return;
    }
    let ev = chord(&mut g, btn::DPAD_UP, 1);
    assert!(ev.contains(&Event::MarkerSet), "{ev:?}");
    assert!(!g.markers.marker.unwrap().on_board);
    let walk = PadRaw {
        stick_y: 100,
        ..pad(0)
    };
    run(&mut g, 90, walk);
    let ev = chord(&mut g, btn::DPAD_DOWN, 20);
    assert!(ev.contains(&Event::MarkerReturned), "{ev:?}");
    run(&mut g, 30, pad(0));
    assert!(g.skater.walking());
    assert!(path(&g).starts_with("OffBoard/"), "{}", path(&g));
}
