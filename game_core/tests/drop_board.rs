//! Dropping, throwing and picking up the board off the board, and the caveman (Skate 3's
//! SkateboardController possession and LandingOnDeck: `skater::possession`), on the built-in park without
//! imported data, and with it through Skate 3's own graphs (ToggleBoard, `Mount/Air/IntoLandingOnBoard`):
//!
//! - L fully in drops the board at the rider's side; R throws it down ahead, rolling (18 degrees up at 3
//!   m/s standing, `ThrownSkateboardVelocity`).
//! - Walked away from, it stays where it is; L or R again flies it back to the hands
//!   (`RetrievalTimeVsDist`); Y next to it steps onto it where it lies, further away Y picks it up
//!   first (from 5 m at most, in 0.1 s: `DistanceForMountedReturn`, `MinTimeToRetrieveBoardWhileMounting`).
//! - Running with it dropped beside him and jumping, the landing assist brings the rider down onto it
//!   riding (the caveman).

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::{Vec3, v3};
use game_core::skater::possession::Hold;
use game_core::skater::{Event, Mode};

const TICK: f32 = 1.0 / 60.0;
/// A clear lane across the built-in park (nothing in the way along +z for 38 m).
const LANE: Vec3 = Vec3 {
    x: -7.0,
    y: 0.0,
    z: -18.0,
};

#[derive(Clone, Copy, Default)]
struct In {
    stick: [i8; 2],
    buttons: u32,
    l: u8,
    r: u8,
}

fn pad(i: In) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        stick_x: i.stick[0],
        stick_y: i.stick[1],
        buttons: i.buttons,
        trig_l: i.l,
        trig_r: i.r,
        ..Default::default()
    }
}

fn run(g: &mut Game, ticks: usize, i: In) {
    for _ in 0..ticks {
        g.update(&pad(i), TICK);
    }
}

fn idle(g: &mut Game, ticks: usize) {
    run(g, ticks, In::default());
}

const L: In = In {
    stick: [0, 0],
    buttons: 0,
    l: 255,
    r: 0,
};
const R: In = In {
    stick: [0, 0],
    buttons: 0,
    l: 0,
    r: 255,
};
const Y: In = In {
    stick: [0, 0],
    buttons: btn::Y,
    l: 0,
    r: 0,
};

/// The stick (full) that walks the rider towards `target`, read against the camera as the walker reads
/// it (`biped::world_stick`).
fn toward(g: &Game, target: Vec3) -> [i8; 2] {
    let d = (target - g.skater.pos)
        .horizontal()
        .normalize_or(Vec3::ZERO);
    let yaw = g.skater.cam_yaw;
    let (f, r) = (
        v3(yaw.sin(), 0.0, yaw.cos()),
        v3(-yaw.cos(), 0.0, yaw.sin()),
    );
    [(d.dot(r) * 100.0) as i8, (d.dot(f) * 100.0) as i8]
}

/// On foot at `pos` facing `heading`, the board in hand, the camera settled.
fn on_foot(g: &mut Game, pos: Vec3, heading: f32) {
    g.place_rider(pos, heading, 0.0);
    idle(g, 30);
    run(g, 1, Y);
    idle(g, 120);
    assert_eq!(g.skater.mode, Mode::Walk, "off the board");
    assert_eq!(g.skater.possession.hold, Hold::Held, "the board in hand");
}

/// Presses a trigger for a moment and lets it go.
fn press(g: &mut Game, i: In) {
    run(g, 2, i);
    idle(g, 1);
}

#[test]
fn l_drops_the_board_beside_him_where_it_stays() {
    let mut g = Game::new();
    on_foot(&mut g, LANE, 0.0);
    press(&mut g, L);
    assert_eq!(g.skater.possession.hold, Hold::Free, "L lets go of it");
    idle(&mut g, 60);
    let b = g.skater.possession.board;
    assert!(b.on_ground, "it falls onto its wheels");
    assert!(b.vel.length() < 0.01, "and stays: {:?}", b.vel);
    let off = (b.pos - g.skater.pos).horizontal().length();
    assert!(off < 0.6, "at his side: {off} m");
    assert_eq!(g.skater.mode, Mode::Walk, "still on foot");
}

