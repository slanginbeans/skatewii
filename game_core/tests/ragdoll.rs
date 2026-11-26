//! Ragdoll bails (`skater::ragdoll`, `skater::wipeout`), bail steering and Hall of Meat
//! (`skater::meat`) through the real input path, on built-in parks without imported data:
//!
//! - L and R fully in with Z (Skate 3's both triggers with both sticks clicked in) bails on purpose,
//!   riding or on foot; either alone doesn't.
//! - Bailing off a 3 m ledge, the body falls, lands on the ground below without sinking into it or
//!   flying apart, settles and is put back.
//! - The main stick steers the fall (with a gesture that drifts: the swan dive, C-stick down), so he
//!   ends up somewhere else; the C-stick picks Skate 3's profiles.
//! - A big fall hurts (Hall of Meat dollars and broken bones on the HUD), a slow bail doesn't.
//! - The replay records the ragdoll's parts and plays them back.
//! - With the player's imported data, Skate 3's graphs play their wipeout states over the ragdoll, which
//!   poses the imported skeleton.

mod common;

use game_core::Game;
use game_core::input::{Pad, PadRaw, btn, flag};
use game_core::math::{Vec3, v3};
use game_core::park::{Park, Shape, Terrain};
use game_core::skater::Event;
use game_core::skater::ragdoll::{PELVIS, RADIUS};

const TICK: f32 = 1.0 / 60.0;

/// A 3 m ledge 4 m wide, from z = 0 to 10, on flat ground.
fn ledge() -> Park {
    Park {
        shapes: vec![Shape::Block {
            min: v3(-2.0, 0.0, 0.0),
            max: v3(2.0, 3.0, 10.0),
            rgba: 0x808080ff,
        }],
        half_size: 60.0,
        wall_height: 2.0,
        spawn: v3(0.0, 0.0, -30.0),
        spawn_yaw: 0.0,
        rails: Vec::new(),
    }
}

fn flat() -> Park {
    Park {
        shapes: Vec::new(),
        half_size: 100.0,
        wall_height: 2.0,
        spawn: v3(0.0, 0.0, -80.0),
        spawn_yaw: 0.0,
        rails: Vec::new(),
    }
}

#[derive(Clone, Copy, Default)]
struct In {
    stick: (i8, i8),
    cstick: (i8, i8),
    triggers: bool,
    buttons: u32,
}

fn pad(i: In) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        stick_x: i.stick.0,
        stick_y: i.stick.1,
        cstick_x: i.cstick.0,
        cstick_y: i.cstick.1,
        trig_l: if i.triggers { 255 } else { 0 },
        trig_r: if i.triggers { 255 } else { 0 },
        buttons: i.buttons,
        ..Default::default()
    }
}

fn run(g: &mut Game, frames: usize, i: In) -> Vec<Event> {
    let mut ev = Vec::new();
    for _ in 0..frames {
        // Without imported data a synthetic graph's Wipeout behaviour: the C-stick as the gesture.
        if g.graph.is_none() {
            g.skater.graph.wipe_gesture = Pad::from_raw(&pad(i), 0).flick;
        }
        g.update(&pad(i), TICK);
        ev.extend(g.events.iter().cloned());
    }
    ev
}

/// The bail chord: both triggers fully in with Z, a frame, then everything let go.
const CHORD: In = In {
    stick: (0, 0),
    cstick: (0, 0),
    triggers: true,
    buttons: btn::Z,
};

fn riding_on_the_ledge(speed: f32) -> Game {
    let mut g = Game::new();
    g.park = ledge();
    g.place_rider(v3(0.0, 3.0, 1.0), 0.0, speed);
    run(&mut g, 10, In::default());
    g
}

/// Rides off the ledge and bails in the air with `held` held from the chord on.
fn bail_off_the_ledge(held: In) -> (Game, Vec<Event>) {
    let mut g = riding_on_the_ledge(6.0);
    for _ in 0..200 {
        run(&mut g, 1, In::default());
        if g.skater.is_airborne() {
            break;
        }
    }
    assert!(g.skater.is_airborne(), "{:?}", g.skater.state());
    let mut ev = run(
        &mut g,
        1,
        In {
            stick: held.stick,
            cstick: held.cstick,
            ..CHORD
        },
    );
    assert!(ev.contains(&Event::Bailed), "{ev:?}");
    assert_eq!(g.skater.bail_reason, Some("bailed on purpose"));
    ev.extend(run(&mut g, 1, held));
    (g, ev)
}

