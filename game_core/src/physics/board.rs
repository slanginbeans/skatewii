//! Skate 3's board: deck, two trucks and four wheels jointed together and solved as one (SK8
//! board_step.rs `BoardStep::advance`, assembly.rs, board.rs, joint_records.rs, drive_frames.rs,
//! truck_frames.rs, drive_parameters.rs, mass.rs; the stock `physicsdeck`, `physicstrucks`,
//! `physicstrucks_drives`, `physicswheels` and `physics_surfaces` values).
//!
//! The deck's axes are the board's: X to its left (up x nose), Y up, Z to the nose. The trucks turn
//! about their 31 degree pivot axes to the steering's targets (hard angular drives); the wheels spin
//! freely on their axles and touch the ground with the surface's wheel friction.

use super::contact::combine;
use super::{
    Body, ContactInput, ContactRow, DriveDynamics, DriveFrame, DriveKind, DriveParams, DriveRow,
    JointFrames, JointParams, JointRow, Quat, Reaction, body::Mass,
};
use crate::math::{Mat3, Vec3, v3};
use crate::park::Terrain;

pub const RIGHT_FRONT_WHEEL: usize = 0;
pub const LEFT_FRONT_WHEEL: usize = 1;
pub const RIGHT_BACK_WHEEL: usize = 2;
pub const LEFT_BACK_WHEEL: usize = 3;
pub const FRONT_TRUCK: usize = 4;
pub const BACK_TRUCK: usize = 5;
pub const DECK: usize = 6;
pub const PARTS: usize = 7;

/// `physicswheels`: the wheels' radius and their distance from the board's middle line (m).
pub const WHEEL_RADIUS: f32 = 0.031;
pub const WHEEL_X: f32 = 0.095;
/// `physicstrucks` `TruckYPos`, `TruckZPosFront`/`Back` and `physicsdeck` `DeckMidLength`: the
/// trucks' axles under the deck's centre of mass and along it (m).
pub const TRUCK_Y: f32 = -0.0565;
pub const TRUCK_Z: f32 = 0.59 * 0.5 - 0.052;
/// `TruckRotationAxisAngle` (degrees): the trucks' pivot axes from the deck's plane.
pub const TRUCK_AXIS_ANGLE: f32 = 31.0;
/// `physicsdeck` `DeckForceYOffset`: queued forces act this far above their deck point (m).
pub const FORCE_Y_OFFSET: f32 = 0.05;
/// The skater simulation's constraint iterations (SK8 settings.rs `SIMULATION_ITERATIONS`, measured
/// in TU3).
pub const ITERATIONS: u32 = 50;
/// `physics/default` `WorldGravity`.
pub const GRAVITY: Vec3 = v3(0.0, -9.8, 0.0);
/// The deck's centre of mass over the ground with all four wheels down (m).
pub const DECK_HEIGHT: f32 = WHEEL_RADIUS - TRUCK_Y;

/// The parts' masses (SK8 mass.rs for the stock collections, `SkateboardMassFactor` 5): wheel spheres
/// of 0.083 x 5 kg; truck capsules of 0.39 x 5 kg; the deck 1.2 x 5 kg with its boxes' and end
/// capsules' aggregate inertia and `DeckAngularDrag` 0.45 x 60.
pub fn wheel_mass() -> Mass {
    Mass::sphere(0.083 * 5.0, WHEEL_RADIUS, 59_999.996)
}
pub fn truck_mass() -> Mass {
    Mass {
        inverse_inertia: v3(285.8012, 285.8012, 18470.771),
        inverse_mass: 1.0 / (0.39 * 5.0),
        max_speed: f32::MAX,
        max_spin: f32::MAX,
        linear_drag: 0.0,
        angular_drag: 0.0,
    }
}
pub fn deck_mass() -> Mass {
    Mass {
        inverse_inertia: v3(1.926_521_9, 1.841_311_8, 37.752_766),
        inverse_mass: 1.0 / 6.0,
        max_speed: f32::MAX,
        max_spin: f32::MAX,
        linear_drag: 0.0,
        angular_drag: 0.45 * 60.0,
    }
}
/// The board's mass (kg): what turns a queued acceleration into its force.
pub fn total_mass() -> f32 {
    4.0 * 0.083 * 5.0 + 2.0 * 0.39 * 5.0 + 1.2 * 5.0
}