#[test]
fn r_throws_it_down_ahead_rolling() {
    let mut g = Game::new();
    on_foot(&mut g, LANE, 0.0);
    let start = g.skater.pos;
    press(&mut g, R);
    let p = &g.skater.possession;
    assert_eq!(p.hold, Hold::Free, "R throws it");
    // Thrown at 18 degrees up at 3 m/s (standing): 2.85 m/s ahead, still rising a few ticks on.
    let v = p.board.vel;
    assert!((v.z - 3.0 * 18f32.to_radians().cos()).abs() < 0.05, "{v:?}");
    assert!(v.y > 0.0 && v.x.abs() < 1e-3, "ahead and up: {v:?}");
    let mut landed = None;
    for _ in 0..90 {
        idle(&mut g, 1);
        let b = g.skater.possession.board;
        if b.on_ground && landed.is_none() {
            landed = Some(b.pos.z - start.z);
        }
    }
    let landed = landed.expect("it comes down");
    let b = g.skater.possession.board;
    assert!(landed > 0.5, "ahead of him: {landed}");
    assert!(
        b.pos.z - start.z > landed + 2.0,
        "rolls on: {}",
        b.pos.z - start.z
    );
    assert!(
        b.vel.z > 1.0,
        "still rolling after a second and a half: {:?}",
        b.vel
    );
}

#[test]
fn walked_away_from_and_come_back_to_he_steps_onto_it_where_it_lies() {
    let mut g = Game::new();
    on_foot(&mut g, LANE, 0.0);
    press(&mut g, L);
    idle(&mut g, 40);
    let board = g.skater.possession.board.pos;
    // Away from it...
    run(
        &mut g,
        60,
        In {
            stick: [0, 100],
            ..In::default()
        },
    );
    let away = (g.skater.pos - board).horizontal().length();
    assert!(away > 4.0, "walked {away} m away");
    assert_eq!(g.skater.possession.hold, Hold::Free, "it stays behind");
    assert!((g.skater.possession.board.pos - board).length() < 0.01);
    // ...and back to it.
    for _ in 0..400 {
        if (g.skater.pos - board).horizontal().length() < 0.8 {
            break;
        }
        let stick = toward(&g, board);
        run(
            &mut g,
            1,
            In {
                stick,
                ..In::default()
            },
        );
    }
    idle(&mut g, 30);
    assert!(
        (g.skater.pos - board).horizontal().length() < 1.5,
        "back at it"
    );
    run(&mut g, 1, Y);
    assert_eq!(g.skater.mode, Mode::Rolling, "Y gets on");
    assert!(g.events.contains(&Event::SteppedOn));
    assert_eq!(g.skater.possession.hold, Hold::Underfoot);
    let at = (g.skater.pos - board).horizontal().length();
    assert!(
        at < 0.01,
        "onto the board where it lay, not one under his feet: {at}"
    );
    // Facing back the way he came: the board's tail is taken as its nose.
    assert!(g.skater.fwd.z < -0.9, "{:?}", g.skater.fwd);
    // And rides it: A pushes (towards the park's end, so only so far).
    let mut fastest: f32 = 0.0;
    for _ in 0..40 {
        run(
            &mut g,
            1,
            In {
                buttons: btn::A,
                ..In::default()
            },
        );
        fastest = fastest.max(g.skater.speed);
    }
    assert_eq!(g.skater.mode, Mode::Rolling);
    assert!(fastest > 1.0, "pushes off: {fastest}");
}