/// No point in the ground, none flown off.
fn check_body(g: &Game, park: &dyn Terrain) {
    let b = &g.skater.wipeout.body;
    for (i, &p) in b.pos.iter().enumerate() {
        let ground = park.ground(p.x, p.z, p.y + 0.5).height;
        assert!(
            p.y - RADIUS[i] > ground - 0.04,
            "point {i} in the ground: {p:?} over {ground}"
        );
        assert!(
            (p - b.pos[PELVIS]).length() < 2.0,
            "point {i} flew off: {p:?} {:?}",
            b.pos[PELVIS]
        );
    }
    assert!(b.com_vel(TICK).length() < 20.0);
}

#[test]
fn l_and_r_fully_in_with_z_bails_on_purpose() {
    let mut g = Game::new();
    g.park = flat();
    g.place_rider(v3(0.0, 0.0, -60.0), 0.0, 5.0);
    run(&mut g, 20, In::default());
    // Z alone, the triggers alone, half way in with Z: riding on.
    for i in [
        In {
            buttons: btn::Z,
            ..In::default()
        },
        In {
            triggers: true,
            ..In::default()
        },
    ] {
        let ev = run(&mut g, 5, i);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        run(&mut g, 2, In::default());
    }
    let mut half = pad(CHORD);
    half.trig_l = 150;
    for _ in 0..5 {
        g.update(&half, TICK);
        assert!(!g.events.contains(&Event::Bailed));
    }
    run(&mut g, 2, In::default());
    assert!(g.skater.rolling());
    let ev = run(&mut g, 1, CHORD);
    assert!(ev.contains(&Event::Bailed), "{ev:?}");
    assert!(g.skater.ragdoll_active());
    // The board lets go and goes its own way.
    assert!(g.skater.possession.free());
    // Held on, nothing more happens; let go, the body lies there and he is put back on his own.
    run(&mut g, 30, CHORD);
    let ev = run(&mut g, 60 * 25, In::default());
    assert!(ev.contains(&Event::Respawned), "put back after the bail");
    assert!(g.skater.rolling());

    // On foot too.
    let mut g = Game::new();
    g.park = flat();
    g.place_rider(v3(0.0, 0.0, -60.0), 0.0, 0.0);
    run(&mut g, 20, In::default());
    common::get_off(&mut g);
    run(&mut g, 30, In::default());
    assert!(g.skater.walking());
    let ev = run(&mut g, 1, CHORD);
    assert!(ev.contains(&Event::Bailed), "{ev:?}");
    assert!(g.skater.ragdoll_active());
}

#[test]
fn a_bail_off_a_ledge_lands_the_body_on_the_ground_below() {
    let (mut g, _) = bail_off_the_ledge(In::default());
    let park = ledge();
    let mut landed = None;
    let mut ev = Vec::new();
    for t in 0..60 * 20 {
        ev.extend(run(&mut g, 1, In::default()));
        if !g.skater.ragdoll_active() {
            break;
        }
        check_body(&g, &park);
        let pelvis = g.skater.wipeout.body.pos[PELVIS];
        if landed.is_none() && pelvis.y < 0.6 && g.skater.wipeout.body.contacts.any() {
            landed = Some(t);
        }
        // The rider's place (the camera's subject, the reset's spot) follows the body.
        assert!((g.skater.pos.horizontal() - pelvis.horizontal()).length() < 1e-3);
    }
    let landed = landed.expect("the body came down off the ledge");
    assert!(landed < 90, "down in {landed} ticks");
    assert!(ev.contains(&Event::Respawned), "settled and put back");
}

