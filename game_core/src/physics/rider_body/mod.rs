//! The rider's physical skeleton in the board's solve (SK8-ENGINE/skate-3-rust-engine, GPL-3.0:
//! skate-core physics/skeleton_body/{joints,drives,drive_dynamics,drive_frames,targets,target_update,
//! collision_mode,collision_update,collision_feedback,errors}.rs, physics/skeleton_board_frames.rs,
//! physics/board_step.rs `advance_attached`; skate-game physics/solve.rs).
//!
//! Skate 3 solves the rider's 26 bodies ([`super::skeleton`]) in the same pass as the board: 22 joints
//! hold each bone to its physical parent, every bone is driven to the animated pose twice (to its parent,
//! "LOCAL", and to the hips, "LOCAL_ROOT"), the hips to the animated hips by an immovable target, and
//! the bones touch the world, the board and (forearms against the trunk and thighs) each other. The
//! contacts feed SkeletonCollision's feedback: the contact flags and the pose error that the riding's
//! collision response reads.
//!
//! Two reductions keep it inside the Wii's tick (docs/graph-runner.md, "The rider's body", has SK8's
//! traces for both):
//! - bones that change nothing are merged ([`MERGED`]: the hands into the forearms, the toes into the
//!   feet, the neck into the head) or follow their targets ([`KINEMATIC`]: the skateboard root and the
//!   two extra capsules, which no joint, drive or contact ties to another body);
//! - the skeleton is solved only when its result can differ from the animated pose: in the air, or when
//!   a bone can touch a wall or the board. Otherwise it follows the animated pose.

mod tables;
pub use tables::{INITIAL_BONES, PHYS_BONES, STOCK_JOINTS};

use super::board as pb;
use super::contact::combine;
use super::skeleton::{ANIMATED_PARTS, BONES, PARTS, Shape, stock_parts};
use super::{
    Body, ContactInput, ContactRow, DriveDynamics, DriveFrame, DriveKind, DriveParams, DriveRow,
    JointFrames, JointParams, JointRow, Quat, Reaction, STEP,
};
use crate::math::{Mat3, Vec3, Xform, v3};
use crate::park::Terrain;
use alloc::vec::Vec;

pub const JOINTS: usize = 22;

/// A joint between a bone (body A, the child) and its physical parent (body B).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointRecord {
    pub parent: usize,
    pub child: usize,
    pub params: JointParams,
    pub frames: JointFrames,
}

/// A bone's `PHYS_TPOSE` frames: the joint's orientation in the parent and in the bone (words 0-7), the
/// volume's frame on the animation bone (12-18) and the twist and swing limits (20, 21).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysBone {
    pub parent_orientation: Quat,
    pub joint_orientation: Quat,
    pub volume_orientation: Quat,
    pub volume_translation: Vec3,
    pub twist_limit: f32,
    pub swing_limit: f32,
}

impl PhysBone {
    /// The physical body's frame on its animation bone (SK8 `physics_bone_frame`).
    pub fn volume(&self) -> Xform {
        Xform::new(self.volume_orientation.to_mat3(), self.volume_translation)
    }
}

/// `physics_skeleton_joints/default/JOINT_*`: a ball joint, and the swing and twist limits' scales.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointSettings {
    pub ball_joint: bool,
    pub swing_angle: f32,
    pub twist_angle: f32,
}

const fn js(swing_angle: f32, twist_angle: f32) -> JointSettings {
    JointSettings {
        ball_joint: false,
        swing_angle,
        twist_angle,
    }
}

/// The stock joints' settings, in SK8's `JOINT_NAMES` order (the children 1 to 22).
pub const JOINT_SETTINGS: [JointSettings; JOINTS] = [
    js(1.0, 1.0),
    js(1.2, 1.0),
    js(1.9, 2.1),
    js(2.1, 2.4),
    js(1.6, 3.1),
    js(1.1, 1.1),
    js(1.9, 2.1),
    js(2.1, 2.4),
    js(1.6, 3.1),
    js(1.1, 1.1),
    js(1.0, 1.0),
    js(1.0, 1.0),
    js(1.0, 1.0),
    js(1.4, 1.6),
    js(1.2, 1.2),
    js(2.5, 2.5),
    js(2.5, 2.5),
    js(2.5, 2.5),
    js(1.2, 1.2),
    js(2.5, 2.5),
    js(2.5, 2.5),
    js(2.5, 2.5),
];

/// `physics_skeleton/default` `DisplacementLimit`, `TwistDisplacementLimit`, `SwingDisplacementLimit`
/// and `physics_skeleton_joints/default` `EnforceSwingFree`, `EnforceTwistFree`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobalJointSettings {
    pub displacement_limit: Vec3,
    pub twist_displacement_limit: f32,
    pub swing_displacement_limit: f32,
    pub enforce_swing_free: bool,
    pub enforce_twist_free: bool,
}

impl GlobalJointSettings {
    pub const STOCK: GlobalJointSettings = GlobalJointSettings {
        displacement_limit: v3(0.01, 0.01, 0.01),
        twist_displacement_limit: 10.0,
        swing_displacement_limit: 10.0,
        enforce_swing_free: false,
        enforce_twist_free: false,
    };
}

/// Each bone's physical parent (the nearest physical ancestor in the rig); the skateboard root and the
/// hips have none.
pub const PARENTS: [Option<usize>; ANIMATED_PARTS] = [
    None,
    Some(2),
    Some(11),
    Some(4),
    Some(5),
    Some(6),
    Some(11),
    Some(8),
    Some(9),
    Some(10),
    Some(11),
    Some(12),
    Some(13),
    Some(14),
    Some(23),
    Some(16),
    Some(17),
    Some(18),
    Some(23),
    Some(20),
    Some(21),
    Some(22),
    Some(23),
    None,
];

/// The left and right bones' partners (a mirrored rider's left bone is the clips' right one).
pub const PARTNER: [usize; ANIMATED_PARTS] = [
    0, 1, 2, 7, 8, 9, 10, 3, 4, 5, 6, 11, 12, 13, 14, 19, 20, 21, 22, 15, 16, 17, 18, 23,
];