#[test]
fn far_from_it_l_flies_it_back_to_the_hands() {
    let mut g = Game::new();
    on_foot(&mut g, LANE, 0.0);
    press(&mut g, L);
    idle(&mut g, 30);
    run(
        &mut g,
        150,
        In {
            stick: [0, 100],
            ..In::default()
        },
    );
    idle(&mut g, 60);
    let d = (g.skater.possession.board.pos - g.skater.pos).length();
    assert!(d > 10.0, "{d} m away");
    press(&mut g, L);
    assert_eq!(
        g.skater.possession.hold,
        Hold::Returning,
        "L asks for it back"
    );
    let t = g.skater.tuning.possession.retrieval_time.eval(d);
    let mut ticks = 3;
    while g.skater.possession.hold == Hold::Returning && ticks < 200 {
        idle(&mut g, 1);
        ticks += 1;
    }
    assert_eq!(g.skater.possession.hold, Hold::Held, "back in his hands");
    let took = ticks as f32 * TICK;
    assert!(
        (took - t).abs() < 0.1,
        "took {took} s, Skate 3's curve says {t}"
    );
    // In hand again: R throws it.
    press(&mut g, R);
    assert_eq!(g.skater.possession.hold, Hold::Free);
}

#[test]
fn y_far_from_it_picks_it_up_on_the_way_on() {
    let mut g = Game::new();
    on_foot(&mut g, LANE, 0.0);
    press(&mut g, L);
    idle(&mut g, 30);
    run(
        &mut g,
        90,
        In {
            stick: [0, 100],
            ..In::default()
        },
    );
    idle(&mut g, 60);
    assert!((g.skater.possession.board.pos - g.skater.pos).length() > 5.0);
    let here = g.skater.pos;
    run(&mut g, 1, Y);
    assert_eq!(
        g.skater.possession.hold,
        Hold::Returning,
        "getting on fetches it"
    );
    let mut on = None;
    for i in 0..30 {
        idle(&mut g, 1);
        if g.skater.mode == Mode::Rolling {
            on = Some(i);
            break;
        }
    }
    let on = on.expect("on the board");
    assert!(on <= 8, "within Skate 3's 0.1 s: {on} ticks");
    assert!(
        (g.skater.pos - here).horizontal().length() < 0.5,
        "where he stood"
    );
}

#[test]
fn far_enough_away_it_is_hidden_until_picked_up() {
    let mut g = Game::new();
    on_foot(&mut g, LANE, 0.0);
    press(&mut g, L);
    idle(&mut g, 30);
    // 30 m away (`MaxDistance`) it is out of play.
    g.skater.pos += v3(0.0, 0.0, 31.0);
    idle(&mut g, 2);
    assert_eq!(g.skater.possession.hold, Hold::Hidden);
    assert!(
        g.free_board_frame().is_some_and(|f| f.0.y < -100.0),
        "out of sight"
    );
    press(&mut g, R);
    assert_eq!(g.skater.possession.hold, Hold::Returning);
    idle(&mut g, 70);
    assert_eq!(g.skater.possession.hold, Hold::Held);
}

#[test]
fn caveman_running_with_it_dropped_a_jump_lands_on_it_riding() {
    let mut g = Game::new();
    on_foot(&mut g, LANE, 0.0);
    let ahead = In {
        stick: [0, 100],
        ..In::default()
    };
    run(&mut g, 45, ahead);
    assert!(g.skater.biped.speed > 5.0, "running");
    // Dropped on the run: it rolls along beside him.
    run(&mut g, 2, In { l: 255, ..ahead });
    run(&mut g, 30, ahead);
    let b = g.skater.possession.board;
    assert_eq!(g.skater.possession.hold, Hold::Free);
    assert!(
        b.on_ground && b.vel.z > 4.0,
        "rolling with him: {:?}",
        b.vel
    );
    // Jump: the landing assist takes him down onto it.
    run(
        &mut g,
        1,
        In {
            buttons: btn::B,
            ..ahead
        },
    );
    let mut caveman = false;
    let mut riding = None;
    for i in 0..90 {
        run(&mut g, 1, ahead);
        caveman |= g.skater.landing_on_free_board();
        if g.skater.mode == Mode::Rolling {
            riding = Some(i);
            break;
        }
    }
    assert!(caveman, "comes down onto the board (landing on deck)");
    assert!(riding.is_some(), "riding it: {:?}", g.skater.mode);
    assert_eq!(g.skater.possession.hold, Hold::Underfoot);
    let at = (g.skater.pos - g.skater.possession.board.pos)
        .horizontal()
        .length();
    assert!(at < 0.6, "on it: {at}");
    assert!(g.skater.speed > 3.0, "riding on: {}", g.skater.speed);
}

