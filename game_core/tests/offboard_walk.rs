//! Walking off the board over stairs and obstacles (Skate 3's biped, `skater::biped`), on the built-in
//! park with stairs and walls added to it:
//!
//! - Stairs: the ground ahead with steps in it is Skate 3's stairs kinds (`GroundSlopeType`'s
//!   `StairUp/Dn` `Shallow/Steep`), a slope without them its ramp kinds; with the player's imported data
//!   the motion graph plays its own stairs walk and run cycles (`Walk_StairUpShallow` ...).
//! - Obstacles (SK8 movement_intent.rs): walking straight into a wall turns the rider to face it and
//!   stops him there; walking into it at an angle turns him along it and he walks on.

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::{Vec3, v3};
use game_core::park::Shape;
use game_core::skater::{Event, Mode};

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
    run(&mut g, 1, [0, 0], btn::Y);
    for _ in 0..120 {
        if g.skater.mode == Mode::Walk {
            break;
        }
        run(&mut g, 1, [0, 0], 0);
    }
    assert_eq!(g.skater.mode, Mode::Walk, "Y gets off the board standing");
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
