//! The shared iteration (SK8 solver.rs, solver/packed.rs; TU3 0x82AE27D0): every iteration is a pass
//! over the contacts, then the joints, then the drives, each row seeing the corrections of the rows
//! before it. Families exchange corrections on every pass; solving one to completion first changes the
//! result.

use super::{ContactRow, DriveRow, JointRow, Reaction};

/// Two distinct reactions (or a scratch one standing in for the immovable world).
fn pair<'a>(
    reactions: &'a mut [Reaction],
    a: Option<usize>,
    b: Option<usize>,
    world: &'a mut [Reaction; 2],
) -> (&'a mut Reaction, &'a mut Reaction) {
    match (a, b) {
        (Some(a), Some(b)) => {
            assert_ne!(a, b);
            if a < b {
                let (l, r) = reactions.split_at_mut(b);
                (&mut l[a], &mut r[0])
            } else {
                let (l, r) = reactions.split_at_mut(a);
                (&mut r[0], &mut l[b])
            }
        }
        (Some(a), None) => {
            world[1] = Reaction::default();
            (&mut reactions[a], &mut world[1])
        }
        (None, Some(b)) => {
            world[0] = Reaction::default();
            (&mut world[0], &mut reactions[b])
        }
        (None, None) => {
            let [w0, w1] = world;
            (w0, w1)
        }
    }
}