#[test]
fn y_in_the_air_with_the_board_in_hand_still_gets_on_there() {
    let mut g = Game::new();
    on_foot(&mut g, LANE, 0.0);
    run(
        &mut g,
        30,
        In {
            stick: [0, 100],
            ..In::default()
        },
    );
    run(
        &mut g,
        1,
        In {
            stick: [0, 100],
            buttons: btn::B,
            ..In::default()
        },
    );
    run(&mut g, 8, In::default());
    assert!(g.skater.biped.air.is_some());
    run(&mut g, 1, Y);
    assert!(g.skater.landing_on_deck(), "{:?}", g.skater.mode);
    idle(&mut g, 60);
    assert_eq!(g.skater.mode, Mode::Rolling);
    assert_eq!(g.skater.possession.hold, Hold::Underfoot);
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

/// With the player's imported data, off the board (`None`: no data, or a motion pack without the
/// off-board or ToggleBoard clips).
fn imported_on_foot() -> Option<Game> {
    let dir = data_dir()?;
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
    g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
    let lib = g.motion.as_ref()?;
    let posed = |n: &str| lib.db.clip_index(n).is_some_and(|i| lib.pose(i).is_some());
    if !posed("OFB_THROW_90") || !posed("NB_WALK_FWD_CYC") {
        eprintln!(
            "drop_board: the motion pack has no ToggleBoard clips (import it again): skipped"
        );
        return None;
    }
    g.place_rider(LANE, 0.0, 0.0);
    idle(&mut g, 30);
    run(&mut g, 1, Y);
    idle(&mut g, 120);
    if g.skater.mode != Mode::Walk {
        return None;
    }
    Some(g)
}

fn path(g: &Game) -> String {
    g.graph.as_ref().unwrap().runner.motion_state()
}

fn retrieve_channel(g: &Game) -> bool {
    g.graph
        .as_ref()
        .unwrap()
        .runner
        .s
        .player
        .has_channel(game_core::graph::host::CHANNEL_RETRIEVE)
}

/// With the player's imported data: L plays Skate 3's drop (`OFB_THROW_90` on the `RetrieveBoard`
/// channel) and the board leaves the hands at its `OB_RELEASEBOARD`, not at once; the graphs then walk
/// him without it (`NoBoard`). L again plays the retrieve clips, the board flies back, and he carries it
/// again (`WithBoard`); R throws it (`OFB_THROW_0`'s `OB_THROWBOARD`).
#[test]
fn imported_toggle_board_drops_retrieves_and_throws() {
    let Some(mut g) = imported_on_foot() else {
        eprintln!("drop_board: no imported data (FG_DATA): skipped");
        return;
    };
    run(&mut g, 2, L);
    assert!(retrieve_channel(&g), "the drop clip plays");
    assert_eq!(g.skater.possession.hold, Hold::Held, "not let go at once");
    let mut released = None;
    for i in 0..60 {
        idle(&mut g, 1);
        if released.is_none() && g.skater.possession.hold == Hold::Free {
            released = Some(i);
        }
    }
    let released = released.expect("let go at the clip's release");
    assert!(released > 10, "at OB_RELEASEBOARD (0.4 s in): {released}");
    idle(&mut g, 30);
    assert!(path(&g).contains("/NoBoard"), "{}", path(&g));
    // Walk off a little, then L: the retrieve clips, the board back in hand.
    run(
        &mut g,
        40,
        In {
            stick: [0, 100],
            ..In::default()
        },
    );
    idle(&mut g, 30);
    run(&mut g, 2, L);
    let (mut retrieving, mut held) = (false, None);
    for i in 0..150 {
        idle(&mut g, 1);
        retrieving |= retrieve_channel(&g);
        if held.is_none() && g.skater.possession.held() {
            held = Some(i);
        }
    }
    assert!(retrieving, "the retrieve clips play");
    assert!(
        held.is_some(),
        "the board comes back: {:?}",
        g.skater.possession.hold
    );
    assert!(path(&g).contains("/WithBoard"), "{}", path(&g));
    // R: the throw.
    run(&mut g, 2, R);
    let mut thrown = false;
    for _ in 0..60 {
        idle(&mut g, 1);
        thrown |= g.skater.possession.hold == Hold::Free;
    }
    assert!(thrown, "R throws it");
    assert!(
        g.skater.possession.board.pos.z > g.skater.pos.z + 1.0,
        "ahead"
    );
}

/// With the player's imported data: Y next to the dropped board plays Skate 3's mount (getting on
/// fetches the board into the hands, `OB_Mounting`) and rides on.
#[test]
fn imported_y_next_to_the_dropped_board_gets_on() {
    let Some(mut g) = imported_on_foot() else {
        eprintln!("drop_board: no imported data (FG_DATA): skipped");
        return;
    };
    run(&mut g, 2, L);
    idle(&mut g, 90);
    assert_eq!(g.skater.possession.hold, Hold::Free);
    run(&mut g, 1, Y);
    let (mut mounting, mut on) = (false, false);
    for _ in 0..120 {
        idle(&mut g, 1);
        mounting |= path(&g).starts_with("Mount/");
        on |= g.skater.mode == Mode::Rolling;
    }
    assert!(mounting, "the mount plays: {}", path(&g));
    assert!(on && path(&g).starts_with("OnBoard/"), "{}", path(&g));
    assert_eq!(g.skater.possession.hold, Hold::Underfoot);
}

/// With the player's imported data: running with the board dropped beside him, a jump comes down onto
/// it through Skate 3's landing on deck (`Mount/Air/IntoLandingOnBoard` into `OnBoard/LandingOnBoard`,
/// its LandOnBoard) and rides on, never wiping out.
#[test]
fn imported_caveman_lands_on_the_board() {
    let Some(mut g) = imported_on_foot() else {
        eprintln!("drop_board: no imported data (FG_DATA): skipped");
        return;
    };
    g.skater.pos = LANE;
    let ahead = In {
        stick: [0, 100],
        ..In::default()
    };
    run(&mut g, 45, ahead);
    run(&mut g, 2, In { l: 255, ..ahead });
    for _ in 0..60 {
        run(&mut g, 1, ahead);
        if g.skater.possession.hold == Hold::Free && g.skater.possession.board.on_ground {
            break;
        }
    }
    assert_eq!(g.skater.possession.hold, Hold::Free, "dropped on the run");
    run(
        &mut g,
        1,
        In {
            buttons: btn::B,
            ..ahead
        },
    );
    let (mut into, mut landing, mut riding) = (false, false, false);
    for _ in 0..120 {
        run(&mut g, 1, ahead);
        let p = path(&g);
        assert!(!p.starts_with("WipeOut"), "{p}");
        into |= p.starts_with("Mount/Air/IntoLandingOnBoard");
        landing |= p.starts_with("OnBoard/LandingOnBoard");
        riding |= g.skater.mode == Mode::Rolling && p.starts_with("OnBoard/");
    }
    assert!(
        into && landing,
        "Skate 3's landing on the board plays: {}",
        path(&g)
    );
    assert!(riding, "riding: {:?} {}", g.skater.mode, path(&g));
}