/// SK8 `inverse_native_step` and the drives' `FREQUENCY`, `FREQUENCY_SQUARED` (TU3's constants).
const INVERSE_STEP: f32 = 59.999_996;
const FREQUENCY: f32 = f32::from_bits(0x426F_FFFF);
const FREQUENCY_SQUARED: f32 = f32::from_bits(0x4560_FFFE);
const HARD_VELOCITY: f32 = f32::from_bits(0x4415_FFFF);
const HARD_STRENGTH: f32 = f32::from_bits(0x470C_9FFF);

/// The rigid inverse of `x`.
pub fn inverse(x: &Xform) -> Xform {
    let c = &x.rot.cols;
    let t = Mat3::from_cols(
        v3(c[0].x, c[1].x, c[2].x),
        v3(c[0].y, c[1].y, c[2].y),
        v3(c[0].z, c[1].z, c[2].z),
    );
    Xform::new(t, -t.apply(x.pos))
}

fn rotation(q: Quat) -> Xform {
    Xform::new(q.to_mat3(), Vec3::ZERO)
}

/// SK8 `SkeletonJoints::new` (TU3 InitJoints 0x82BE5B30 and SetUpNormalJointLimits 0x82BE58A8): each
/// bone's joint to its parent, its frames from the rig's initial bone frames `initial` and the bones'
/// `PHYS_TPOSE` frames, its limits from the settings.
pub fn build_joints(
    initial: &[Xform; ANIMATED_PARTS],
    parents: &[Option<usize>; ANIMATED_PARTS],
    bones: &[PhysBone; ANIMATED_PARTS],
    settings: &[JointSettings; JOINTS],
    global: &GlobalJointSettings,
) -> [JointRecord; JOINTS] {
    let mut out = [STOCK_JOINTS[0]; JOINTS];
    let mut n = 0;
    for child in 0..ANIMATED_PARTS {
        let Some(parent) = parents[child] else {
            continue;
        };
        let s = settings[n];
        let bone = bones[child];
        let inverse_child = inverse(&bone.volume());
        let inverse_parent = inverse(&bones[parent].volume());
        let parent_joint = inverse_parent.then(&inverse(&rotation(bone.parent_orientation)));
        let joint = rotation(bone.joint_orientation);
        let relative = inverse(&initial[parent]).then(&initial[child]);
        let parent_anchor = inverse_parent.then(&relative).pos;
        let child_frame = inverse_child.then(&joint);
        let parent_frame = parent_joint.then(&joint);
        let minimum = |v: f32| v.max(0.01);
        let swing = minimum(bone.swing_limit * s.swing_angle);
        let twist = minimum(bone.twist_limit * s.twist_angle);
        out[n] = JointRecord {
            parent,
            child,
            params: JointParams {
                position_allowance: Vec3::ZERO,
                velocity_allowance: global.displacement_limit * INVERSE_STEP,
                twist_velocity: global.twist_displacement_limit * FREQUENCY,
                swing_velocity: global.swing_displacement_limit * FREQUENCY,
                swing_threshold: libm::cosf(swing),
                twist_threshold: libm::cosf(twist),
                swing_mode: if s.ball_joint || global.enforce_swing_free {
                    4
                } else {
                    1
                },
                twist_mode: if s.ball_joint || global.enforce_twist_free {
                    2
                } else {
                    1
                },
            },
            frames: JointFrames {
                orientation_a: Quat::from_mat3(&child_frame.rot),
                anchor_a: inverse_child.pos,
                orientation_b: Quat::from_mat3(&parent_frame.rot),
                anchor_b: parent_anchor,
                linear_orientation_b: Quat::IDENTITY,
            },
        };
        n += 1;
    }
    assert_eq!(n, JOINTS, "the physical skeleton has 22 joints");
    out
}

/// Reduction 1: bones merged into a neighbour (child, host). Their mass rides on the host, their
/// volumes keep touching at their animated place on it.
pub const MERGED: [(usize, usize); 5] = [(3, 4), (7, 8), (15, 16), (19, 20), (2, 1)];
/// Bodies with no joint, drive or contact to another (the skateboard root, held to the physical
/// deck; the extra capsules, held to the centre of mass): they follow their targets.
pub const KINEMATIC: [usize; 3] = [0, 24, 25];

/// The body that carries `part` in the solve.
pub const fn host(part: usize) -> usize {
    let mut i = 0;
    while i < MERGED.len() {
        if MERGED[i].0 == part {
            return MERGED[i].1;
        }
        i += 1;
    }
    part
}

/// The solved bodies: the 23 bones less the merged ones.
pub const SOLVED: usize = ANIMATED_PARTS - 1 - MERGED.len();

const fn solved_parts() -> [usize; SOLVED] {
    let mut out = [0; SOLVED];
    let (mut part, mut n) = (1, 0);
    while part < ANIMATED_PARTS {
        if host(part) == part {
            out[n] = part;
            n += 1;
        }
        part += 1;
    }
    out
}
pub const SOLVED_PARTS: [usize; SOLVED] = solved_parts();

/// Each part's host's place among `SOLVED_PARTS` (the skateboard root, which has none, at 0).
const fn solved_slots() -> [usize; ANIMATED_PARTS] {
    let mut out = [0; ANIMATED_PARTS];
    let mut k = 0;
    while k < SOLVED {
        out[SOLVED_PARTS[k]] = k;
        k += 1;
    }
    let mut part = 0;
    while part < ANIMATED_PARTS {
        out[part] = out[host(part)];
        part += 1;
    }
    out
}
const SOLVED_SLOT: [usize; ANIMATED_PARTS] = solved_slots();

/// The self-collision pairs SkeletonCollision doesn't cull (forearms against the trunk and thighs).
const SELF_PAIRS: [(usize, usize); 6] = [(14, 4), (14, 8), (23, 4), (23, 8), (4, 18), (8, 22)];