/// Runs `iterations` passes. `reactions` has one entry per body, cleared by the caller.
pub fn solve(
    contacts: &mut [ContactRow],
    joints: &mut [JointRow],
    drives: &mut [DriveRow],
    reactions: &mut [Reaction],
    iterations: u32,
) {
    let mut world = [Reaction::default(); 2];
    for _ in 0..iterations {
        for c in contacts.iter_mut() {
            let (ra, rb) = pair(reactions, c.a, c.b, &mut world);
            c.solve(ra, rb);
        }
        for j in joints.iter_mut() {
            let (ra, rb) = pair(reactions, Some(j.lanes.a), Some(j.lanes.b), &mut world);
            j.solve(ra, rb);
        }
        for d in drives.iter_mut() {
            let (ra, rb) = pair(reactions, Some(d.lanes.a), Some(d.lanes.b), &mut world);
            d.solve(ra, rb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::body::Mass;
    use super::super::*;
    use crate::math::{Vec3, v3};
    use alloc::vec;
    use alloc::vec::Vec;

    const G: Vec3 = v3(0.0, -9.8, 0.0);

    /// A ground contact under a sphere of radius `r` on the plane y = 0.
    fn ground(b: &Body, r: f32, friction: f32) -> ContactInput {
        ContactInput {
            point_a: b.position - v3(0.0, r, 0.0),
            point_b: v3(b.position.x, 0.0, b.position.z),
            normal: Vec3::UP,
            restitution: 0.0,
            static_friction: friction,
            dynamic_friction: friction,
        }
    }

    fn step(bodies: &mut [Body], contacts: &[ContactInput], iterations: u32) -> Vec<ContactRow> {
        let mut rows: Vec<ContactRow> = contacts
            .iter()
            .map(|c| ContactRow::build(bodies, Some(0), None, c))
            .collect();
        let mut reactions = vec![Reaction::default(); bodies.len()];
        solve(&mut rows, &mut [], &mut [], &mut reactions, iterations);
        for (b, r) in bodies.iter_mut().zip(&reactions) {
            b.integrate(r, G);
        }
        rows
    }

    #[test]
    fn a_sphere_rests_on_the_ground_carrying_its_weight() {
        let r = 0.031;
        let mut b = [Body::new(
            Mass::sphere(0.415, r, f32::MAX),
            v3(0.0, r + 0.01, 0.0),
            Quat::IDENTITY,
            G,
        )];
        let mut load = 0.0;
        for _ in 0..120 {
            let touching = b[0].position.y - r < 0.02;
            let c = if touching {
                vec![ground(&b[0], r, 0.8)]
            } else {
                vec![]
            };
            let rows = step(&mut b, &c, 50);
            load = rows.first().map_or(0.0, |r| r.normal_force());
        }
        assert!((b[0].position.y - r).abs() < 1e-3, "{:?}", b[0].position);
        assert!(b[0].velocity.length() < 1e-3);
        assert!((load - 0.415 * 9.8).abs() < 0.05, "{load}");
    }

    #[test]
    fn friction_rolls_a_sliding_sphere_and_holds_it_within_its_limit() {
        let r = 0.031;
        let mut b = [Body::new(
            Mass::sphere(0.415, r, f32::MAX),
            v3(0.0, r, 0.0),
            Quat::IDENTITY,
            G,
        )];
        b[0].velocity = v3(2.0, 0.0, 0.0);
        let c = ground(&b[0], r, 0.7);
        let rows = step(&mut b, &[c], 50);
        // The tick's friction is at most mu x the load, against the sliding.
        let f = rows[0].friction_force();
        assert!(
            f.x < 0.0 && f.x.abs() <= 0.7 * rows[0].normal_force() + 1e-3,
            "{f:?}"
        );
        for _ in 0..60 {
            let c = ground(&b[0], r, 0.7);
            step(&mut b, &[c], 50);
        }
        // Rolling without slipping: the contact point is at rest.
        let contact = b[0].point_velocity(v3(0.0, -r, 0.0));
        assert!(contact.length() < 0.02, "{contact:?} {:?}", b[0].velocity);
        // A solid sphere keeps 5/7 of its sliding speed once it rolls.
        assert!(
            (b[0].velocity.x - 2.0 * 5.0 / 7.0).abs() < 0.05,
            "{:?}",
            b[0].velocity
        );
    }

    fn pendulum_joint() -> (JointParams, JointFrames) {
        (
            JointParams {
                position_allowance: Vec3::ZERO,
                velocity_allowance: Vec3::ZERO,
                twist_velocity: 0.0,
                swing_velocity: 0.0,
                swing_threshold: 1.0,
                twist_threshold: 1.0,
                swing_mode: 0,
                twist_mode: 0,
            },
            JointFrames {
                orientation_a: Quat::IDENTITY,
                anchor_a: v3(0.0, 0.5, 0.0),
                orientation_b: Quat::IDENTITY,
                anchor_b: Vec3::ZERO,
                linear_orientation_b: Quat::IDENTITY,
            },
        )
    }

    #[test]
    fn a_locked_joint_holds_a_body_to_a_fixed_one() {
        let (p, f) = pendulum_joint();
        let mut fixed = Body::new(
            Mass::sphere(1.0, 0.1, f32::MAX),
            v3(0.0, 2.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
        );
        fixed.active = false;
        let hung = Body::new(
            Mass::sphere(1.0, 0.1, f32::MAX),
            v3(0.0, 1.5, 0.0),
            Quat::IDENTITY,
            G,
        );
        let mut bodies = [hung, fixed];
        bodies[0].velocity = v3(1.0, 0.0, 0.0);
        bodies[0].spin = v3(0.0, 0.0, 2.0);
        for _ in 0..60 {
            let mut joints = [JointRow::build(&bodies, 0, 1, &p, &f)];
            let mut reactions = vec![Reaction::default(); 2];
            solve(&mut [], &mut joints, &mut [], &mut reactions, 50);
            for (b, r) in bodies.iter_mut().zip(&reactions) {
                b.integrate(r, G);
            }
        }
        // Held at the anchor, unturned and at rest: welded.
        assert!(
            (bodies[0].position - v3(0.0, 1.5, 0.0)).length() < 2e-3,
            "{:?}",
            bodies[0].position
        );
        assert!(bodies[0].velocity.length() < 0.02);
        assert!(
            bodies[0].orientation.w > 0.9999,
            "{:?}",
            bodies[0].orientation
        );
    }

    #[test]
    fn a_free_spin_lane_lets_a_wheel_spin() {
        // Skate 3's wheel joint: twist and one swing locked, the spin about B's second axis free.
        let (mut p, f) = pendulum_joint();
        p.swing_mode = 3;
        p.swing_velocity = 1.0e6;
        let mut fixed = Body::new(
            Mass::sphere(1.0, 0.1, f32::MAX),
            v3(0.0, 2.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
        );
        fixed.active = false;
        let mut bodies = [
            Body::new(
                Mass::sphere(0.4, 0.03, f32::MAX),
                v3(0.0, 1.5, 0.0),
                Quat::IDENTITY,
                Vec3::ZERO,
            ),
            fixed,
        ];
        bodies[0].spin = v3(3.0, 5.0, 4.0);
        for _ in 0..10 {
            let mut joints = [JointRow::build(&bodies, 0, 1, &p, &f)];
            let mut reactions = vec![Reaction::default(); 2];
            solve(&mut [], &mut joints, &mut [], &mut reactions, 50);
            for (b, r) in bodies.iter_mut().zip(&reactions) {
                b.integrate(r, Vec3::ZERO);
            }
        }
        // Only the spin about Y is left.
        assert!(
            bodies[0].spin.x.abs() < 0.05 && bodies[0].spin.z.abs() < 0.05,
            "{:?}",
            bodies[0].spin
        );
        assert!((bodies[0].spin.y - 5.0).abs() < 0.2, "{:?}", bodies[0].spin);
    }

    #[test]
    fn a_hard_angular_drive_turns_a_body_at_most_its_speed() {
        let mut fixed = Body::new(
            Mass::sphere(1.0, 0.1, f32::MAX),
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ZERO,
        );
        fixed.active = false;
        let mut bodies = [
            Body::new(
                Mass::sphere(1.0, 0.1, f32::MAX),
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::ZERO,
            ),
            fixed,
        ];
        let target = DriveFrame {
            orientation: Quat::from_axis_angle(Vec3::UP, 1.0),
            translation: Vec3::ZERO,
        };
        let here = DriveFrame {
            orientation: Quat::IDENTITY,
            translation: Vec3::ZERO,
        };
        // Skate 3's truck drive: hard, 10 rad a second at most, strength 11 x 3600.
        let d = DriveDynamics {
            linear: DriveParams::OFF,
            angular: DriveParams {
                spring_or_max_velocity: 600.0,
                damping: 0.0,
                max_strength: 11.0 * 3600.0,
                kind: DriveKind::Hard,
            },
        };
        let mut angles = vec![];
        for _ in 0..30 {
            let mut drives = [DriveRow::build(&bodies, 0, 1, &here, &target, &d)];
            let mut reactions = vec![Reaction::default(); 2];
            solve(&mut [], &mut [], &mut drives, &mut reactions, 50);
            for (b, r) in bodies.iter_mut().zip(&reactions) {
                b.integrate(r, Vec3::ZERO);
            }
            let nose = bodies[0].axes.cols[2];
            angles.push(libm::atan2f(nose.x, nose.z));
        }
        assert!((angles[29] - 1.0).abs() < 1e-3, "{angles:?}");
        assert!(angles[0] > 0.1, "{angles:?}");
    }
}
