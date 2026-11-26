//! Board adjust in the air (Skate 3's BoardAdjust: the C-stick turns and tips the deck against the body),
//! on synthetic parks and without imported data:
//!
//! - Ollied onto a rail along the travel the board locks on its trucks; the C-stick held to the side
//!   swings the deck across the rail on the way up and the same jump is a boardslide.
//! - Coming down with the body crooked to the travel, a deck lined up with the travel lands straight
//!   (Skate 3's landing reads the board, not the body); left square to the body, the same landing is
//!   sketchy. Held across the travel all the way down, the board lands sideways.
//! - With the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`; skipped without it):
//!   holding the C-stick in the air puts Skate 3's motion graph in its board-adjust states and its clips
//!   pose the deck, which the physics follows.

use std::path::PathBuf;

use game_core::Game;
use game_core::flick::FlickOut;
use game_core::gesture::Flick;
use game_core::input::{Pad, PadRaw, flag};
use game_core::math::{PI, atan2f, v3};
use game_core::park::Park;
use game_core::rails::Rail;
use game_core::skater::adjust::DeckAdjust;
use game_core::skater::{Event, Family, Skater, Stance};

const TICK: f32 = 1.0 / 60.0;

/// Flat ground, the rider at z = -3 facing +z; with `rail`, a thin rail 0.5 m up along +z from z = 0.
fn park(rail: bool) -> Park {
    Park {
        shapes: Vec::new(),
        half_size: 60.0,
        wall_height: 2.0,
        spawn: v3(0.0, 0.0, -3.0),
        spawn_yaw: 0.0,
        rails: if rail {
            vec![Rail::new(vec![v3(0.0, 0.5, 0.0), v3(0.0, 0.5, 20.0)])]
        } else {
            Vec::new()
        },
    }
}

/// Rolls at 5 m/s, ollies and holds the C-stick at `stick` until the board is down (on a rail or the
/// ground), at most 90 ticks.
fn ollie(p: &Park, stick: [f32; 2]) -> (Skater, Vec<Event>) {
    let mut s = Skater::spawn(p);
    s.since_spawn = 10.0;
    s.speed = 5.0;
    let mut ev = Vec::new();
    // The state graph pops an ollie (its takeoff clip's jump).
    s.graph.pop = Some((Flick::OLLIE, 1.0));
    s.step(p, &Pad::default(), &FlickOut::default(), TICK, &mut ev);
    s.graph.pop = None;
    assert!(s.is_airborne(), "popped");
    let pad = Pad {
        flick: stick,
        ..Pad::default()
    };
    // The action graph's BoardAdjust runs while the C-stick is held.
    s.graph.board_adjust = stick != [0.0, 0.0];
    for _ in 0..90 {
        s.step(p, &pad, &FlickOut::default(), TICK, &mut ev);
        if !s.is_airborne() {
            break;
        }
    }
    (s, ev)
}

#[test]
fn adjusting_the_board_across_a_rail_turns_a_truck_grind_into_a_boardslide() {
    let p = park(true);
    // Square to the body: onto the trucks (it takes the leading truck first, a 5-0, from the air).
    let (s, ev) = ollie(&p, [0.0, 0.0]);
    let family = s.grind.as_ref().map(|g| g.family);
    assert!(
        matches!(family, Some(Family::FiftyFifty | Family::FiveO)),
        "{family:?} {ev:?}"
    );
    // The C-stick held left (or right): the deck swings across the rail and slides on it.
    for x in [-1.0, 1.0] {
        let (s, ev) = ollie(&p, [x, 0.0]);
        assert!(s.grinding(), "{ev:?}");
        let g = s.grind.as_ref().unwrap();
        assert_eq!(
            g.family,
            Family::Boardslide,
            "stick {x}: {}",
            g.graph_name()
        );
        // The deck the family was picked from is the board now: across the rail.
        assert!(s.fwd.z.abs() < 0.3, "stick {x}: nose {:?}", s.fwd);
        assert!(s.deck_adjust.is_none(), "baked in on the rail");
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    }
}