/// `physics_skeleton_drives` and `animation/default`: the two channels' animation drives (linear
/// strength, linear displacement, angular strength, angular displacement), the collision drive and
/// `DriveStrengthLocal`, `DriveStrengthRootLocal`, `Use_Drives`, and each part's collision strengths
/// (`PART_*` words 5 and 6 times `CollisionDriveScalar`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveSettings {
    pub animation: [[f32; 4]; 2],
    pub collision_soft_displacement: f32,
    pub collision_soft_strength: f32,
    pub enabled: bool,
    pub strength: [f32; 2],
    pub collision_strength: [[f32; 2]; ANIMATED_PARTS],
}

impl DriveSettings {
    pub const STOCK: DriveSettings = DriveSettings {
        animation: [[1.0, 1.0, 4.0, 4.0]; 2],
        collision_soft_displacement: 25000.0,
        collision_soft_strength: 100.0,
        enabled: true,
        strength: [1.0, 1.0],
        collision_strength: [
            [0.0, 0.0],
            [1.0, 0.001],
            [0.5, 0.001],
            [0.5, 0.0],
            [0.5, 0.0],
            [0.5, 0.0],
            [0.5, 0.001],
            [0.5, 0.0],
            [0.5, 0.0],
            [0.5, 0.0],
            [0.5, 0.001],
            [0.2, 0.0001],
            [0.2, 0.1],
            [0.2, 0.1],
            [0.2, 0.1],
            [0.5, 0.5],
            [0.5, 0.5],
            [0.25, 0.25],
            [0.25, 0.25],
            [0.5, 0.5],
            [0.5, 0.5],
            [0.25, 0.25],
            [0.25, 0.25],
            [0.5, 0.5],
        ],
    };
}

const fn hard(velocity: f32, strength: f32) -> DriveParams {
    DriveParams {
        spring_or_max_velocity: velocity,
        damping: 0.0,
        max_strength: strength,
        kind: DriveKind::Hard,
    }
}
const fn soft(spring: f32, damping: f32, strength: f32) -> DriveParams {
    DriveParams {
        spring_or_max_velocity: spring,
        damping,
        max_strength: strength,
        kind: DriveKind::Soft,
    }
}

/// SK8 `BoneDriveDynamics::enable` for the riding modes: 0, the animation's hard drives; 3, the soft
/// collision drive while a contact's effect lasts.
pub fn bone_dynamics(
    s: &DriveSettings,
    channel: usize,
    collision: bool,
    strength: f32,
) -> DriveDynamics {
    if collision {
        DriveDynamics {
            linear: soft(0.0, 0.0, 0.0),
            angular: soft(
                s.collision_soft_displacement * strength,
                200.0,
                s.collision_soft_strength * strength * FREQUENCY_SQUARED,
            ),
        }
    } else {
        let a = s.animation[channel];
        DriveDynamics {
            linear: hard(a[1] * FREQUENCY, a[0] * FREQUENCY_SQUARED),
            angular: hard(a[3] * FREQUENCY, a[2] * FREQUENCY_SQUARED),
        }
    }
}

/// The hips' target drive (SK8 targets.rs, target 0).
const HIPS_DRIVE: DriveDynamics = DriveDynamics {
    linear: hard(HARD_VELOCITY, HARD_STRENGTH),
    angular: hard(HARD_VELOCITY, HARD_STRENGTH),
};

/// SkeletonCollision's settings (`physics_skeleton_collision`, `physics_skeleton_feedback`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedbackSettings {
    pub compliant: [bool; ANIMATED_PARTS],
    pub effect_time: f32,
    pub small_object_mass: f32,
    pub ground_plane_max_distance: f32,
    pub ground_plane_max_angle: f32,
}

impl FeedbackSettings {
    pub const STOCK: FeedbackSettings = FeedbackSettings {
        compliant: [
            true, true, true, false, false, false, true, false, false, false, true, true, true,
            true, true, true, true, true, true, true, true, true, true, true,
        ],
        effect_time: 0.16,
        small_object_mass: 5.5,
        ground_plane_max_distance: 0.3,
        ground_plane_max_angle: 0.8,
    };
}

/// SK8 `SkeletonContactFlags` (the ones the riding reads).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContactFlags {
    /// Some bone touched something.
    pub any: bool,
    /// A foot or toe touched the board.
    pub foot_board: bool,
    /// A bone touched something other than the board.
    pub nonboard: bool,
    pub compliant: bool,
    pub noncompliant: bool,
    pub has_impulse: bool,
    /// A contact's effect is fading (the drives are soft).
    pub recovering: bool,
}

/// A bone's hardest contact this tick (SK8 `BoneContact`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoneContact {
    pub force: f32,
    pub normal: Vec3,
    pub point: Vec3,
}

/// SK8 collision_update.rs `REGIONS` (TU3 0x820CFCB0): each bone's body region.
const REGIONS: [usize; ANIMATED_PARTS] = [
    usize::MAX,
    0,
    1,
    2,
    2,
    2,
    1,
    3,
    3,
    3,
    1,
    1,
    1,
    1,
    1,
    6,
    6,
    4,
    4,
    7,
    7,
    5,
    5,
    1,
];

/// What touched a bone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Other {
    World,
    Board(usize),
}

/// One contact of a bone this tick, for the feedback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneReport {
    pub part: usize,
    pub other: Other,
    pub normal: Vec3,
    pub point: Vec3,
}

/// SkeletonCollision's feedback (SK8 collision_update.rs `update`, errors.rs): the contact flags, the
/// bones' and regions' hardest contacts, the drive weight and the pose error.
#[derive(Clone, Debug, PartialEq)]
pub struct Feedback {
    pub settings: FeedbackSettings,
    pub flags: ContactFlags,
    pub bones: [BoneContact; ANIMATED_PARTS],
    pub regions: [Option<(usize, f32)>; 8],
    pub current: [bool; ANIMATED_PARTS],
    pub timer: f32,
    /// 1 with no contact's effect left, down to 0 just after one (the drives soften with it).
    pub drive_weight: f32,
    /// Each bone's physical position less its animated one (m, world; the root's is zero).
    pub errors: [Vec3; ANIMATED_PARTS],
    /// The largest error among the hips, legs, feet and head (SK8 `maximum_error`).
    pub max_error: f32,
}