/// Wheel friction on the surface (`physics_surfaces/smooth`: static, dynamic) and restitution
/// (`WheelRestitution`); the world's own material adds nothing (static 0, dynamic 0, restitution 1).
pub const WHEEL_FRICTION: [f32; 3] = [0.8, 0.7, 0.0];
const FLOOR: [f32; 3] = [0.0, 0.0, 1.0];

/// The trucks' joint and drive frames on the deck with the steering's angles (SK8 drive_frames.rs
/// `calculate_truck_transforms`, truck_frames.rs `steering_truck_transforms`): each truck's X is its
/// pivot axis, tipped `TRUCK_AXIS_ANGLE` out of the deck's plane towards its end; its Z the axle.
pub fn truck_frame(front: bool, angle: f32) -> (Mat3, Vec3) {
    let alpha = TRUCK_AXIS_ANGLE.to_radians();
    let base = if front {
        Mat3::rot_x(-alpha).mul(&Mat3::rot_y(-core::f32::consts::FRAC_PI_2))
    } else {
        Mat3::rot_x(alpha).mul(&Mat3::rot_y(core::f32::consts::FRAC_PI_2))
    };
    let z = if front { TRUCK_Z } else { -TRUCK_Z };
    (base.mul(&Mat3::rot_x(angle)), v3(0.0, TRUCK_Y, z))
}

/// The wheels' joint frame: a quarter turn about X, so the frame's Y (the free spin) is the truck's
/// Z, the axle (SK8 joint_records.rs `wheel_joint_orientation`).
fn wheel_frame() -> Quat {
    Quat::from_mat3(&Mat3::rot_x(core::f32::consts::FRAC_PI_2))
}

/// Which truck and which side of it each wheel is on (+ along the truck's axle).
const WHEELS: [(usize, f32); 4] = [
    (FRONT_TRUCK, WHEEL_X),
    (FRONT_TRUCK, -WHEEL_X),
    (BACK_TRUCK, WHEEL_X),
    (BACK_TRUCK, -WHEEL_X),
];

/// The trucks' joints: swing locked, twist about the pivot within 14 degrees, at most 86.1 degrees a
/// tick (`physicstrucks`); the wheels': all locked but the spin (SK8 joint_records.rs).
fn truck_joint() -> JointParams {
    JointParams {
        position_allowance: Vec3::ZERO,
        velocity_allowance: Vec3::ZERO,
        twist_velocity: 86.1_f32.to_radians() * 60.0,
        swing_velocity: 0.0,
        swing_threshold: 1.0,
        twist_threshold: libm::cosf(14.0_f32.to_radians()),
        swing_mode: 0,
        twist_mode: 1,
    }
}
fn wheel_joint() -> JointParams {
    JointParams {
        position_allowance: Vec3::ZERO,
        velocity_allowance: Vec3::ZERO,
        twist_velocity: 0.0,
        swing_velocity: 1.0e6_f32.to_radians() * 60.0,
        swing_threshold: 1.0,
        twist_threshold: 1.0,
        swing_mode: 3,
        twist_mode: 0,
    }
}

/// The truck drives (SK8 drive_parameters.rs `retail_truck_drive_dynamics`): no linear drive
/// (`UseTruckDrives` off), a hard angular drive of `Angular_Hard_Displacement` 10 x 60 rad/s at most,
/// `Angular_Hard_Strength` 11 x 3600.
pub const TRUCK_DRIVE: DriveDynamics = DriveDynamics {
    linear: DriveParams::OFF,
    angular: DriveParams {
        spring_or_max_velocity: 10.0 * 60.0,
        damping: 0.0,
        max_strength: 11.0 * 3600.0,
        kind: DriveKind::Hard,
    },
};

/// A wheel on the ground after a step: where it touches, the surface's normal there and the load it
/// carried (N).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelContact {
    pub point: Vec3,
    pub normal: Vec3,
    pub load: f32,
}

