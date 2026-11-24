//! Off the board in the air (Skate 3's off-board air body tweak and hippie hurdle):
//!
//! - Jumping on foot, the C-stick (let go first) plays Skate 3's air body tweak
//!   (`OffboardBodyTweakBlend`: `B_OBAIR_BODYTWEAK_*` on the `AirBodyTweak` channel); Z plays it
//!   centred; nothing pushed, it doesn't play.
//! - A hippie jump's landing on deck: on flatground's static parks it never hurdles; a moving obstacle
//!   met on the way down (fed to `skater::hurdle` here, as flatground has no moving objects) plays
//!   `OnBoard/HippyJump/HurdleCyc`, and he lands back on the board once it is over.

use game_core::Game;
use game_core::graph::host::CHANNEL_BODY_TWEAK;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::v3;
use game_core::skater::Mode;
use game_core::skater::hurdle::{Flight, Hit, Hurdle};

const TICK: f32 = 1.0 / 60.0;
/// A clear lane across the built-in park (as `tests/offboard.rs`).
const LANE: game_core::math::Vec3 = game_core::math::Vec3 {
    x: -7.0,
    y: 0.0,
    z: -18.0,
};

fn pad(stick: [i8; 2], cstick: [i8; 2], buttons: u32) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        stick_x: stick[0],
        stick_y: stick[1],
        cstick_x: cstick[0],
        cstick_y: cstick[1],
        buttons,
        ..Default::default()
    }
}