impl Feedback {
    pub fn new(settings: FeedbackSettings) -> Feedback {
        Feedback {
            settings,
            flags: ContactFlags::default(),
            bones: [BoneContact::default(); ANIMATED_PARTS],
            regions: [None; 8],
            current: [false; ANIMATED_PARTS],
            timer: 0.0,
            drive_weight: 0.0,
            errors: [Vec3::ZERO; ANIMATED_PARTS],
            max_error: 0.0,
        }
    }

    /// One tick's contacts (SK8 `SkeletonCollisionFeedback::update`, riding: not a ragdoll, on the
    /// board). `plane` is the ground the riding stands on (point, normal), `reference` the board's
    /// velocity, `velocity(part)` the bone's.
    pub fn update(
        &mut self,
        reports: &[BoneReport],
        plane: (Vec3, Vec3),
        reference: Vec3,
        board_mass: f32,
        board_velocity: Vec3,
        velocity: &dyn Fn(usize) -> Vec3,
    ) {
        self.timer -= STEP;
        self.flags = ContactFlags::default();
        self.bones = [BoneContact::default(); ANIMATED_PARTS];
        self.regions = [None; 8];
        self.current = [false; ANIMATED_PARTS];
        let speed = reference.length();
        let direction = reference.normalize_or(Vec3::ZERO);
        let s = self.settings;
        for r in reports {
            let part = r.part;
            self.flags.any = true;
            if r.other != Other::World && [15, 16, 19, 20].contains(&part) {
                self.flags.foot_board = true;
            }
            let distance = plane.1.dot(r.point - plane.0);
            if distance < s.ground_plane_max_distance
                && r.normal.dot(plane.1).abs() > s.ground_plane_max_angle
            {
                continue;
            }
            self.current[part] = true;
            let own = direction * direction.dot(velocity(part));
            let own = if own.length() > speed {
                own.normalize_or(Vec3::ZERO) * speed
            } else {
                own
            };
            let (other_velocity, other_mass) = match r.other {
                Other::World => (Vec3::ZERO, 0.0),
                Other::Board(_) => (board_velocity, board_mass),
            };
            let small = other_mass > 0.0001 && other_mass < s.small_object_mass;
            let mut force = (other_velocity - own).dot(r.normal).abs();
            if small {
                force *= 0.5;
            }
            if r.other == Other::World {
                self.flags.nonboard = true;
            }
            if part > 0 {
                let bone = &mut self.bones[part];
                if force > bone.force {
                    *bone = BoneContact {
                        force,
                        normal: r.normal,
                        point: r.point,
                    };
                }
                let region = &mut self.regions[REGIONS[part]];
                if region.is_none_or(|(_, f)| force > f) {
                    *region = Some((part, force));
                }
                self.timer = s.effect_time;
            }
            if !s.compliant[part] || small {
                self.flags.noncompliant = true;
            } else {
                self.flags.compliant = true;
            }
            self.flags.has_impulse = self.flags.compliant;
        }
        self.drive_weight = if s.effect_time == 0.0 {
            1.0
        } else {
            1.0 - (self.timer / s.effect_time).clamp(0.0, 1.0)
        };
        self.flags.recovering = self.drive_weight < 1.0;
    }

    /// SK8 `SkeletonPoseErrors::update` and `maximum_error`.
    pub fn set_errors(
        &mut self,
        physical: &[Vec3; ANIMATED_PARTS],
        target: &[Xform; ANIMATED_PARTS],
    ) {
        self.errors[0] = Vec3::ZERO;
        for p in 1..ANIMATED_PARTS {
            self.errors[p] = physical[p] - target[p].pos;
        }
        self.max_error = [23, 17, 18, 16, 21, 22, 20, 1]
            .map(|p| self.errors[p].length())
            .into_iter()
            .fold(0.0, f32::max);
    }
}

/// What the rider's body is driven to this tick.
#[derive(Clone, Copy, Debug)]
pub struct RiderInput {
    /// The animated pose: each physical body's frame (world).
    pub pose: [Xform; ANIMATED_PARTS],
    /// Off the ground: the skeleton is solved.
    pub airborne: bool,
    /// The ground the riding stands on (point, normal), for the feedback's ground filter.
    pub ground: (Vec3, Vec3),
    /// The board's volumes touch the bones: only off the board (its collision group 7; riding, its
    /// group 4 against the skater's 5 is culled by TU3's group table, SK8 assembly_contacts.rs).
    pub board_contacts: bool,
    /// A microsecond clock (for [`RiderBody::cost_us`]).
    pub clock: Option<fn() -> u32>,
}

/// A volume's spheres: a sphere's, or a capsule's ends and middle (its axis is the body's z).
fn spheres(shape: Shape) -> ([f32; 3], usize, f32) {
    match shape {
        Shape::Sphere { radius } => ([0.0; 3], 1, radius),
        Shape::Capsule {
            radius,
            half_length,
        } => ([-half_length, 0.0, half_length], 3, radius),
        Shape::Box { half_extents } => ([0.0; 3], 1, half_extents.length()),
    }
}

