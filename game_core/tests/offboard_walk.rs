//! Walking off the board over stairs and obstacles (Skate 3's biped, `skater::biped`), on the built-in
//! park with stairs and walls added to it:
//!
//! - Stairs: the ground ahead with steps in it is Skate 3's stairs kinds (`GroundSlopeType`'s
//!   `StairUp/Dn` `Shallow/Steep`), a slope without them its ramp kinds; with the player's imported data
//!   the motion graph plays its own stairs walk and run cycles (`Walk_StairUpShallow` ...).
//! - Obstacles (SK8 movement_intent.rs): walking straight into a wall turns the rider to face it and
//!   stops him there; walking into it at an angle turns him along it and he walks on.
//! - Foot IK (SK8 offboard/feet.rs, the skeleton IK's leg solve), with the player's imported rider:
//!   standing with a foot over a step, that foot is lifted onto it (the knee bent, the leg's length
//!   kept); walking, the foot holding the ground stays put in the world.
//! - Climbing (SK8's own extension, physics/climbing/): running and jumping (B) at a 2 m wall the rider
//!   catches its edge and hangs; B again pulls him up onto the top, where he walks on with the board
//!   still in hand. A wall too high is not caught; the stock graphs wait while he holds the edge.

mod common;

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::{Vec3, v3};
use game_core::park::Shape;
use game_core::skater::Event;