#[test]
fn a_board_lined_up_with_the_travel_lands_clean_where_the_body_would_not() {
    // Coming down 25 cm over flat ground at 5 m/s along +z, the body turned 40 degrees off the travel.
    let land = |lined_up: bool| {
        let p = park(false);
        let mut s = Skater::spawn(&p);
        s.since_spawn = 10.0;
        s.put_at(&p, v3(0.0, 0.25, 0.0), 40.0 * PI / 180.0, 0.0);
        s.force_state(game_core::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        s.vel = v3(0.0, -2.5, 5.0);
        // The C-stick right swings the nose back towards the travel by 40 degrees (of Skate 3's 80 at a
        // full stick), already there.
        let stick = if lined_up { [0.5, 0.0] } else { [0.0, 0.0] };
        if lined_up {
            s.deck_adjust = DeckAdjust::target(&s.tuning.adjust, stick, s.mirrored());
        }
        let pad = Pad {
            flick: stick,
            ..Pad::default()
        };
        // The action graph's BoardAdjust runs while the C-stick is held.
        s.graph.board_adjust = lined_up;
        let mut ev = Vec::new();
        for _ in 0..30 {
            s.step(&p, &pad, &FlickOut::default(), TICK, &mut ev);
            if !s.is_airborne() {
                break;
            }
        }
        assert!(s.rolling(), "{ev:?}");
        let name = ev
            .iter()
            .find_map(|e| match e {
                Event::Landed { name, .. } => Some(name.clone()),
                _ => None,
            })
            .unwrap_or_default();
        (s, name)
    };
    let (crooked, _) = land(false);
    let off = crooked.last_landing.unwrap().crooked;
    assert!(off > 30.0 * PI / 180.0, "the body's way: {off} rad off");
    let (s, _) = land(true);
    let off = s.last_landing.unwrap().crooked;
    assert!(off < 3.0 * PI / 180.0, "lined up: {off} rad off");
    // The board rolls on along the travel, the body over it.
    let yaw = atan2f(s.fwd.x, s.fwd.z);
    assert!(yaw.abs() < 3.0 * PI / 180.0, "{yaw}");
    assert!(s.deck_adjust.is_none());
}

#[test]
fn a_board_held_across_the_travel_lands_sideways() {
    let p = park(false);
    let (straight, ev) = ollie(&p, [0.0, 0.0]);
    assert!(
        straight.last_landing.unwrap().crooked < 0.1,
        "{:?} {ev:?}",
        straight.last_landing
    );
    let (s, ev) = ollie(&p, [1.0, 0.0]);
    // Skate 3's 80 degrees: a bail, or at best a sketchy landing far off the travel.
    let off = s.last_landing.map_or(0.0, |l| l.crooked);
    assert!(
        ev.contains(&Event::Bailed) || off > 60.0 * PI / 180.0,
        "{off} {ev:?}"
    );
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
    assert!(g.graph.is_some(), "state graphs on");
    g
}

/// With the player's imported data: an ollie at 6 m/s, then the C-stick held one way in the air. Skate
/// 3's motion graph goes into that board adjust (`OnBoard/InAir/BoardAdjusts/<way>`), the action graph
/// publishes the stick to it, its clips swing (or tip) the posed deck, and the physics' deck follows.
#[test]
fn imported_the_c_stick_in_the_air_plays_skate_3s_board_adjust() {
    let Some(dir) = data_dir() else {
        eprintln!("board adjust: no imported data (FG_DATA): skipped");
        return;
    };
    let c = game_core::input::C_RANGE;
    // A regular rider is the clips mirrored (the posed board turned end for end, SK8's
    // `board_flipped`): the stick plays the other side's adjust (BoardAdjust negates its angle), which
    // swings the deck the same way round.
    for (x, y, way, stance) in [
        (1.0f32, 0.0f32, "Right", Stance::Goofy),
        (-1.0, 0.0, "Left", Stance::Goofy),
        (0.0, 1.0, "Up", Stance::Goofy),
        (0.0, -1.0, "Down", Stance::Goofy),
        (1.0, 0.0, "Left", Stance::Regular),
        (-1.0, 0.0, "Right", Stance::Regular),
        (0.0, 1.0, "Up", Stance::Regular),
    ] {
        let mut g = imported(&dir);
        g.skater.stance = stance;
        g.place_rider(v3(-16.0, 0.0, -17.0), 0.0, 6.0);
        for _ in 0..90 {
            g.update(
                &PadRaw {
                    flags: flag::CONNECTED,
                    ..Default::default()
                },
                TICK,
            );
        }
        let mut entered = false;
        let mut adjusting = false;
        let mut most = DeckAdjust::NONE;
        let mut posed = DeckAdjust::NONE;
        for p in 0..80 {
            // Wound up and popped (the C-stick back, then forward), then let go, then held one way.
            let (cx, cy) = if p < 40 {
                (0.0, -c)
            } else if p < 45 {
                (0.0, c)
            } else if p < 52 {
                (0.0, 0.0)
            } else {
                (x * c, y * c)
            };
            g.update(
                &PadRaw {
                    flags: flag::CONNECTED,
                    cstick_x: cx as i8,
                    cstick_y: cy as i8,
                    ..Default::default()
                },
                TICK,
            );
            if !g.skater.is_airborne() {
                continue;
            }
            let state = g.graph.as_ref().unwrap().runner.motion_state();
            entered |= state.starts_with(&format!("OnBoard/InAir/BoardAdjusts/{way}"));
            adjusting |= g.skater.graph.board_adjust;
            let a = g.skater.deck_adjust;
            if a.yaw.abs() + a.pitch.abs() > most.yaw.abs() + most.pitch.abs() {
                most = a;
            }
            if let Some((_, nose)) = g.board_axes_in_character() {
                let nose = if g.skater.mirrored() { -nose } else { nose };
                posed = DeckAdjust::from_pose(nose);
            }
        }
        assert!(entered, "{way}: never in BoardAdjusts/{way}");
        assert!(
            adjusting,
            "{way}: the action graph never published the stick"
        );
        let deg = 180.0 / PI;
        let (yaw, pitch) = (most.yaw * deg, most.pitch * deg);
        let (posed_yaw, posed_pitch) = (posed.yaw * deg, posed.pitch * deg);
        // Skate 3's clips: about 80 degrees right, 88 left, 20 nose down, 24 nose up (goofy).
        // The C-stick right swings the nose to the board's -x (up x nose) in either stance.
        match way {
            _ if y != 0.0 => {}
            _ if x > 0.0 => assert!(yaw < -60.0 && posed_yaw < -60.0, "{way}: {yaw} {posed_yaw}"),
            _ => assert!(yaw > 60.0 && posed_yaw > 60.0, "{way}: {yaw} {posed_yaw}"),
        }
        match way {
            "Up" => assert!(
                pitch < -12.0 && posed_pitch < -12.0,
                "{way}: {pitch} {posed_pitch}"
            ),
            "Down" => assert!(
                pitch > 12.0 && posed_pitch > 12.0,
                "{way}: {pitch} {posed_pitch}"
            ),
            _ => {}
        }
    }
}

/// An adjusted landing keeps its adjusted heading through the hand-back to the physical board: Skate
/// 3's physical board flies hooked to the posed (adjusted) deck, so the board the ground state adopts
/// on touching down is the adjusted one (it flew square to the body, and the tick after the landing
/// the rider snapped back to it, undoing the adjust).
#[test]
fn an_adjusted_landing_keeps_its_heading_after_touching_down() {
    let p = park(false);
    for x in [0.5, -0.5] {
        let mut s = Skater::spawn(&p);
        s.since_spawn = 10.0;
        s.speed = 5.0;
        let mut ev = Vec::new();
        // Rolling first: the rigid board is under the rider.
        for _ in 0..10 {
            s.step(&p, &Pad::default(), &FlickOut::default(), TICK, &mut ev);
        }
        // The state graph pops an ollie (its takeoff clip's jump).
        s.graph.pop = Some((Flick::OLLIE, 1.0));
        s.step(&p, &Pad::default(), &FlickOut::default(), TICK, &mut ev);
        s.graph.pop = None;
        assert!(s.is_airborne(), "popped");
        let pad = Pad {
            flick: [x, 0.0],
            ..Pad::default()
        };
        s.graph.board_adjust = true;
        for _ in 0..90 {
            s.step(&p, &pad, &FlickOut::default(), TICK, &mut ev);
            if !s.is_airborne() {
                break;
            }
        }
        assert!(!s.is_airborne(), "landed: {ev:?}");
        let landed = atan2f(s.fwd.x, s.fwd.z);
        assert!(
            landed.abs() > 20.0 * PI / 180.0,
            "stick {x}: adjusted {landed}"
        );
        // The wheels' grip straightens it a little a tick; no snap back to the body's heading.
        let mut last = landed;
        for i in 0..10 {
            s.step(&p, &pad, &FlickOut::default(), TICK, &mut ev);
            let yaw = atan2f(s.fwd.x, s.fwd.z);
            assert!(
                (yaw - last).abs() < 3.0 * PI / 180.0,
                "stick {x}, tick {i}: {last} -> {yaw}"
            );
            last = yaw;
        }
        assert!(last.abs() > 20.0 * PI / 180.0, "stick {x}: {last}");
    }
}