/// The closest points of segments `p0 p1` and `q0 q1` (Ericson 5.1.9).
fn closest_segments(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> (Vec3, Vec3) {
    let (d1, d2, r) = (p1 - p0, q1 - q0, p0 - q0);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (s, t) = if a <= 1e-9 && e <= 1e-9 {
        (0.0, 0.0)
    } else if a <= 1e-9 {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= 1e-9 {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s = if denom > 1e-12 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    (p0 + d1 * s, q0 + d2 * t)
}

/// The skeleton's contact material (`physics_skeleton_collision` normal material: no friction, no
/// restitution).
const MATERIAL: [f32; 3] = [0.0, 0.0, 0.0];
/// Contacts are looked for this far past touching (m), as the board's are.
const MARGIN: f32 = 0.02;

/// The rider's physical body.
#[derive(Clone, Debug, PartialEq)]
pub struct RiderBody {
    /// The 26 bodies (merged and kinematic ones are placed from their hosts and targets).
    pub parts: [Body; PARTS],
    pub joints: [JointRecord; JOINTS],
    pub drive_settings: DriveSettings,
    pub feedback: Feedback,
    /// The animated pose the body was last driven to.
    pub target: [Xform; ANIMATED_PARTS],
    /// The physical deck's frame after the last step (SK8 `SkeletonBoardFrames::physical_board`): the
    /// air's hook target (phase 3).
    pub physical_board: Xform,
    /// Whether the last step solved the skeleton (else it followed the animated pose).
    pub solved: bool,
    /// The step's cost (us, with the shared solve when it solved).
    pub cost_us: u32,
    /// Stepped last tick: a gap restarts it at the animated pose.
    live: bool,
    idle: u32,
    shapes: [Shape; PARTS],
    contacts: Vec<ContactRow>,
    reports: Vec<(usize, Other, usize)>,
    joint_rows: Vec<JointRow>,
    drive_rows: Vec<DriveRow>,
    bodies: Vec<Body>,
    reactions: Vec<Reaction>,
    tris: Vec<([Vec3; 3], Vec3)>,
    tri_boxes: Vec<(Vec3, Vec3)>,
    near: Vec<([Vec3; 3], Vec3)>,
    found: Vec<Candidate>,
    pairs: Vec<(usize, usize, ContactInput)>,
    bone_reports: Vec<BoneReport>,
    /// Per tick, from the animated pose: each bone's inverse frame, the rotation of a world-axes frame
    /// on it (the drives' frames), and a merged bone's frame on its host (identity for the others).
    inverse_pose: [Xform; ANIMATED_PARTS],
    drive_rotation: [Quat; ANIMATED_PARTS],
    on_host: [Xform; ANIMATED_PARTS],
}

/// A candidate contact: the bone part, its volume sphere's world centre and radius, what it meets.
type Candidate = (usize, ContactInput, Other);

impl RiderBody {
    /// The stock rider at the animated pose `pose`, at rest.
    pub fn new(pose: &[Xform; ANIMATED_PARTS]) -> RiderBody {
        let stock = stock_parts();
        let mut parts: [Body; PARTS] = core::array::from_fn(|i| {
            let at = if i < ANIMATED_PARTS {
                pose[i]
            } else {
                pose[23]
            };
            Body::new(
                stock[i].animated,
                at.pos,
                Quat::from_mat3(&at.rot),
                pb::GRAVITY,
            )
        });
        // A merged bone's mass rides on its host (its inertia scaled with the mass).
        for (child, into) in MERGED {
            let (mc, mh) = (
                1.0 / stock[child].animated.inverse_mass,
                1.0 / parts[into].mass.inverse_mass,
            );
            let k = mh / (mh + mc);
            let m = &mut parts[into].mass;
            m.inverse_mass *= k;
            m.inverse_inertia = m.inverse_inertia * k;
            let q = parts[into].orientation;
            parts[into].set_orientation(q);
        }
        let mut body = RiderBody {
            parts,
            joints: STOCK_JOINTS,
            drive_settings: DriveSettings::STOCK,
            feedback: Feedback::new(FeedbackSettings::STOCK),
            target: *pose,
            physical_board: Xform::IDENTITY,
            solved: false,
            cost_us: 0,
            live: false,
            idle: 0,
            shapes: stock.map(|p| p.shape),
            contacts: Vec::new(),
            reports: Vec::new(),
            joint_rows: Vec::new(),
            drive_rows: Vec::new(),
            bodies: Vec::new(),
            reactions: Vec::new(),
            tris: Vec::new(),
            tri_boxes: Vec::new(),
            near: Vec::new(),
            found: Vec::with_capacity(64),
            pairs: Vec::with_capacity(SELF_PAIRS.len()),
            bone_reports: Vec::with_capacity(64),
            inverse_pose: [Xform::IDENTITY; ANIMATED_PARTS],
            drive_rotation: [Quat::IDENTITY; ANIMATED_PARTS],
            on_host: [Xform::IDENTITY; ANIMATED_PARTS],
        };
        // The solve's rows, reserved so that a first solved tick doesn't grow them.
        body.contacts.reserve(64);
        body.reports.reserve(64);
        body.joint_rows.reserve(32);
        body.drive_rows.reserve(64);
        body.bodies.reserve(PARTS + 8);
        body.reactions.reserve(PARTS + 8);
        body.set_target(pose);
        body
    }

    /// Drives the body to `pose` this tick (and the per-tick frames read from it).
    fn set_target(&mut self, pose: &[Xform; ANIMATED_PARTS]) {
        self.target = *pose;
        for (p, at) in pose.iter().enumerate() {
            let inv = inverse(at);
            self.drive_rotation[p] = Quat::from_mat3(&inv.rot.mul(&Mat3::IDENTITY)).normalize();
            self.inverse_pose[p] = inv;
        }
        for (child, into) in MERGED {
            self.on_host[child] = self.inverse_pose[into].then(&pose[child]);
        }
    }

    /// SK8 `bone_drive_frames` for `part` driven from `parent`'s body: a frame at the part's animated
    /// origin, in the world's axes, expressed on each.
    fn drive_frames(&self, part: usize, parent: usize) -> (DriveFrame, DriveFrame) {
        let at = self.target[part].pos;
        (
            DriveFrame {
                orientation: self.drive_rotation[part],
                translation: self.inverse_pose[part].point(at),
            },
            DriveFrame {
                orientation: self.drive_rotation[parent],
                translation: self.inverse_pose[parent].point(at),
            },
        )
    }

    /// The contact rows of the last solved step (the board's and the skeleton's; 0 when it followed).
    pub fn contact_rows(&self) -> usize {
        if self.solved { self.contacts.len() } else { 0 }
    }

    /// Each animated bone's physical frame now (merged bones at their animated place on their host).
    pub fn pose(&self) -> [Xform; ANIMATED_PARTS] {
        core::array::from_fn(|p| self.frame(p))
    }

    fn body_frame(&self, p: usize) -> Xform {
        let b = &self.parts[p];
        Xform::new(b.axes, b.position)
    }

    fn frame(&self, p: usize) -> Xform {
        let h = host(p);
        if h == p {
            return self.body_frame(p);
        }
        self.body_frame(h).then(&self.on_host[p])
    }

    /// Puts every body at the animated pose, at rest (SK8 `reset_to_animation`).
    pub fn reset(&mut self, pose: &[Xform; ANIMATED_PARTS]) {
        for (i, b) in self.parts.iter_mut().enumerate() {
            let at = pose[i.min(ANIMATED_PARTS - 1)];
            b.set_orientation(Quat::from_mat3(&at.rot));
            b.position = at.pos;
            b.velocity = Vec3::ZERO;
            b.spin = Vec3::ZERO;
            b.force = pb::GRAVITY;
            b.torque = Vec3::ZERO;
        }
        self.set_target(pose);
        self.live = true;
    }

    /// A tick passes: a body not stepped in it restarts at the animated pose next time.
    pub fn tick(&mut self) {
        self.idle += 1;
    }

    /// Moves a following body to `to` with the velocity that gets it there in a tick.
    fn follow(b: &mut Body, to: &Xform) {
        let q = Quat::from_mat3(&to.rot);
        let d = q.compose(b.orientation.conjugate());
        let d = if d.w < 0.0 {
            Quat {
                x: -d.x,
                y: -d.y,
                z: -d.z,
                w: -d.w,
            }
        } else {
            d
        };
        b.spin = d.vector() * (2.0 / STEP);
        b.velocity = (to.pos - b.position) * (1.0 / STEP);
        b.position = to.pos;
        b.set_orientation(q);
        b.force = pb::GRAVITY;
        b.torque = Vec3::ZERO;
    }

    /// The bones' contact candidates with the world, the board and each other this tick.
    fn candidates(
        &mut self,
        park: &dyn Terrain,
        board: &[Body],
        board_contacts: bool,
        out: &mut Vec<Candidate>,
        pairs: &mut Vec<(usize, usize, ContactInput)>,
    ) {
        let hips = self.parts[23].position;
        let reach = 1.4 + self.parts[23].velocity.length() * STEP;
        self.tris.clear();
        let has_tris = park.triangles_near(hips, reach, &mut self.tris);
        // Each triangle's box, for a bone's first cut. The edge fix-up's neighbours come along: one
        // across an edge or corner a sphere touches holds it, within the sphere's reach (the 1 cm of
        // slack covers the fix-up's 1 mm weld; the drop-in and four University rides are bit for bit
        // the same as with 0.3 m).
        self.tri_boxes.clear();
        for (t, _) in &self.tris {
            let lo = v3(
                t[0].x.min(t[1].x).min(t[2].x),
                t[0].y.min(t[1].y).min(t[2].y),
                t[0].z.min(t[1].z).min(t[2].z),
            );
            let hi = v3(
                t[0].x.max(t[1].x).max(t[2].x),
                t[0].y.max(t[1].y).max(t[2].y),
                t[0].z.max(t[1].z).max(t[2].z),
            );
            self.tri_boxes.push((lo, hi));
        }
        let floor = combine(MATERIAL, pb::FLOOR);
        for (part, bone) in BONES.iter().enumerate().skip(1) {
            if !bone.has_collision {
                continue;
            }
            let f = self.frame(part);
            let (offsets, n, radius) = spheres(self.shapes[part]);
            let v = self.parts[host(part)].velocity;
            if has_tris {
                let (e0, e1) = (
                    f.point(v3(0.0, 0.0, offsets[0])),
                    f.point(v3(0.0, 0.0, offsets[n - 1])),
                );
                let slack = radius + MARGIN + v.length() * STEP + 0.01;
                let lo =
                    v3(e0.x.min(e1.x), e0.y.min(e1.y), e0.z.min(e1.z)) - v3(slack, slack, slack);
                let hi =
                    v3(e0.x.max(e1.x), e0.y.max(e1.y), e0.z.max(e1.z)) + v3(slack, slack, slack);
                self.near.clear();
                for (t, (bl, bh)) in self.tris.iter().zip(&self.tri_boxes) {
                    if bl.x <= hi.x
                        && bh.x >= lo.x
                        && bl.y <= hi.y
                        && bh.y >= lo.y
                        && bl.z <= hi.z
                        && bh.z >= lo.z
                    {
                        self.near.push(*t);
                    }
                }
            }
            for &z in &offsets[..n] {
                let c = f.point(v3(0.0, 0.0, z));
                if has_tris {
                    let r = radius + MARGIN + v.length() * STEP;
                    for ci in pb::sphere_triangles(
                        &self.near,
                        c,
                        c - v * STEP,
                        radius,
                        r,
                        Vec3::UP,
                        floor,
                        -1.0,
                    )
                    .into_iter()
                    .flatten()
                    {
                        out.push((part, ci, Other::World));
                    }
                } else {
                    let g = park.ground(c.x, c.z, c.y + radius);
                    let gap = c.y - g.height;
                    if g.height > f32::MIN / 2.0 && gap < radius + MARGIN && gap > -radius {
                        let n = g.normal;
                        out.push((
                            part,
                            ContactInput {
                                point_a: c - n * radius,
                                point_b: v3(c.x, g.height, c.z),
                                normal: n,
                                restitution: floor[2],
                                static_friction: floor[0],
                                dynamic_friction: floor[1],
                            },
                            Other::World,
                        ));
                    }
                }
                // The board's volumes as spheres (its deck and trucks).
                for (bp, at, br) in pb::board_spheres(board).filter(|_| board_contacts) {
                    let d = c - at;
                    let l = d.length();
                    if l < radius + br + MARGIN && l > 1e-6 {
                        let n = d * (1.0 / l);
                        let m = combine(MATERIAL, pb::DECK_MATERIAL);
                        out.push((
                            part,
                            ContactInput {
                                point_a: c - n * radius,
                                point_b: at + n * br,
                                normal: n,
                                restitution: m[2],
                                static_friction: m[0],
                                dynamic_friction: m[1],
                            },
                            Other::Board(bp),
                        ));
                    }
                }
            }
        }
        // The forearms against the trunk and thighs.
        for (a, b) in SELF_PAIRS {
            let seg = |p: usize| {
                let f = self.frame(p);
                let (o, n, r) = spheres(self.shapes[p]);
                (
                    f.point(v3(0.0, 0.0, o[0])),
                    f.point(v3(0.0, 0.0, o[n - 1])),
                    r,
                )
            };
            let ((a0, a1, ra), (b0, b1, rb)) = (seg(a), seg(b));
            let (pa, pb_) = closest_segments(a0, a1, b0, b1);
            let d = pa - pb_;
            let l = d.length();
            if l < ra + rb + MARGIN && l > 1e-6 {
                let n = d * (1.0 / l);
                pairs.push((
                    a,
                    b,
                    ContactInput {
                        point_a: pa - n * ra,
                        point_b: pb_ + n * rb,
                        normal: n,
                        restitution: 0.0,
                        static_friction: 0.0,
                        dynamic_friction: 0.0,
                    },
                ));
            }
        }
    }

    /// Reduction 2: a contact the animated pose can't stand in for (anything but the ground under the
    /// riding, or the board).
    fn needs_solve(c: &[Candidate], ground: (Vec3, Vec3), s: &FeedbackSettings) -> bool {
        c.iter().any(|(_, ci, other)| match other {
            Other::Board(_) => (ci.point_a - ci.point_b).dot(ci.normal) < 0.0,
            Other::World => {
                let distance = ground.1.dot(ci.point_b - ground.0);
                !(distance < s.ground_plane_max_distance
                    && ci.normal.dot(ground.1).abs() > s.ground_plane_max_angle)
            }
        })
    }

    /// One tick with the board (SK8 `advance_attached`): the drives to `input`'s pose, the contacts,
    /// and, when the skeleton is solved, its rows appended to the board's in one solve. `board` is the
    /// board's bodies (and its hook), its rows are solved here with the skeleton's and `reactions` gets
    /// the board's share. Returns whether it solved (else the caller solves the board alone).
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        park: &dyn Terrain,
        board: &[Body],
        contacts: &mut [ContactRow],
        joints: &mut [JointRow],
        drives: &mut [DriveRow],
        reactions: &mut [Reaction],
        input: &RiderInput,
    ) -> bool {
        let t0 = input.clock.map_or(0, |f| f());
        let solved = self.step_inner(park, board, contacts, joints, drives, reactions, input);
        self.cost_us = input.clock.map_or(0, |f| f()).wrapping_sub(t0);
        solved
    }

    #[allow(clippy::too_many_arguments)]
    fn step_inner(
        &mut self,
        park: &dyn Terrain,
        board: &[Body],
        contacts: &mut [ContactRow],
        joints: &mut [JointRow],
        drives: &mut [DriveRow],
        reactions: &mut [Reaction],
        input: &RiderInput,
    ) -> bool {
        let pose = &input.pose;
        // A tick without a step, or a jump of the hips target (a respawn, a kinematic move), restarts
        // the body at the animated pose (SK8 `update_positions`' continuity check).
        if !self.live || self.idle > 1 || (pose[23].pos - self.target[23].pos).length() > 1.0 {
            self.reset(pose);
        }
        self.idle = 0;
        self.set_target(pose);
        self.physical_board = Xform::new(board[pb::DECK].axes, board[pb::DECK].position);

        let mut found = core::mem::take(&mut self.found);
        let mut pairs = core::mem::take(&mut self.pairs);
        found.clear();
        pairs.clear();
        self.candidates(park, board, input.board_contacts, &mut found, &mut pairs);
        let solve =
            input.airborne || Self::needs_solve(&found, input.ground, &self.feedback.settings);
        self.solved = solve;
        if !solve {
            for p in 0..PARTS {
                let to = match p {
                    0 => self.physical_board,
                    _ if p >= ANIMATED_PARTS => pose[23],
                    _ => pose[p],
                };
                Self::follow(&mut self.parts[p], &to);
            }
            self.reports.clear();
            for (i, (part, _, other)) in found.iter().enumerate() {
                self.reports.push((*part, *other, i));
            }
            self.finish(&found, board, input);
            self.found = found;
            self.pairs = pairs;
            return false;
        }

        // Kinematic bodies follow their targets.
        let deck_frame = self.physical_board;
        Self::follow(&mut self.parts[0], &deck_frame);
        for p in [24, 25] {
            Self::follow(&mut self.parts[p], &pose[23]);
        }
        // SK8 `apply_gravity_displacement`: half the gravity's tick displacement as a force.
        for &p in &SOLVED_PARTS {
            let b = &mut self.parts[p];
            b.force.y += -0.5 * 9.8 * STEP * b.mass.inverse_mass;
        }

        // Bodies: the board's, the solved bones, the hips' target.
        let base = board.len();
        let index = |p: usize| base + SOLVED_SLOT[p];
        let target_index = base + SOLVED;
        self.bodies.clear();
        self.bodies.extend_from_slice(board);
        for &p in &SOLVED_PARTS {
            self.bodies.push(self.parts[p]);
        }
        let mut hips = Body::new(
            self.parts[23].mass,
            pose[23].pos,
            Quat::from_mat3(&pose[23].rot),
            Vec3::ZERO,
        );
        hips.active = false;
        self.bodies.push(hips);

        // Contacts: the board's, then the skeleton's with the world and board, then its own pairs.
        self.contacts.clear();
        self.contacts.extend_from_slice(contacts);
        self.reports.clear();
        for (part, ci, other) in &found {
            let b = match other {
                Other::World => None,
                Other::Board(bp) => Some(*bp),
            };
            self.reports.push((*part, *other, self.contacts.len()));
            self.contacts
                .push(ContactRow::build(&self.bodies, Some(index(*part)), b, ci));
        }
        for (a, b, ci) in &pairs {
            self.contacts.push(ContactRow::build(
                &self.bodies,
                Some(index(*a)),
                Some(index(*b)),
                ci,
            ));
        }

        // Joints: the board's, then the bones' (merged ones re-expressed on their hosts).
        self.joint_rows.clear();
        self.joint_rows.extend_from_slice(joints);
        for j in &self.joints {
            let (hc, hp) = (host(j.child), host(j.parent));
            if hc == hp {
                continue;
            }
            let mut f = j.frames;
            if hc != j.child {
                let rc = &self.on_host[j.child];
                f.orientation_a = Quat::from_mat3(&rc.rot).compose(f.orientation_a);
                f.anchor_a = rc.point(f.anchor_a);
            }
            if hp != j.parent {
                let rp = &self.on_host[j.parent];
                let q = Quat::from_mat3(&rp.rot);
                f.orientation_b = q.compose(f.orientation_b);
                f.anchor_b = rp.point(f.anchor_b);
                f.linear_orientation_b = q.compose(f.linear_orientation_b);
            }
            self.joint_rows.push(JointRow::build(
                &self.bodies,
                index(j.child),
                index(j.parent),
                &j.params,
                &f,
            ));
        }

        // Drives: the board's, the hips' target, then LOCAL and LOCAL_ROOT for each solved bone (SK8
        // `SkeletonDrives::update` and `build`).
        self.drive_rows.clear();
        self.drive_rows.extend_from_slice(drives);
        let id = DriveFrame {
            orientation: Quat::IDENTITY,
            translation: Vec3::ZERO,
        };
        self.drive_rows.push(DriveRow::build(
            &self.bodies,
            index(23),
            target_index,
            &id,
            &id,
            &HIPS_DRIVE,
        ));
        let s = self.drive_settings;
        let base_strength = if s.enabled { s.strength } else { [0.0; 2] };
        let weight = self.feedback.drive_weight;
        for &part in &SOLVED_PARTS {
            if part == 23 {
                continue;
            }
            let collision = weight < 1.0;
            for (channel, parent) in [PARENTS[part].unwrap(), 23].into_iter().enumerate() {
                if host(parent) == part {
                    continue;
                }
                let strength = if collision {
                    let c = s.collision_strength[part][channel];
                    ((1.0 - c) * weight + c) * s.strength[channel]
                } else {
                    base_strength[channel]
                };
                let d = bone_dynamics(&s, channel, collision, strength);
                let (fa, fb) = self.drive_frames(part, host(parent));
                self.drive_rows.push(DriveRow::build(
                    &self.bodies,
                    index(part),
                    index(parent),
                    &fa,
                    &fb,
                    &d,
                ));
            }
        }

        self.reactions.clear();
        self.reactions
            .resize(self.bodies.len(), Reaction::default());
        super::solve(
            &mut self.contacts,
            &mut self.joint_rows,
            &mut self.drive_rows,
            &mut self.reactions,
            pb::ITERATIONS,
        );
        contacts.copy_from_slice(&self.contacts[..contacts.len()]);
        reactions[..base].copy_from_slice(&self.reactions[..base]);
        for (k, &p) in SOLVED_PARTS.iter().enumerate() {
            let mut b = self.bodies[base + k];
            b.integrate(&self.reactions[base + k], pb::GRAVITY);
            self.parts[p] = b;
        }
        for (child, into) in MERGED {
            let to = self.body_frame(into).then(&self.on_host[child]);
            Self::follow(&mut self.parts[child], &to);
        }
        self.finish(&found, board, input);
        self.found = found;
        self.pairs = pairs;
        true
    }

    /// The feedback and pose error after the step.
    fn finish(&mut self, found: &[Candidate], board: &[Body], input: &RiderInput) {
        let mut reports = core::mem::take(&mut self.bone_reports);
        reports.clear();
        reports.extend(found.iter().map(|(part, ci, other)| BoneReport {
            part: *part,
            other: *other,
            normal: ci.normal,
            point: ci.point_b,
        }));
        let board_mass = pb::total_mass();
        let deck = &board[pb::DECK];
        let parts = &self.parts;
        let velocity = |p: usize| parts[host(p)].velocity;
        self.feedback.update(
            &reports,
            input.ground,
            deck.velocity,
            board_mass,
            deck.velocity,
            &velocity,
        );
        let physical: [Vec3; ANIMATED_PARTS] = core::array::from_fn(|p| self.frame(p).pos);
        let target = self.target;
        self.feedback.set_errors(&physical, &target);
        self.bone_reports = reports;
        self.live = true;
    }
}

/// The animated pose of the physical bodies from the rider's bone frames (`bones`: each physical bone's
/// animation bone, in the rider's own space, axes then origin), placed in the world by `to_world`.
/// `mirrored`: the rider is drawn reflected across its x = 0 plane, so each body takes its partner's
/// bone, reflected, turned by the fixed turn that makes it the body's own frame in the rig's initial
/// pose (the rig is symmetric there).
pub fn pose_from_bones(
    bones: &[Xform; ANIMATED_PARTS],
    to_world: &Xform,
    mirrored: bool,
) -> [Xform; ANIMATED_PARTS] {
    core::array::from_fn(|p| {
        let local = if mirrored {
            let q = PARTNER[p];
            let reflect = |x: &Xform| {
                let m = |v: Vec3| v3(-v.x, v.y, v.z);
                Xform::new(
                    Mat3::from_cols(m(x.rot.cols[0]), m(x.rot.cols[1]), m(x.rot.cols[2])),
                    m(x.pos),
                )
            };
            // M P_q(t) R0 with R0 = P_q(0)^-1 M P_p(0): the reflection is undone in the rotation.
            let now = reflect(&bones[q].then(&PHYS_BONES[q].volume()));
            let rest_q = INITIAL_BONES[q].then(&PHYS_BONES[q].volume());
            let rest_p = INITIAL_BONES[p].then(&PHYS_BONES[p].volume());
            let r0 = inverse(&rest_q).then(&reflect(&rest_p));
            // `now` and `r0` are both improper: their product is a rotation.
            Xform::new(now.rot.mul(&r0.rot), now.pos)
        } else {
            bones[p].then(&PHYS_BONES[p].volume())
        };
        to_world.then(&local)
    })
}

#[cfg(test)]
mod tests;