#[test]
fn the_stick_steers_where_the_bail_ends_up() {
    // Swan dive (C-stick down): the main stick drifts the fall across.
    let end = |x: i8| {
        let (mut g, _) = bail_off_the_ledge(In {
            stick: (x, 0),
            cstick: (0, -80),
            ..In::default()
        });
        let held = In {
            stick: (x, 0),
            cstick: (0, -80),
            ..In::default()
        };
        for _ in 0..90 {
            run(&mut g, 1, held);
            check_body(&g, &ledge());
        }
        assert_eq!(g.skater.wipeout.profile, 3, "swan dive");
        g.skater.wipeout.body.pos[PELVIS]
    };
    let (left, right, straight) = (end(-100), end(100), end(0));
    assert!(
        (left - right).horizontal().length() > 0.8,
        "{left:?} {right:?}"
    );
    assert!((left - straight).horizontal().length() > 0.3);
    // The C-stick up tucks him (Skate 3's cannon ball): the drives pull the knees up.
    let (mut g, _) = bail_off_the_ledge(In {
        cstick: (0, 80),
        ..In::default()
    });
    run(
        &mut g,
        30,
        In {
            cstick: (0, 80),
            ..In::default()
        },
    );
    assert_eq!(g.skater.wipeout.profile, 1, "cannon ball");
    assert!(g.skater.wipeout.drive > 0.0);
}

#[test]
fn a_big_fall_hurts_and_shows_in_the_hall_of_meat() {
    let (mut g, _) = bail_off_the_ledge(In::default());
    run(&mut g, 120, In::default());
    let (meat, _) = g.meat.expect("a Hall of Meat result");
    assert!(meat.hits > 0 && meat.score() > 0, "{meat:?}");
    let ledge_score = meat.score();
    // Kept on screen after he is put back, then gone.
    let ev = run(&mut g, 60 * 20, In::default());
    assert!(ev.contains(&Event::Respawned));
    let _ = ev;
    // A slow bail on flat ground hardly hurts (falling over from standing knocks the head a little).
    let mut g = Game::new();
    g.park = flat();
    g.place_rider(v3(0.0, 0.0, -60.0), 0.0, 1.0);
    run(&mut g, 20, In::default());
    run(&mut g, 1, CHORD);
    run(&mut g, 120, In::default());
    let (meat, _) = g.meat.expect("a result for the bail");
    assert!(
        meat.score() * 4 < ledge_score && meat.bones() == 0,
        "{meat:?} against {ledge_score}"
    );
    // From 8 m bones break.
    let mut g = Game::new();
    g.park = Park {
        shapes: vec![Shape::Block {
            min: v3(-2.0, 0.0, 0.0),
            max: v3(2.0, 8.0, 10.0),
            rgba: 0x808080ff,
        }],
        ..ledge()
    };
    g.place_rider(v3(0.0, 8.0, 4.0), 0.0, 8.0);
    for _ in 0..200 {
        run(&mut g, 1, In::default());
        if g.skater.is_airborne() {
            break;
        }
    }
    run(&mut g, 1, CHORD);
    run(&mut g, 150, In::default());
    let (meat, _) = g.meat.unwrap();
    assert!(meat.bones() > 0, "{meat:?}");
    assert!(meat.broken_names().count() as u32 == meat.bones());
}

#[test]
fn the_ragdoll_is_the_same_tick_by_tick() {
    let a = bail_off_the_ledge(In {
        stick: (60, 30),
        cstick: (0, 80),
        ..In::default()
    });
    let b = bail_off_the_ledge(In {
        stick: (60, 30),
        cstick: (0, 80),
        ..In::default()
    });
    let (mut a, mut b) = (a.0, b.0);
    for _ in 0..120 {
        run(&mut a, 1, In::default());
        run(&mut b, 1, In::default());
        assert_eq!(a.skater.wipeout, b.skater.wipeout);
    }
}

#[test]
fn the_replay_plays_the_ragdoll_back() {
    let (mut g, _) = bail_off_the_ledge(In::default());
    run(&mut g, 40, In::default());
    let n = g.recording.len();
    assert_eq!(
        g.recording.bone_count(),
        game_core::skater::ragdoll::PARTS,
        "the placeholder records its parts"
    );
    let last = g.recording.header(n - 1).unwrap();
    assert!(last.figure.ragdoll);
    let parts: Vec<_> = g.recording.bones(n - 1).to_vec();
    // Z + d-pad left: the replay draws the recorded parts.
    run(
        &mut g,
        1,
        In {
            buttons: btn::Z,
            ..In::default()
        },
    );
    run(
        &mut g,
        1,
        In {
            buttons: btn::Z | btn::DPAD_LEFT,
            ..In::default()
        },
    );
    assert!(g.replay.is_some());
    let f = g.shown_figure();
    assert!(f.ragdoll);
    let parts_now = g.recording.bones(g.recording.len() - 1);
    assert_eq!(parts_now.len(), parts.len());
    let rows = game_core::replay::unpack(&parts_now[0]);
    assert!(rows[3].iter().all(|v| v.is_finite()));
}