/// What the riding puts on the board this tick (SK8's force queue and deck corrections).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardInput {
    /// World forces (N) at deck-local points.
    pub forces: alloc::vec::Vec<(Vec3, Vec3)>,
    /// Turns of the deck this tick on top of its motion (rad, world): the manual's.
    pub angular: Vec3,
    /// Limited turns (rad, world): heading, straightening, collision turns.
    pub limited: Vec3,
    /// The trucks' angles about their pivots (rad; SK8 `TruckSteeringState::targets`, front then
    /// back), the back one turned the other way by its frame.
    pub trucks: [f32; 2],
    /// The wheels' friction on the surface (static, dynamic).
    pub wheel_friction: [f32; 2],
    /// The ground's linear drag on every part this tick (per second; SK8 grounded/drag.rs: 60 while
    /// a brake holds a slow board, which stops it dead).
    pub linear_drag: f32,
}

/// The board's seven bodies.
#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub parts: [Body; PARTS],
    pub wheels: [Option<WheelContact>; 4],
}

impl Board {
    /// The board put down with its deck's centre of mass at `deck` and its axes `axes`, the trucks
    /// straight, at rest.
    pub fn new(deck: Vec3, axes: &Mat3) -> Board {
        let q = Quat::from_mat3(axes);
        let mut parts = [Body::new(wheel_mass(), deck, q, GRAVITY); PARTS];
        parts[FRONT_TRUCK] = Body::new(truck_mass(), deck, q, GRAVITY);
        parts[BACK_TRUCK] = Body::new(truck_mass(), deck, q, GRAVITY);
        parts[DECK] = Body::new(deck_mass(), deck, q, GRAVITY);
        let mut b = Board {
            parts,
            wheels: [None; 4],
        };
        b.place(deck, axes, Vec3::ZERO, Vec3::ZERO);
        b
    }

    pub fn deck(&self) -> &Body {
        &self.parts[DECK]
    }

    /// Puts the whole board at `deck` and `axes` with the trucks straight, moving at `velocity` and
    /// turning at `spin`, its wheels rolling with it (the hook from outside: a landing, a respawn, a
    /// kinematic move).
    pub fn place(&mut self, deck: Vec3, axes: &Mat3, velocity: Vec3, spin: Vec3) {
        let q = Quat::from_mat3(axes);
        let up = axes.cols[1];
        let d = &mut self.parts[DECK];
        d.set_orientation(q);
        d.position = deck;
        d.velocity = velocity;
        d.spin = spin;
        for (i, front) in [(FRONT_TRUCK, true), (BACK_TRUCK, false)] {
            let (basis, at) = truck_frame(front, 0.0);
            let p = &mut self.parts[i];
            p.set_orientation(q.compose(Quat::from_mat3(&basis)));
            let arm = axes.apply(at);
            p.position = deck + arm;
            p.velocity = velocity + spin.cross(arm);
            p.spin = spin;
        }
        for (w, &(truck, side)) in WHEELS.iter().enumerate() {
            let t = self.parts[truck];
            let arm = t.axes.apply(v3(0.0, 0.0, side));
            let p = &mut self.parts[w];
            p.set_orientation(t.orientation);
            p.position = t.position + arm;
            let v = velocity + spin.cross(p.position - deck);
            p.velocity = v;
            // Rolling: the contact point at rest.
            p.spin = spin + up.cross(v - up * up.dot(v)) * (1.0 / WHEEL_RADIUS);
        }
        for p in &mut self.parts {
            p.force = GRAVITY;
            p.torque = Vec3::ZERO;
        }
        self.wheels = [None; 4];
    }