fn run(g: &mut Game, ticks: usize, stick: [i8; 2], cstick: [i8; 2], buttons: u32) {
    for _ in 0..ticks {
        g.update(&pad(stick, cstick, buttons), TICK);
    }
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

fn tweak_plays(g: &Game) -> bool {
    g.graph
        .as_ref()
        .unwrap()
        .runner
        .s
        .player
        .has_channel(CHANNEL_BODY_TWEAK)
}

/// Winds up and lets go of a hippie jump (A and B), rolling along the lane.
fn hippie_jump(g: &mut Game) {
    g.place_rider(LANE, 0.0, 5.0);
    run(g, 40, [0, 0], [0, 0], 0);
    run(g, 30, [0, 0], [0, 0], btn::A | btn::B);
    run(g, 1, [0, 0], [0, 0], 0);
    assert!(g.skater.hippie_air(), "the hippie jump is in the air");
}

#[test]
fn a_hippie_jump_on_the_static_park_never_hurdles() {
    let mut g = Game::new();
    hippie_jump(&mut g);
    let mut landed = false;
    for _ in 0..90 {
        run(&mut g, 1, [0, 0], [0, 0], 0);
        assert!(!g.skater.hurdle.hurdling());
        if g.skater.hippie_air() {
            assert!(
                !g.skater.hurdle.wants_query(),
                "the query settles the flight"
            );
        }
        landed |= g.skater.mode == Mode::Rolling;
    }
    assert!(landed, "back on the board");
}

/// With the player's imported data: jumping on foot, the C-stick (let go at take-off) plays Skate 3's
/// air body tweak on its channel, Z plays it centred, and with neither it doesn't play.
#[test]
fn imported_the_c_stick_tweaks_the_body_jumping_on_foot() {
    let Some(dir) = data_dir() else {
        eprintln!("offboard_air: no imported data (FG_DATA): skipped");
        return;
    };
    let probe = imported(&dir);
    if probe
        .motion
        .as_ref()
        .and_then(|m| m.db.find("B_OBAIR_BODYTWEAK_NB_INTO"))
        .is_none()
    {
        eprintln!("offboard_air: the motion pack has no air body tweak clips: skipped");
        return;
    }
    // (C-stick, buttons) held through the jump; whether the tweak should play.
    for (cstick, buttons, plays) in [
        ([0, 0], 0, false),
        ([60, 0], 0, true),
        ([0, -60], 0, true),
        ([0, 0], btn::Z, true),
    ] {
        let mut g = imported(&dir);
        g.place_rider(LANE, 0.0, 0.0);
        run(&mut g, 30, [0, 0], [0, 0], 0);
        run(&mut g, 1, [0, 0], [0, 0], btn::Y);
        run(&mut g, 120, [0, 0], [0, 0], 0);
        if g.skater.mode != Mode::Walk {
            eprintln!("offboard_air: the motion pack has no off-board clips: skipped");
            return;
        }
        g.skater.pos = LANE;
        run(&mut g, 40, [0, 100], [0, 0], 0);
        run(&mut g, 1, [0, 100], [0, 0], btn::B);
        // The C-stick centred into the air (a stick already pushed doesn't count until let go).
        run(&mut g, 4, [0, 100], [0, 0], 0);
        let (mut seen, mut in_air) = (false, false);
        for _ in 0..56 {
            run(&mut g, 1, [0, 100], cstick, buttons);
            let p = path(&g);
            if p.starts_with("OffBoard/OBAir/") {
                in_air = true;
                seen |= tweak_plays(&g);
            }
        }
        assert!(in_air, "the jump is in the air: {}", path(&g));
        // The tweak trees' parameters are set (none played unset).
        let db = &g.motion.as_ref().unwrap().db;
        let unset = &g.graph.as_ref().unwrap().runner.s.player.unset;
        for n in ["bodytweakx", "bodytweaky"] {
            let id = db.name_id(n);
            assert!(
                id.is_none() || !unset.iter().any(|u| Some(u.1) == id),
                "{n} unset"
            );
        }
        assert_eq!(
            seen,
            plays,
            "C-stick {cstick:?}, buttons {buttons:#x}: {}",
            path(&g)
        );
        // Down again: the channel fades out.
        run(&mut g, 60, [0, 0], [0, 0], 0);
        assert!(!tweak_plays(&g), "faded out on the ground: {}", path(&g));
    }
}

/// With the player's imported data: a moving obstacle met by a hippie jump's landing on deck (two
/// queries 0.05 m apart: 3 m/s) is Skate 3's hippie hurdle: the graph plays `HippyJump/HurdleCyc`, holds
/// the landing on the board off until 0.3 s past the apex, then lands him on the board riding.
#[test]
fn imported_a_moving_obstacle_hurdles_the_hippie_jump() {
    let Some(dir) = data_dir() else {
        eprintln!("offboard_air: no imported data (FG_DATA): skipped");
        return;
    };
    for hurdle in [false, true] {
        let mut g = imported(&dir);
        hippie_jump(&mut g);
        if hurdle {
            let mut h = Hurdle::default();
            let f = Flight {
                time_to_land: 0.5,
                landing: g.skater.pos + v3(0.0, 0.0, 2.5),
                descending: false,
                height: g.skater.pos.y,
            };
            for k in 0..2 {
                h.prepare(g.skater.pos);
                h.sync(
                    Some(Hit {
                        point: g.skater.pos + v3(0.0, 0.6, 1.0),
                        body: g.skater.pos + v3(-1.0 + 0.05 * k as f32, 0.0, 1.0),
                    }),
                    &f,
                );
            }
            assert!(h.hurdling());
            g.skater.hurdle = h;
        }
        let (mut hurdled, mut landed_riding) = (false, false);
        let mut hurdle_then_landing = false;
        for _ in 0..120 {
            run(&mut g, 1, [0, 0], [0, 0], 0);
            let p = path(&g);
            assert!(!p.starts_with("WipeOut"), "{p}");
            if p.starts_with("OnBoard/HippyJump/HurdleCyc") {
                hurdled = true;
                assert!(g.skater.hippie_air(), "hurdling in the air");
            }
            if hurdled && p.starts_with("OnBoard/HippyJump/LandingOnBoard") {
                hurdle_then_landing = true;
            }
            landed_riding |= g.skater.mode == Mode::Rolling && p.starts_with("OnBoard/");
        }
        assert_eq!(hurdled, hurdle, "hurdle {hurdle}: {}", path(&g));
        assert!(landed_riding, "hurdle {hurdle}: lands riding: {}", path(&g));
        if hurdle {
            assert!(
                hurdle_then_landing,
                "the landing on the board after the hurdle"
            );
        }
    }
}