const TICK: f32 = 1.0 / 60.0;
/// A clear lane across the built-in park (nothing in the way along +z for 38 m).
const LANE: Vec3 = Vec3 {
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

/// Off the board at `pos` facing `heading` (+z at 0), standing, the camera settled behind.
fn on_foot(mut g: Game, pos: Vec3, heading: f32) -> Game {
    g.place_rider(pos, heading, 0.0);
    run(&mut g, 30, [0, 0], 0);
    if g.graph.is_some() {
        run(&mut g, 1, [0, 0], btn::Y);
        for _ in 0..120 {
            if g.skater.walking() {
                break;
            }
            run(&mut g, 1, [0, 0], 0);
        }
    } else {
        common::get_off(&mut g);
    }
    assert!(g.skater.walking(), "Y gets off the board standing");
    run(&mut g, 60, [0, 0], 0);
    g.skater.pos = pos;
    g.skater.biped.heading = heading;
    run(&mut g, 30, [0, 0], 0);
    g
}

/// Stairs across the lane from `z0` along +z: `count` steps up of `rise` over `tread`, a 3 m landing,
/// and as many steps back down. Returns where the way down ends.
fn add_stairs(g: &mut Game, z0: f32, rise: f32, tread: f32, count: usize) -> f32 {
    let end = z0 + 2.0 * tread * count as f32 + 3.0;
    for k in 0..count {
        g.park.shapes.push(Shape::Block {
            min: v3(LANE.x - 4.0, 0.0, z0 + tread * k as f32),
            max: v3(LANE.x + 4.0, rise * (k + 1) as f32, end - tread * k as f32),
            rgba: 0x808080ff,
        });
    }
    end
}

#[test]
fn stairs_up_and_down_are_skate_3s_stairs_kinds() {
    let mut g = Game::new();
    // Shallow stairs (0.15 m over 0.3 m: 27 degrees) 3 m ahead, up and over.
    let end = add_stairs(&mut g, LANE.z + 3.0, 0.15, 0.3, 8);
    let mut g = on_foot(g, LANE, 0.0);
    let mut seen = [0; 9];
    let mut top = false;
    for _ in 0..400 {
        run(&mut g, 1, [0, 60], 0);
        seen[g.skater.biped.slope_kind as usize] += 1;
        top |= g.skater.pos.y > 1.1 && g.skater.biped.air.is_none();
        if g.skater.pos.z > end + 1.0 {
            break;
        }
    }
    assert!(top, "walked up onto the landing");
    assert!(
        g.skater.pos.z > end,
        "and down the far side: {:?}",
        g.skater.pos
    );
    assert!(seen[1] > 20, "stairs up, shallow: {seen:?}");
    assert!(seen[3] > 20, "stairs down, shallow: {seen:?}");
    assert!(
        seen[5..].iter().all(|&n| n == 0),
        "never a ramp kind: {seen:?}"
    );
    assert_eq!(g.skater.biped.slope_kind, 0, "flat at the bottom");
}

#[test]
fn steep_stairs_are_the_steep_kinds() {
    let mut g = Game::new();
    // 0.2 m over 0.25 m: 39 degrees, past Skate 3's 32.
    let end = add_stairs(&mut g, LANE.z + 3.0, 0.2, 0.25, 6);
    let mut g = on_foot(g, LANE, 0.0);
    let mut seen = [0; 9];
    for _ in 0..400 {
        run(&mut g, 1, [0, 60], 0);
        seen[g.skater.biped.slope_kind as usize] += 1;
        if g.skater.pos.z > end + 1.0 {
            break;
        }
    }
    assert!(seen[2] > 10, "stairs up, steep: {seen:?}");
    assert!(seen[4] > 10, "stairs down, steep: {seen:?}");
}

#[test]
fn a_slope_without_steps_is_a_ramp_kind() {
    let mut g = Game::new();
    g.park.shapes.push(Shape::Ramp {
        min: v3(LANE.x - 1.5, 0.0, LANE.z + 3.0),
        max: v3(LANE.x + 1.5, 1.2, LANE.z + 5.4),
        axis: game_core::park::Axis::Z,
        h0: 0.0,
        h1: 1.2,
        rgba: 0x808080ff,
    });
    g.park.shapes.push(Shape::Block {
        min: v3(LANE.x - 1.5, 0.0, LANE.z + 5.4),
        max: v3(LANE.x + 1.5, 1.2, LANE.z + 8.4),
        rgba: 0x808080ff,
    });
    let mut g = on_foot(g, LANE, 0.0);
    let mut seen = [0; 9];
    let mut top = 0.0f32;
    for _ in 0..240 {
        run(&mut g, 1, [0, 60], 0);
        seen[g.skater.biped.slope_kind as usize] += 1;
        top = top.max(g.skater.pos.y);
    }
    assert!(top > 1.1, "up the ramp: {top}");
    assert!(seen[5] > 10, "a shallow ramp up (27 degrees): {seen:?}");
    assert!(seen[1..5].iter().all(|&n| n == 0), "no stairs: {seen:?}");
}

/// A wall 2 m high across the lane `ahead` of it.
fn add_wall(g: &mut Game, ahead: f32) -> f32 {
    let z = LANE.z + ahead;
    g.park.shapes.push(Shape::Block {
        min: v3(LANE.x - 12.0, 0.0, z),
        max: v3(LANE.x + 12.0, 2.0, z + 0.5),
        rgba: 0x808080ff,
    });
    z
}

#[test]
fn walking_straight_into_a_wall_faces_it_and_stops() {
    let mut g = Game::new();
    let wall = add_wall(&mut g, 4.0);
    // A little off straight (10 degrees right): still into the wall.
    let mut g = on_foot(g, LANE, -0.17);
    run(&mut g, 150, [0, 80], 0);
    let b = &g.skater.biped;
    assert!(b.obstacle.is_some() && b.obstacle_centred, "against it");
    assert!(b.speed < 0.05, "stopped: {}", b.speed);
    assert!(
        g.skater.pos.z < wall - game_core::skater::biped::BODY_RADIUS + 0.12,
        "the body short of the wall: {:?}",
        g.skater.pos
    );
    assert!(
        game_core::math::wrap_angle(b.heading).abs() < 0.08,
        "turned to face it square: {}",
        b.heading
    );
}

#[test]
fn walking_into_a_wall_at_an_angle_turns_along_it() {
    for along in [1.0f32, -1.0] {
        let mut g = Game::new();
        let wall = add_wall(&mut g, 4.0);
        let mut g = on_foot(g, LANE, along * std::f32::consts::FRAC_PI_4);
        run(&mut g, 60, [0, 0], 0);
        // Walking ahead 45 degrees into the wall (+x first, then -x), the camera behind.
        let x0 = g.skater.pos.x;
        run(&mut g, 90, [0, 100], 0);
        let b = &g.skater.biped;
        assert!(g.skater.pos.z < wall, "{:?}", g.skater.pos);
        assert!(b.obstacle.is_some(), "along the wall: {:?}", g.skater.pos);
        assert!(!b.obstacle_centred);
        // From 45 degrees round towards along it (the turn eases in as he nears the wall's way).
        let yaw = game_core::math::wrap_angle(b.heading) * along;
        assert!(
            yaw > 1.05 && yaw < std::f32::consts::FRAC_PI_2 + 0.1,
            "turned along the wall ({along}): {yaw}"
        );
        assert!(b.speed > 1.5, "walking on: {}", b.speed);
        assert!((g.skater.pos.x - x0) * along > 2.0, "{:?}", g.skater.pos);
    }
}

/// A block `height` high and 3 m deep across the lane from `ahead` of it.
fn add_block(g: &mut Game, ahead: f32, height: f32) -> f32 {
    let z = LANE.z + ahead;
    g.park.shapes.push(Shape::Block {
        min: v3(LANE.x - 6.0, 0.0, z),
        max: v3(LANE.x + 6.0, height, z + 3.0),
        rgba: 0x808080ff,
    });
    z
}

/// Runs at the block ahead, jumping (B) when the feet are `at` from its face; returns whether the
/// rider caught its edge.
fn run_and_jump(g: &mut Game, face: f32, at: f32) -> bool {
    for _ in 0..240 {
        run(g, 1, [0, 100], 0);
        if face - g.skater.pos.z < at {
            break;
        }
    }
    run(g, 1, [0, 100], btn::B);
    for _ in 0..60 {
        run(g, 1, [0, 100], 0);
        if g.skater.climbing() {
            return true;
        }
    }
    false
}

#[test]
fn jumping_at_a_ledge_catches_it_and_b_climbs_onto_it() {
    let mut g = Game::new();
    let face = add_block(&mut g, 6.0, 2.0);
    let mut g = on_foot(g, LANE, 0.0);
    let held = g.skater.possession.held();
    assert!(run_and_jump(&mut g, face, 1.0), "caught the edge");
    // Hanging: the body under the edge facing the wall, held there however long.
    run(&mut g, 90, [0, 0], 0);
    let c = g.skater.climb.attached.expect("holding");
    assert_eq!(c.phase, game_core::skater::climb::Phase::Hang);
    assert!(
        g.skater.pos.z < face && g.skater.pos.y < 1.0,
        "{:?}",
        g.skater.pos
    );
    // Facing into the wall (the built-in park's height field gives a wall the ray's own way back).
    let into = c.ledge.forward;
    let yaw = game_core::math::atan2f(into.x, into.z);
    assert!(game_core::math::wrap_angle(g.skater.heading - yaw).abs() < 1e-3);
    assert!(yaw.abs() < 0.2, "{yaw}");
    assert!(g.skater.walking());
    // B pulls him up onto the top.
    run(&mut g, 1, [0, 0], btn::B);
    assert_eq!(
        g.skater.climb.attached.unwrap().phase,
        game_core::skater::climb::Phase::Mantle
    );
    let mut over = None;
    for i in 0..180 {
        run(&mut g, 1, [0, 0], 0);
        if !g.skater.climbing() {
            over = Some(i);
            break;
        }
    }
    let over = over.expect("up and over");
    let t = game_core::skater::climb::MANTLE + game_core::skater::climb::SETTLE;
    assert!((over as f32 / 60.0 - t).abs() < 0.1, "{over} ticks");
    assert!(
        (g.skater.pos.y - 2.0).abs() < 0.03 && g.skater.pos.z > face + 0.4,
        "standing on the top: {:?}",
        g.skater.pos
    );
    assert_eq!(g.skater.possession.held(), held, "the board still in hand");
    // And walks on along the top.
    run(&mut g, 60, [0, 60], 0);
    assert!(g.skater.pos.z > face + 1.5 && (g.skater.pos.y - 2.0).abs() < 0.01);
    assert!(!g.skater.climbing() && g.skater.biped.air.is_none());
}

#[test]
fn a_ledge_too_high_is_not_caught() {
    let mut g = Game::new();
    let face = add_block(&mut g, 6.0, 3.2);
    let mut g = on_foot(g, LANE, 0.0);
    assert!(!run_and_jump(&mut g, face, 1.0), "nothing to reach");
    run(&mut g, 60, [0, 0], 0);
    assert!(g.skater.pos.y.abs() < 1e-3 && g.skater.biped.air.is_none());
}

#[test]
fn walking_up_to_a_ledge_without_jumping_never_catches_it() {
    let mut g = Game::new();
    let face = add_block(&mut g, 4.0, 1.8);
    let mut g = on_foot(g, LANE, 0.0);
    for _ in 0..120 {
        run(&mut g, 1, [0, 100], 0);
        assert!(!g.skater.climbing());
    }
    assert!(g.skater.pos.z < face, "{:?}", g.skater.pos);
    assert!(
        g.skater.climb.reach() > 0.3,
        "reaching for it: {}",
        g.skater.climb.reach()
    );
}

/// With the player's imported data: the stock graphs wait in the jump while the rider holds the edge,
/// and pick up walking on the top after.
#[test]
fn imported_climbing_holds_the_graphs_and_walks_on_after() {
    let Some(mut g) = imported() else {
        eprintln!("offboard_walk: no imported data with the off-board clips (FG_DATA): skipped");
        return;
    };
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    let face = add_block(&mut g, 6.0, 2.0);
    let mut g = on_foot(g, LANE, 0.0);
    assert!(
        run_and_jump(&mut g, face, 1.0),
        "caught the edge: {}",
        path(&g)
    );
    let held = path(&g);
    run(&mut g, 60, [0, 0], 0);
    assert_eq!(path(&g), held, "the graphs wait");
    run(&mut g, 1, [0, 0], btn::B);
    run(&mut g, 120, [0, 0], 0);
    assert!(!g.skater.climbing());
    run(&mut g, 60, [0, 60], 0);
    assert!(
        path(&g).starts_with("OffBoard/OBGround/"),
        "walking on the top: {}",
        path(&g)
    );
    assert!((g.skater.pos.y - 2.0).abs() < 0.01, "{:?}", g.skater.pos);
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

/// The built-in park with the player's imported rules, rider and motion (`None` without them, or with a
/// motion pack imported before the off-board clips were).
fn imported() -> Option<Game> {
    let dir = data_dir()?;
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .unwrap();
    g.load_skater(leak(dir.join("skater.fgpak"))).unwrap();
    g.load_motion(leak(dir.join("motion.fgpak"))).unwrap();
    let m = g.motion.as_ref()?;
    m.db.clip_index("NB_WALK_FWD_STAIRUP30_CYC")
        .and_then(|i| m.pose(i))?;
    Some(g)
}

/// With the player's imported data: walking and running up and down stairs plays the motion graph's
/// own stairs cycles (`OffBoard/OBGround/Locomotion/NotStand/NotThin/Walk_StairUpShallow` ...).
#[test]
fn imported_stairs_play_the_graphs_stairs_cycles() {
    let path = |g: &Game| g.graph.as_ref().unwrap().runner.motion_state();
    for (stick, up, down) in [
        ([0, 40], "/Walk_StairUpShallow/", "/Walk_StairDnShallow/"),
        ([0, 100], "/Run_StairUpShallow/", "/Run_StairDnShallow/"),
    ] {
        let Some(mut g) = imported() else {
            eprintln!(
                "offboard_walk: no imported data with the off-board clips (FG_DATA): skipped"
            );
            return;
        };
        let end = add_stairs(&mut g, LANE.z + 3.0, 0.15, 0.3, 12);
        let mut g = on_foot(g, LANE, 0.0);
        let (mut went_up, mut went_down) = (false, false);
        for _ in 0..600 {
            run(&mut g, 1, stick, 0);
            went_up |= path(&g).contains(up);
            went_down |= path(&g).contains(down);
            assert!(
                !g.events.contains(&Event::SteppedOn),
                "still on foot: {}",
                path(&g)
            );
            if g.skater.pos.z > end + 1.0 {
                break;
            }
        }
        assert!(went_up, "{up}: {}", path(&g));
        assert!(went_down, "{down}: {}", path(&g));
    }
}

/// A toe's drawn place in the world (`LEFTTOEBASE` / `RIGHTTOEBASE`) and a bone's in character space.
fn toe_world(g: &Game, foot: usize) -> Vec3 {
    let r = g.rider().unwrap();
    let b = r
        .character
        .bone(["LEFTTOEBASE", "RIGHTTOEBASE"][foot])
        .unwrap();
    let t = g.rider_bones().unwrap()[b][3];
    g.imported_xform().point(v3(t[0], t[1], t[2]))
}

fn bone_at(g: &Game, name: &str) -> Vec3 {
    let r = g.rider().unwrap();
    let t = g.rider_bones().unwrap()[r.character.bone(name).unwrap()][3];
    v3(t[0], t[1], t[2])
}

/// With the player's imported data: standing with one foot over a 0.12 m step, the foot placement lifts
/// that foot onto it (the clip has both on the flat), the leg bent to reach it and its length kept, the
/// other foot left on the ground.
#[test]
fn imported_a_foot_over_a_step_stands_on_it() {
    let Some(g) = imported() else {
        eprintln!("offboard_walk: no imported data with the off-board clips (FG_DATA): skipped");
        return;
    };
    let mut g = on_foot(g, LANE, 0.0);
    run(&mut g, 60, [0, 0], 0);
    // Where the feet stand on the flat, and the legs' length.
    let flat = [toe_world(&g, 0), toe_world(&g, 1)];
    let thigh = (bone_at(&g, "LEFTLEG") - bone_at(&g, "LEFTUPLEG")).length();
    let shin = (bone_at(&g, "LEFTFOOT") - bone_at(&g, "LEFTLEG")).length();
    let (high, low) = if flat[0].x > flat[1].x {
        (0, 1)
    } else {
        (1, 0)
    };
    // A step under the foot further towards +x only (its edge halfway between the feet).
    let edge = (flat[0].x + flat[1].x) * 0.5;
    g.park.shapes.push(Shape::Block {
        min: v3(edge, 0.0, LANE.z - 1.0),
        max: v3(edge + 1.0, 0.12, LANE.z + 1.0),
        rgba: 0x808080ff,
    });
    run(&mut g, 60, [0, 0], 0);
    assert!(g.skater.pos.y.abs() < 1e-4, "still standing on the flat");
    let now = [toe_world(&g, 0), toe_world(&g, 1)];
    let lift = now[high].y - flat[high].y;
    assert!(
        (lift - 0.12).abs() < 0.03,
        "the foot over the step stands on it: {lift} ({:?} -> {:?})",
        flat[high],
        now[high]
    );
    assert!(
        (now[low].y - flat[low].y).abs() < 0.01,
        "the other stays on the ground: {:?} -> {:?}",
        flat[low],
        now[low]
    );
    let thigh2 = (bone_at(&g, "LEFTLEG") - bone_at(&g, "LEFTUPLEG")).length();
    let shin2 = (bone_at(&g, "LEFTFOOT") - bone_at(&g, "LEFTLEG")).length();
    assert!(
        (thigh2 - thigh).abs() < 1e-3 && (shin2 - shin).abs() < 1e-3,
        "the leg keeps its length"
    );
}

/// With the player's imported data: walking up stairs, the foot holding the ground stays where it was
/// in the world (following the clip by at most 0.08 m a tick).
#[test]
fn imported_the_support_foot_stays_put_walking_up_stairs() {
    let Some(mut g) = imported() else {
        eprintln!("offboard_walk: no imported data with the off-board clips (FG_DATA): skipped");
        return;
    };
    add_stairs(&mut g, LANE.z + 2.0, 0.15, 0.3, 10);
    let mut g = on_foot(g, LANE, 0.0);
    let (mut held, mut moved) = (0, 0.0f32);
    let mut last: Option<(u8, Vec3)> = None;
    for _ in 0..180 {
        run(&mut g, 1, [0, 40], 0);
        let support = g.rider().unwrap().foot_support();
        if support < 2 {
            let toe = toe_world(&g, support as usize);
            if let Some((s, p)) = last
                && s == support
            {
                held += 1;
                moved = moved.max(v3(toe.x - p.x, 0.0, toe.z - p.z).length());
            }
            last = Some((support, toe));
        } else {
            last = None;
        }
    }
    assert!(g.skater.pos.y > 0.5, "up the stairs: {:?}", g.skater.pos);
    assert!(held > 30, "a foot holds the ground for {held} ticks");
    assert!(moved <= 0.081, "and stays put: {moved}");
}