#[test]
fn the_ragdoll_costs_little_per_tick() {
    // Host timing of the bail's body (the ragdoll, its control, collision, landing prediction and
    // injuries); the Wii logs the skater's whole step in a bail as `ragdoll` in its frame line.
    let (g, _) = bail_off_the_ledge(In::default());
    let park = ledge();
    let t = g.skater.tuning;
    let mut w = g.skater.wipeout;
    let input = game_core::skater::wipeout::Input {
        control: [0.5, 0.2],
        gesture: [0.0, -1.0],
        push_off: false,
    };
    let n = 600;
    let start = std::time::Instant::now();
    for _ in 0..n {
        w.step(&park, &t, input, TICK);
    }
    let per = start.elapsed().as_secs_f64() / n as f64;
    eprintln!("ragdoll: {:.1} us a tick on this host", per * 1e6);
    // Only bound it on the host: the PowerPC test run is emulated under qemu, far slower than a Wii.
    #[cfg(not(target_arch = "powerpc"))]
    assert!(per < 1e-3, "{per}");
}

// --- With the player's imported data. ---

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

fn imported(dir: &std::path::Path) -> Game {
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
    g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
    g
}

#[test]
fn imported_the_graphs_play_their_wipeout_over_the_ragdoll() {
    let Some(dir) = data_dir() else {
        eprintln!("ragdoll: no imported data (FG_DATA): skipped");
        return;
    };
    for gesture in [(0, 0), (0, 80)] {
        let mut g = imported(&dir);
        g.park = ledge();
        g.place_rider(v3(0.0, 3.0, 1.0), 0.0, 6.0);
        run(&mut g, 10, In::default());
        for _ in 0..200 {
            run(&mut g, 1, In::default());
            if g.skater.is_airborne() {
                break;
            }
        }
        let held = In {
            cstick: gesture,
            ..In::default()
        };
        let ev = run(
            &mut g,
            1,
            In {
                cstick: gesture,
                ..CHORD
            },
        );
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
        let mut states = Vec::new();
        let park = ledge();
        for _ in 0..90 {
            run(&mut g, 1, held);
            if !g.skater.ragdoll_active() {
                break;
            }
            check_body(&g, &park);
            let path = g.graph.as_ref().unwrap().runner.motion_state();
            if states.last() != Some(&path) {
                states.push(path);
            }
        }
        assert!(
            states.iter().any(|s| s.starts_with("WipeOut")),
            "Skate 3's wipeout states: {states:?}"
        );
        // The ragdoll holds the imported skeleton: its hips are where the body's pelvis is.
        let r = g.rider().unwrap();
        assert!(r.ragdoll_weight() > 0.99);
        let hips = r.character.bone("HIPS").unwrap();
        let at = r.character.world()[hips][3];
        let at = g.imported_xform().point(v3(at[0], at[1], at[2]));
        let pelvis = g.skater.wipeout.body.between(g.blend())[PELVIS];
        assert!((at - pelvis).length() < 0.15, "{at:?} {pelvis:?}");
        // The graphs' timers read the body, and the gesture is theirs.
        let w = &g.skater.wipeout;
        assert!(w.no_support_time >= 0.0);
        if gesture != (0, 0) {
            eprintln!(
                "ragdoll: imported gesture {gesture:?}: profile {} ({states:?})",
                w.profile
            );
        }
        let log: Vec<&String> = g
            .log
            .iter()
            .filter(|l| l.contains("unsupported") && l.contains("GestureType"))
            .collect();
        assert!(log.is_empty(), "{log:?}");
        let _: Vec<Vec3> = Vec::new();
    }
}
