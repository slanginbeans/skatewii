use super::*;
use crate::park::Surface;
use crate::physics::board::{Board, BoardInput, DECK_HEIGHT};

/// Flat ground at y = 0, with an optional wall (the plane x = `wall`, facing -x) as triangles.
struct Yard {
    wall: Option<f32>,
}

impl Terrain for Yard {
    fn ground(&self, _x: f32, _z: f32, _max: f32) -> Surface {
        Surface {
            height: 0.0,
            normal: Vec3::UP,
        }
    }
    fn spawn(&self) -> (Vec3, f32) {
        (Vec3::ZERO, 0.0)
    }
    fn triangles_near(&self, c: Vec3, r: f32, out: &mut Vec<([Vec3; 3], Vec3)>) -> bool {
        let (x0, x1, z0, z1) = (c.x - r - 1.0, c.x + r + 1.0, c.z - r - 1.0, c.z + r + 1.0);
        out.push((
            [v3(x0, 0.0, z0), v3(x0, 0.0, z1), v3(x1, 0.0, z0)],
            Vec3::UP,
        ));
        out.push((
            [v3(x1, 0.0, z0), v3(x0, 0.0, z1), v3(x1, 0.0, z1)],
            Vec3::UP,
        ));
        if let Some(x) = self.wall {
            let n = v3(-1.0, 0.0, 0.0);
            out.push(([v3(x, 0.0, z0), v3(x, 3.0, z0), v3(x, 0.0, z1)], n));
            out.push(([v3(x, 0.0, z1), v3(x, 3.0, z0), v3(x, 3.0, z1)], n));
        }
        true
    }
}

fn close(a: f32, b: f32, tol: f32, what: &str) {
    assert!(
        (a - b).abs() <= tol * (1.0 + b.abs()),
        "{what}: {a} vs SK8 {b}"
    );
}

fn same_rotation(a: Quat, b: Quat, what: &str) {
    let (ma, mb) = (a.to_mat3(), b.to_mat3());
    for k in 0..3 {
        let d = (ma.cols[k] - mb.cols[k]).length();
        assert!(
            d < 2e-4,
            "{what} axis {k}: {:?} vs SK8 {:?}",
            ma.cols[k],
            mb.cols[k]
        );
    }
}

#[test]
fn the_joints_are_built_as_sk8_builds_them() {
    let built = build_joints(
        &INITIAL_BONES,
        &PARENTS,
        &PHYS_BONES,
        &JOINT_SETTINGS,
        &GlobalJointSettings::STOCK,
    );
    for (b, s) in built.iter().zip(&STOCK_JOINTS) {
        let w = alloc::format!("joint {} -> {}", s.child, s.parent);
        assert_eq!((b.parent, b.child), (s.parent, s.child), "{w}");
        let (p, q) = (&b.params, &s.params);
        assert_eq!(
            (p.swing_mode, p.twist_mode),
            (q.swing_mode, q.twist_mode),
            "{w}"
        );
        for k in 0..3 {
            close(
                p.velocity_allowance.get(k),
                q.velocity_allowance.get(k),
                1e-6,
                &w,
            );
        }
        close(p.twist_velocity, q.twist_velocity, 1e-6, &w);
        close(p.swing_velocity, q.swing_velocity, 1e-6, &w);
        close(p.swing_threshold, q.swing_threshold, 1e-5, &w);
        close(p.twist_threshold, q.twist_threshold, 1e-5, &w);
        let (f, g) = (&b.frames, &s.frames);
        same_rotation(f.orientation_a, g.orientation_a, &w);
        same_rotation(f.orientation_b, g.orientation_b, &w);
        same_rotation(f.linear_orientation_b, g.linear_orientation_b, &w);
        assert!((f.anchor_a - g.anchor_a).length() < 2e-5, "{w} anchor a");
        assert!((f.anchor_b - g.anchor_b).length() < 2e-5, "{w} anchor b");
    }
}

/// The rig's initial pose standing at `at` (its origin on the ground).
fn standing(at: Vec3) -> [Xform; ANIMATED_PARTS] {
    pose_from_bones(&INITIAL_BONES, &Xform::new(Mat3::IDENTITY, at), false)
}

/// The board parked well away from the rider.
fn parked() -> Board {
    Board::new(v3(4.0, DECK_HEIGHT, 0.0), &Mat3::IDENTITY)
}

fn input(pose: [Xform; ANIMATED_PARTS], airborne: bool) -> RiderInput {
    RiderInput {
        pose,
        airborne,
        ground: (Vec3::ZERO, Vec3::UP),
        board_contacts: false,
        clock: None,
    }
}

#[test]
fn the_merged_and_solved_bodies_cover_the_skeleton() {
    assert_eq!(SOLVED, 18);
    assert!(SOLVED_PARTS.iter().all(|&p| host(p) == p && p > 0));
    for (child, into) in MERGED {
        assert!(!SOLVED_PARTS.contains(&child) && SOLVED_PARTS.contains(&into));
    }
    // The whole mass stays on the solved bodies.
    let stock = stock_parts();
    let total: f32 = (1..ANIMATED_PARTS)
        .map(|p| 1.0 / stock[p].animated.inverse_mass)
        .sum();
    let b = RiderBody::new(&standing(Vec3::ZERO));
    let solved: f32 = SOLVED_PARTS
        .iter()
        .map(|&p| 1.0 / b.parts[p].mass.inverse_mass)
        .sum();
    assert!((total - solved).abs() < 1e-3 * total, "{total} vs {solved}");
    // Each part's solved slot is its host's place among the solved bodies.
    for p in 1..ANIMATED_PARTS {
        assert_eq!(SOLVED_PARTS[SOLVED_SLOT[p]], host(p), "part {p}");
    }
}