    /// The wheels' contacts with the terrain: each wheel sphere against the surface pieces it reaches
    /// (SK8 queries the triangles a sphere touches). The surface is looked for down the deck's up from
    /// the wheel's centre and from a radius ahead of and behind it along its travel; each face found
    /// gives a contact where the wheel's closest point on its plane really is on that face, so over a
    /// concave kink the wheel touches both faces (at most two contacts a wheel), and steep transitions
    /// up to vert are met square to them.
    fn wheel_contacts(
        &self,
        park: &dyn Terrain,
        friction: [f32; 2],
    ) -> [[Option<ContactInput>; 2]; 4] {
        let m = combine([friction[0], friction[1], WHEEL_FRICTION[2]], FLOOR);
        let up = self.parts[DECK].axes.cols[1];
        const ABOVE: f32 = 0.1;
        core::array::from_fn(|w| {
            let wheel = &self.parts[w];
            let c = wheel.position;
            let v = wheel.velocity;
            let travel = (v - up * up.dot(v)).normalize_or(self.parts[DECK].axes.cols[2]);
            let approach = (-v.dot(up)).max(0.0) * super::STEP;
            let mut out: [Option<ContactInput>; 2] = [None; 2];
            for offset in [0.0, WHEEL_RADIUS, -WHEEL_RADIUS] {
                let from = c + travel * offset + up * ABOVE;
                let Some(hit) = park.raycast(from, -up, ABOVE + WHEEL_RADIUS + 0.05 + approach)
                else {
                    continue;
                };
                let n = hit.normal;
                // Faces the wheel comes at from behind (walls, the underside of things) are not ridden.
                if n.dot(up) < 0.3 {
                    continue;
                }
                let gap = (c - hit.point).dot(n);
                let reach = WHEEL_RADIUS + 0.02 + (-v.dot(n)).max(0.0) * super::STEP;
                if gap > reach || gap < -WHEEL_RADIUS {
                    continue;
                }
                let touch = c - n * gap;
                if offset != 0.0 {
                    // The plane's closest point must be on this face, not on its extension.
                    let back = park.raycast(touch + n * 0.01, -n, 0.02);
                    if back.is_none_or(|b| b.normal.dot(n) < 0.999) {
                        continue;
                    }
                }
                if out.iter().flatten().any(|o| o.normal.dot(n) > 0.999) {
                    continue;
                }
                let Some(slot) = out.iter_mut().find(|o| o.is_none()) else {
                    break;
                };
                *slot = Some(ContactInput {
                    point_a: c - n * WHEEL_RADIUS,
                    point_b: touch,
                    normal: n,
                    restitution: m[2],
                    static_friction: m[0],
                    dynamic_friction: m[1],
                });
            }
            out
        })
    }

    /// One tick (SK8 `BoardStep::advance`): the queued forces and deck turns, the rows from this
    /// state, the shared solve and every part's integration.
    pub fn step(&mut self, park: &dyn Terrain, input: &BoardInput) {
        let deck = &mut self.parts[DECK];
        for &(f, p) in &input.forces {
            deck.add_force_at(f, p + v3(0.0, FORCE_Y_OFFSET, 0.0));
        }
        deck.add_angular_displacement(input.angular);
        deck.add_limited_displacement(input.limited);

        let found = self.wheel_contacts(park, input.wheel_friction);
        let mut contacts = [ContactRow::default(); 8];
        let mut count = 0;
        let mut which = [[None; 2]; 4];
        for (w, cs) in found.iter().enumerate() {
            for (k, c) in cs.iter().enumerate() {
                if let Some(c) = c {
                    which[w][k] = Some(count);
                    contacts[count] = ContactRow::build(&self.parts, Some(w), None, c);
                    count += 1;
                }
            }
        }
        let wheel_frame = wheel_frame();
        let mut joints: [JointRow; 6] = core::array::from_fn(|i| {
            if i < 2 {
                let (truck, front) = if i == 0 {
                    (FRONT_TRUCK, true)
                } else {
                    (BACK_TRUCK, false)
                };
                let (basis, at) = truck_frame(front, 0.0);
                let q = Quat::from_mat3(&basis);
                let f = JointFrames {
                    orientation_a: Quat::IDENTITY,
                    anchor_a: Vec3::ZERO,
                    orientation_b: q,
                    anchor_b: at,
                    linear_orientation_b: q,
                };
                JointRow::build(&self.parts, truck, DECK, &truck_joint(), &f)
            } else {
                // The wheels' joints in mirror order (front right, front left, back left, back
                // right): Skate 3's (back right before back left) is chiral, and the sequential solve
                // turned a standing board 0.042 rad in ten seconds with it (SK8's holds still).
                let w = [0, 1, 3, 2][i - 2];
                let (truck, side) = WHEELS[w];
                let f = JointFrames {
                    orientation_a: wheel_frame,
                    anchor_a: Vec3::ZERO,
                    orientation_b: wheel_frame,
                    anchor_b: v3(0.0, 0.0, side),
                    linear_orientation_b: wheel_frame,
                };
                JointRow::build(&self.parts, w, truck, &wheel_joint(), &f)
            }
        });
        let here = DriveFrame {
            orientation: Quat::IDENTITY,
            translation: Vec3::ZERO,
        };
        let mut drives: [DriveRow; 2] = core::array::from_fn(|i| {
            let front = i == 0;
            // The back truck's frame faces the other way: the same steering turns it back.
            let angle = if front {
                input.trucks[0]
            } else {
                -input.trucks[1]
            };
            let (basis, at) = truck_frame(front, angle);
            let target = DriveFrame {
                orientation: Quat::from_mat3(&basis),
                translation: at,
            };
            let truck = if front { FRONT_TRUCK } else { BACK_TRUCK };
            DriveRow::build(&self.parts, truck, DECK, &here, &target, &TRUCK_DRIVE)
        });
        let mut reactions = [Reaction::default(); PARTS];
        super::solve(
            &mut contacts[..count],
            &mut joints,
            &mut drives,
            &mut reactions,
            ITERATIONS,
        );
        for (p, r) in self.parts.iter_mut().zip(&reactions) {
            p.mass.linear_drag = input.linear_drag;
            p.integrate(r, GRAVITY);
        }
        // A wheel on two faces reports their load-weighted normal and point.
        self.wheels = core::array::from_fn(|w| {
            let mut load = 0.0;
            let mut normal = Vec3::ZERO;
            let mut point = Vec3::ZERO;
            let mut any = None;
            for k in 0..2 {
                if let (Some(i), Some(c)) = (which[w][k], found[w][k]) {
                    let l = contacts[i].normal_force().max(0.0);
                    any.get_or_insert((c.normal, c.point_b));
                    load += l;
                    normal += c.normal * l;
                    point += c.point_b * l;
                }
            }
            any.map(|(n, p)| {
                if load > 0.0 {
                    WheelContact {
                        point: point * (1.0 / load),
                        normal: normal.normalize_or(n),
                        load,
                    }
                } else {
                    WheelContact {
                        point: p,
                        normal: n,
                        load: 0.0,
                    }
                }
            })
        });
    }