#[test]
fn the_drives_hold_the_body_to_the_animated_pose() {
    let yard = Yard { wall: None };
    let mut board = parked();
    let mut pose = standing(Vec3::ZERO);
    let mut body = RiderBody::new(&pose);
    let idle = BoardInput {
        wheel_friction: [pb::WHEEL_FRICTION[0], pb::WHEEL_FRICTION[1]],
        ..BoardInput::default()
    };
    // Standing, then gliding along at 6 m/s: solved every tick (as in the air), the body stays on the
    // animated pose against gravity and its own motion.
    let mut worst: f32 = 0.0;
    for tick in 0..240 {
        if tick >= 120 {
            let step = v3(0.0, 0.0, 6.0 * STEP);
            pose = pose.map(|x| Xform::new(x.rot, x.pos + step));
        }
        board.step_with(&yard, &idle, Some((&mut body, &input(pose, true))));
        assert!(body.solved);
        if tick > 30 {
            worst = worst.max(body.feedback.max_error);
            for p in 1..ANIMATED_PARTS {
                let e = body.feedback.errors[p].length();
                assert!(
                    e < 0.06,
                    "tick {tick}: {} off by {e} m",
                    super::super::skeleton::BONE_NAMES[p]
                );
            }
        }
    }
    assert!(worst < 0.03, "largest hips/legs/head error {worst} m");
    // Upright: the head over the hips as animated.
    let now = body.pose();
    let d = (now[1].pos - now[23].pos) - (pose[1].pos - pose[23].pos);
    assert!(d.length() < 0.03, "head off by {d:?}");
}

#[test]
fn on_flat_ground_the_body_follows_the_pose_until_a_wall_is_near() {
    let mut board = parked();
    let pose = standing(Vec3::ZERO);
    let mut body = RiderBody::new(&pose);
    let idle = BoardInput::default();
    // Riding on open ground: nothing the animated pose can't stand in for, so no solve, and the body
    // is the pose.
    let open = Yard { wall: None };
    for _ in 0..5 {
        board.step_with(&open, &idle, Some((&mut body, &input(pose, false))));
        assert!(!body.solved);
    }
    assert!(body.feedback.max_error < 1e-5);
    assert!(!body.feedback.flags.nonboard);
    assert_eq!(body.contact_rows(), 0);
    // A wall at arm's length: an arm can touch it, so the skeleton is solved, and the contact is
    // reported.
    let walled = Yard { wall: Some(0.5) };
    board.step_with(&walled, &idle, Some((&mut body, &input(pose, false))));
    assert!(body.solved);
    assert!(body.feedback.flags.any && body.feedback.flags.nonboard);
    // The board's contacts and the arm's against the wall, in one solve.
    assert!(body.contact_rows() > 0);
    assert!(body.feedback.drive_weight < 1.0);
}

#[test]
fn a_mirrored_rider_takes_its_partners_bones_reflected() {
    // The rig's initial pose is symmetric: mirrored, every body lands on its own initial frame.
    let plain = pose_from_bones(&INITIAL_BONES, &Xform::IDENTITY, false);
    let mirrored = pose_from_bones(&INITIAL_BONES, &Xform::IDENTITY, true);
    for p in 1..ANIMATED_PARTS {
        assert!((plain[p].pos - mirrored[p].pos).length() < 0.02, "part {p}");
        for k in 0..3 {
            let d = (plain[p].rot.cols[k] - mirrored[p].rot.cols[k]).length();
            assert!(d < 0.1, "part {p} axis {k}: {d}");
        }
    }
}

/// The skeleton's rows fit the cost budget: 18 bodies, 17 joints, 35 drives with the hips' target.
#[test]
fn the_solved_skeleton_has_the_reduced_rows() {
    let joints = STOCK_JOINTS
        .iter()
        .filter(|j| host(j.child) != host(j.parent))
        .count();
    assert_eq!(joints, 17);
    let drives = 1 + SOLVED_PARTS
        .iter()
        .filter(|&&p| p != 23)
        .map(|&p| {
            [PARENTS[p].unwrap(), 23]
                .iter()
                .filter(|&&q| host(q) != p)
                .count()
        })
        .sum::<usize>();
    assert_eq!(drives, 34);
}

/// Host cost of a board step with the skeleton solved (run with `--release --ignored --nocapture`):
/// the budget is 150 us on the host (about 1.5 ms on the Wii).
#[test]
#[ignore]
fn rider_body_step_cost() {
    let yard = Yard { wall: None };
    let mut board = Board::new(v3(0.0, DECK_HEIGHT, 0.0), &Mat3::IDENTITY);
    let pose = standing(v3(0.0, 0.0, -2.0));
    let mut body = RiderBody::new(&pose);
    let idle = BoardInput::default();
    let solved = input(pose, true);
    for _ in 0..60 {
        board.step_with(&yard, &idle, Some((&mut body, &solved)));
    }
    let n = 2000;
    let t = std::time::Instant::now();
    for _ in 0..n {
        board.step_with(&yard, &idle, Some((&mut body, &solved)));
    }
    let with = t.elapsed().as_micros() as f32 / n as f32;
    let t = std::time::Instant::now();
    for _ in 0..n {
        board.step(&yard, &idle);
    }
    let alone = t.elapsed().as_micros() as f32 / n as f32;
    let following = input(pose, false);
    let t = std::time::Instant::now();
    for _ in 0..n {
        board.step_with(&yard, &idle, Some((&mut body, &following)));
    }
    let kinematic = t.elapsed().as_micros() as f32 / n as f32;
    std::eprintln!(
        "board alone {alone} us, with the skeleton solved {with} us (+{} us), following {kinematic} us",
        with - alone
    );
    assert!(with - alone < 150.0, "skeleton solve {} us", with - alone);
}