    /// The velocity of the board's centre of mass.
    pub fn velocity(&self) -> Vec3 {
        let mut momentum = Vec3::ZERO;
        let mut mass = 0.0;
        for p in &self.parts {
            let m = 1.0 / p.mass.inverse_mass;
            momentum += p.velocity * m;
            mass += m;
        }
        momentum * (1.0 / mass)
    }

    /// The wheels touching the ground after the last step.
    pub fn wheels_down(&self) -> usize {
        self.wheels
            .iter()
            .filter(|w| w.is_some_and(|w| w.load > 0.0))
            .count()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::park::Surface;

    /// Flat ground at y = 0, falling to `drop` beyond z = `edge`.
    pub(crate) struct Ledge {
        pub edge: f32,
        pub drop: f32,
    }
    impl Terrain for Ledge {
        fn ground(&self, _x: f32, z: f32, _max: f32) -> Surface {
            Surface {
                height: if z > self.edge { self.drop } else { 0.0 },
                normal: Vec3::UP,
            }
        }
        fn spawn(&self) -> (Vec3, f32) {
            (Vec3::ZERO, 0.0)
        }
    }
    pub(crate) const FLAT: Ledge = Ledge {
        edge: f32::MAX,
        drop: 0.0,
    };

    fn input(trucks: f32) -> BoardInput {
        BoardInput {
            trucks: [trucks, trucks],
            wheel_friction: [WHEEL_FRICTION[0], WHEEL_FRICTION[1]],
            ..BoardInput::default()
        }
    }

    fn yaw(b: &Board) -> f32 {
        let f = b.deck().axes.cols[2];
        libm::atan2f(f.x, f.z)
    }

    #[test]
    fn the_board_rests_on_its_wheels_carrying_its_own_weight() {
        let mut b = Board::new(v3(0.0, DECK_HEIGHT + 0.01, 0.0), &Mat3::IDENTITY);
        for _ in 0..60 {
            b.step(&FLAT, &input(0.0));
        }
        let load: f32 = b.wheels.iter().flatten().map(|w| w.load).sum();
        // SK8's board on its course: the wheels carry 113.3 N, the deck 0.0875 m up.
        assert!((load - 113.3).abs() < 1.0, "{load} {:?}", b.wheels);
        assert!(
            (b.deck().position.y - DECK_HEIGHT).abs() < 2e-3,
            "{:?}",
            b.deck().position
        );
        assert_eq!(b.wheels_down(), 4);
        assert!(b.deck().velocity.length() < 0.01);
        for _ in 0..600 {
            b.step(&FLAT, &input(0.0));
        }
        // Standing, it doesn't turn (SK8's holds still).
        assert!(yaw(&b).abs() < 1e-3, "{}", yaw(&b));
    }

    /// Host cost of one board step (run with `--ignored --nocapture` in release).
    #[test]
    #[ignore]
    fn board_step_cost() {
        let mut b = Board::new(v3(0.0, DECK_HEIGHT, 0.0), &Mat3::IDENTITY);
        b.place(
            v3(0.0, DECK_HEIGHT, 0.0),
            &Mat3::IDENTITY,
            v3(0.0, 0.0, 6.5),
            Vec3::ZERO,
        );
        let i = input(0.3);
        let t = std::time::Instant::now();
        for _ in 0..2000 {
            b.step(&FLAT, &i);
        }
        std::eprintln!("board step: {} us", t.elapsed().as_micros() as f32 / 2000.0);
    }

    #[test]
    fn a_straight_board_rolls_on_without_turning_or_slowing() {
        let mut b = Board::new(v3(0.0, DECK_HEIGHT, 0.0), &Mat3::IDENTITY);
        b.place(
            v3(0.0, DECK_HEIGHT, 0.0),
            &Mat3::IDENTITY,
            v3(0.0, 0.0, 6.5),
            Vec3::ZERO,
        );
        for _ in 0..120 {
            b.step(&FLAT, &input(0.0));
        }
        let d = b.deck();
        assert!((d.velocity.z - 6.5).abs() < 0.05, "{:?}", d.velocity);
        assert!(
            yaw(&b).abs() < 0.03 && d.velocity.x.abs() < 0.2,
            "{} {:?}",
            yaw(&b),
            d.velocity
        );
    }

    #[test]
    fn tilted_trucks_carve_the_board_round_and_roll_its_deck() {
        let mut b = Board::new(v3(0.0, DECK_HEIGHT, 0.0), &Mat3::IDENTITY);
        b.place(
            v3(0.0, DECK_HEIGHT, 0.0),
            &Mat3::IDENTITY,
            v3(0.0, 0.0, 6.5),
            Vec3::ZERO,
        );
        // SK8's full-stick carve at 6.5 m/s: trucks at 0.3785 rad, the deck rolled 0.288 to the turn.
        let start = yaw(&b);
        for _ in 0..60 {
            b.step(&FLAT, &input(0.3785));
        }
        let turned = yaw(&b) - start;
        let roll = libm::asinf(b.deck().axes.cols[0].y);
        assert!(turned > 0.3, "{turned}");
        assert!(roll < -0.1, "{roll}");
    }

    #[test]
    fn rolling_off_a_ledge_the_board_pivots_over_its_back_wheels() {
        let mut b = Board::new(v3(0.0, DECK_HEIGHT, 0.0), &Mat3::IDENTITY);
        b.place(
            v3(0.0, DECK_HEIGHT, 0.0),
            &Mat3::IDENTITY,
            v3(0.0, 0.0, 1.5),
            Vec3::ZERO,
        );
        let ledge = Ledge {
            edge: 0.3,
            drop: -1.0,
        };
        let mut pivoted = false;
        for _ in 0..40 {
            b.step(&ledge, &input(0.0));
            let pitch = b.deck().axes.cols[2].y;
            let front =
                b.wheels[RIGHT_FRONT_WHEEL].is_some() || b.wheels[LEFT_FRONT_WHEEL].is_some();
            let back = b.wheels[RIGHT_BACK_WHEEL].is_some() && b.wheels[LEFT_BACK_WHEEL].is_some();
            if !front && back && pitch < -0.1 {
                pivoted = true;
            }
        }
        assert!(
            pivoted,
            "the nose never tipped over the edge on the back wheels"
        );
    }
}
