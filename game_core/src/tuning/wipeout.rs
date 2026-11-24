//! The wipeout's body: Skate 3's `physics_wipeout` (the ragdoll's drag, its drives' fade, the weights'
//! steps, the board's material), `physics_skeleton` (the ragdoll's friction and restitution),
//! `physics_skeleton_joints` (the ragdoll's joint-limit scalars) and the five `physics_wipeout_control`
//! profiles the wipeout gesture picks (free fall, cannon ball, judo kick, swan dive, torpedo; SK8
//! `skate-game/src/physics/wipeout_states/settings.rs` `load_profile`, `ragdoll/settings.rs`). The
//! built-in values are Skate 3's stock ones (read off the disc's `skatercollections` with
//! `fg_import values`); the rules pack carries the same paths.

use super::Curve;

/// One `physics_wipeout_control` profile (SK8 `wipeout_state::profiles::Profile`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    /// The body's axis rolled about on the ground, the axis held level in the air, and the Euler
    /// angles (degrees, x then y) that tip the body's up to the axis aligned with the travel.
    pub roll_axis: [f32; 3],
    pub horizontal_axis: [f32; 3],
    pub align_euler: [f32; 2],
    /// The spin's build-up against the time the gesture has been held (s).
    pub spin_vs_time: Curve,
    /// How fast the profile's forward follows the travel (share a tick).
    pub direction_follow: f32,
    /// Gains: towards the spin asked for (`TorqueVelScalar`) and towards the travel
    /// (`TorqueDistScalar`), and how long a let-go spin input is kept (`SpinInertia`).
    pub torque_velocity: f32,
    pub torque_distance: f32,
    pub spin_inertia: f32,
    /// Rolling along the ground with the stick (`RollOnGround`, `RollingOnGroundTorqueVelScalar`).
    pub roll_on_ground: bool,
    pub ground_roll_torque: f32,
    /// Spin (rad/s) at a full stick across and along the travel.
    pub sideways_spin: f32,
    pub forward_spin: f32,
    /// The level axis keeps its sign (else either way round).
    pub horizontal_directed: bool,
    /// Steering the fall: the most speed it adds along the stick, and the push across and along.
    pub drift_max_speed: f32,
    pub drift_forward: f32,
    pub drift_sideways: f32,
    /// Lines the body up with the travel in the air, and on the ground (`AlignWithVel`, and its
    /// ground flag with `AlignOnGroundTorqueVelScalar`).
    pub align_with_velocity: bool,
    pub align_ground_with_velocity: bool,
    pub ground_align_torque: f32,
    /// The stick tips the aim this far (degrees), and the drift's drag.
    pub tilt_degrees: f32,
    pub drift_drag: f32,
}

/// The five profiles' collection names, in the order the gesture picks them (centre, up, right,
/// down, left) and Skate 3's `GestureType` names.
pub const PROFILE_NAMES: [&str; 5] = [
    "free_fall",
    "cannon_ball",
    "judo_kick",
    "swan_dive",
    "torpedo",
];
pub const GESTURE_NAMES: [&str; 5] = ["Freefall", "CannonBall", "JudoKick", "SwanDive", "Torpedo"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WipeoutTuning {
    pub profiles: [Profile; 5],
    /// `physics_wipeout`: the ragdoll's drag (per second), the target's and the drives' fade time
    /// (s), the controlled and collision weights' steps a tick.
    pub linear_drag: f32,
    pub angular_drag: f32,
    pub remove_target_time: f32,
    pub remove_drives_time: f32,
    pub controlled_step: f32,
    pub collision_step: f32,
    /// The free board's friction and restitution in a wipeout.
    pub board_friction: f32,
    pub board_restitution: f32,
    /// `physics_skeleton`: the ragdoll's friction and restitution, the head's.
    pub friction: f32,
    pub restitution: f32,
    pub head_friction: f32,
    pub head_restitution: f32,
    /// `physics_skeleton_joints`: the ragdoll's joint limits against the animated body's.
    pub swing_scalar: f32,
    pub twist_scalar: f32,
}

const fn profile(
    roll_axis: [f32; 3],
    horizontal_axis: [f32; 3],
    align_euler: [f32; 2],
    spin_vs_time: Curve,
    numbers: [f32; 13],
    flags: [bool; 4],
) -> Profile {
    let [
        direction_follow,
        torque_velocity,
        torque_distance,
        spin_inertia,
        ground_roll_torque,
        sideways_spin,
        forward_spin,
        drift_max_speed,
        drift_forward,
        drift_sideways,
        ground_align_torque,
        tilt_degrees,
        drift_drag,
    ] = numbers;
    let [
        roll_on_ground,
        horizontal_directed,
        align_with_velocity,
        align_ground_with_velocity,
    ] = flags;
    Profile {
        roll_axis,
        horizontal_axis,
        align_euler,
        spin_vs_time,
        direction_follow,
        torque_velocity,
        torque_distance,
        spin_inertia,
        roll_on_ground,
        ground_roll_torque,
        sideways_spin,
        forward_spin,
        horizontal_directed,
        drift_max_speed,
        drift_forward,
        drift_sideways,
        align_with_velocity,
        align_ground_with_velocity,
        ground_align_torque,
        tilt_degrees,
        drift_drag,
    }
}

/// `SpinVsTime` of free fall, judo kick and swan dive (the same on the disc).
const SPIN_RAMP: Curve = Curve::new(&[
    (0.0, 1.0),
    (0.141_693_8, 1.0),
    (0.306_188_9, 1.0),
    (0.402_280_1, 1.0),
    (0.540_716_6, 1.0),
    (0.651_465_8, 1.0),
    (0.770_358_3, 1.0),
    (1.0, 1.0),
]);

impl WipeoutTuning {
    /// Skate 3's stock values.
    pub const BUILT_IN: WipeoutTuning = WipeoutTuning {
        profiles: [
            // follow, torque vel, torque dist, inertia, roll torque, side spin, fwd spin, drift max,
            // drift z, drift x, ground align, tilt, drift drag.
            profile(
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [0.0, 0.0],
                SPIN_RAMP,
                [
                    0.01, 0.25, -0.35, 0.0, 0.0, 2.0, -3.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                ],
                [false, false, false, false],
            ),
            profile(
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 0.0],
                Curve::new(&[
                    (0.0, 0.271_428_6),
                    (0.097_719_87, 0.457_142_9),
                    (0.258_957_7, 0.614_285_7),
                    (0.385_993_5, 0.717_857_1),
                    (0.513_029_3, 0.796_428_6),
                    (0.638_436_5, 0.864_285_7),
                    (0.781_759, 0.932_142_9),
                    (1.0, 1.0),
                ]),
                [
                    0.1, 0.25, -0.35, 0.99, 0.5, 0.0, -6.5, 6.0, 3.0, 3.0, 0.5, 40.0, 0.2,
                ],
                [true, false, false, true],
            ),
            profile(
                [0.0; 3],
                [0.0, 0.0, -1.0],
                [170.0, -90.0],
                SPIN_RAMP,
                [
                    0.05, 0.25, -0.25, 0.0, 0.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.15,
                ],
                [false, true, true, false],
            ),
            profile(
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [50.0, 0.0],
                SPIN_RAMP,
                [
                    0.05, 0.25, -0.35, 0.0, 0.0, 0.0, 0.0, 9.0, 4.0, 4.0, 0.0, 45.0, 1.0,
                ],
                [false, false, true, false],
            ),
            profile(
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 0.0],
                Curve::new(&[
                    (0.0, 0.167_142_9),
                    (0.117_263_8, 0.3),
                    (0.299_674_3, 0.458_571_5),
                    (0.605_863_2, 0.634_285_7),
                    (1.469_055, 1.105_714),
                    (1.609_12, 1.157_143),
                    (1.752_443, 1.187_143),
                    (2.0, 1.2),
                ]),
                [
                    0.1, 0.25, -0.35, 0.0, 0.5, -4.0, 0.0, 5.0, 0.0, 0.0, 0.5, 0.0, 0.0,
                ],
                [true, false, true, true],
            ),
        ],
        linear_drag: 0.0,
        angular_drag: 0.5,
        remove_target_time: 0.0,
        remove_drives_time: 1.5,
        controlled_step: 0.0075,
        collision_step: 0.1,
        board_friction: 0.36,
        board_restitution: 0.0,
        friction: 0.9,
        restitution: 0.0,
        head_friction: 0.9,
        head_restitution: 0.0,
        swing_scalar: 0.5,
        twist_scalar: 0.6,
    };

    /// Takes over a value from the rules pack (`class/key/field`). Returns whether it was one of these.
    pub(super) fn set(&mut self, name: &str, f: Option<f32>, c: Option<Curve>, v: &[f32]) -> bool {
        macro_rules! scalar {
            ($field:expr) => {
                match f {
                    Some(f) => {
                        $field = f;
                        true
                    }
                    None => false,
                }
            };
        }
        macro_rules! flag {
            ($field:expr) => {
                match f {
                    Some(f) => {
                        $field = f != 0.0;
                        true
                    }
                    None => false,
                }
            };
        }
        let axis = |out: &mut [f32; 3]| -> bool {
            if v.len() >= 3 && v[..3].iter().all(|x| x.is_finite()) {
                *out = [v[0], v[1], v[2]];
                true
            } else {
                false
            }
        };
        if let Some(rest) = name.strip_prefix("physics_wipeout_control/")
            && let Some((key, field)) = rest.split_once('/')
            && let Some(i) = PROFILE_NAMES.iter().position(|n| *n == key)
        {
            let p = &mut self.profiles[i];
            return match field {
                "RollOnGroundAxis" => axis(&mut p.roll_axis),
                "HorizAlignAxis" => axis(&mut p.horizontal_axis),
                "AlignAxisEuler" => {
                    if v.len() >= 2 && v[..2].iter().all(|x| x.is_finite()) {
                        p.align_euler = [v[0], v[1]];
                        true
                    } else {
                        false
                    }
                }
                "SpinVsTime" => match c {
                    Some(c) => {
                        p.spin_vs_time = c;
                        true
                    }
                    None => false,
                },
                "Hash_1BC908CDB3520A8E" => scalar!(p.direction_follow),
                "TorqueVelScalar" => scalar!(p.torque_velocity),
                "TorqueDistScalar" => scalar!(p.torque_distance),
                "SpinInertia" => scalar!(p.spin_inertia),
                "RollOnGround" => flag!(p.roll_on_ground),
                "RollingOnGroundTorqueVelScalar" => scalar!(p.ground_roll_torque),
                "Hash_40B2B3C1A87A4D76" => scalar!(p.sideways_spin),
                "Hash_9047531918285FC3" => scalar!(p.forward_spin),
                "Hash_6009FD75F9EDC436" => flag!(p.horizontal_directed),
                "DriftMaxSpeed" => scalar!(p.drift_max_speed),
                "DriftFactorZ" => scalar!(p.drift_forward),
                "DriftFactorX" => scalar!(p.drift_sideways),
                "AlignWithVel" => flag!(p.align_with_velocity),
                "Hash_9626703A9939FE36" => flag!(p.align_ground_with_velocity),
                "AlignOnGroundTorqueVelScalar" => scalar!(p.ground_align_torque),
                "Hash_B65276A8CC7FB760" => scalar!(p.tilt_degrees),
                "Hash_7E6B3A99C0A33ABC" => scalar!(p.drift_drag),
                _ => false,
            };
        }
        match name {
            "physics_wipeout/default/LinearDrag" => scalar!(self.linear_drag),
            "physics_wipeout/default/AngularDrag" => scalar!(self.angular_drag),
            "physics_wipeout/default/TimeToRemoveHook" => scalar!(self.remove_target_time),
            "physics_wipeout/default/TimeToRemoveDrives" => scalar!(self.remove_drives_time),
            "physics_wipeout/default/DynamicDriveWeightVelControlled" => {
                scalar!(self.controlled_step)
            }
            "physics_wipeout/default/DynamicDriveWeightVelCollision" => {
                scalar!(self.collision_step)
            }
            "physics_wipeout/default/SkateboardFriction" => scalar!(self.board_friction),
            "physics_wipeout/default/SkateboardRestitution" => scalar!(self.board_restitution),
            "physics_skeleton/default/FrictionRagdoll" => scalar!(self.friction),
            "physics_skeleton/default/RestitutionRagdoll" => scalar!(self.restitution),
            "physics_skeleton/default/FrictionRagdollHead" => scalar!(self.head_friction),
            "physics_skeleton/default/RestitutionRagdollHead" => scalar!(self.head_restitution),
            "physics_skeleton_joints/default/SwingRagdollScalar" => scalar!(self.swing_scalar),
            "physics_skeleton_joints/default/TwistRagdollScalar" => scalar!(self.twist_scalar),
            _ => false,
        }
    }
}

/// One profile's paths (`physics_wipeout_control/<profile>/<field>`).
macro_rules! profile_fields {
    ($p:literal) => {
        [
            concat!("physics_wipeout_control/", $p, "/RollOnGroundAxis"),
            concat!("physics_wipeout_control/", $p, "/HorizAlignAxis"),
            concat!("physics_wipeout_control/", $p, "/AlignAxisEuler"),
            concat!("physics_wipeout_control/", $p, "/SpinVsTime"),
            concat!("physics_wipeout_control/", $p, "/Hash_1BC908CDB3520A8E"),
            concat!("physics_wipeout_control/", $p, "/TorqueVelScalar"),
            concat!("physics_wipeout_control/", $p, "/TorqueDistScalar"),
            concat!("physics_wipeout_control/", $p, "/SpinInertia"),
            concat!("physics_wipeout_control/", $p, "/RollOnGround"),
            concat!(
                "physics_wipeout_control/",
                $p,
                "/RollingOnGroundTorqueVelScalar"
            ),
            concat!("physics_wipeout_control/", $p, "/Hash_40B2B3C1A87A4D76"),
            concat!("physics_wipeout_control/", $p, "/Hash_9047531918285FC3"),
            concat!("physics_wipeout_control/", $p, "/Hash_6009FD75F9EDC436"),
            concat!("physics_wipeout_control/", $p, "/DriftMaxSpeed"),
            concat!("physics_wipeout_control/", $p, "/DriftFactorZ"),
            concat!("physics_wipeout_control/", $p, "/DriftFactorX"),
            concat!("physics_wipeout_control/", $p, "/AlignWithVel"),
            concat!("physics_wipeout_control/", $p, "/Hash_9626703A9939FE36"),
            concat!(
                "physics_wipeout_control/",
                $p,
                "/AlignOnGroundTorqueVelScalar"
            ),
            concat!("physics_wipeout_control/", $p, "/Hash_B65276A8CC7FB760"),
            concat!("physics_wipeout_control/", $p, "/Hash_7E6B3A99C0A33ABC"),
        ]
    };
}

/// Every profile's paths.
pub const PROFILE_PATHS: [[&str; 21]; 5] = [
    profile_fields!("free_fall"),
    profile_fields!("cannon_ball"),
    profile_fields!("judo_kick"),
    profile_fields!("swan_dive"),
    profile_fields!("torpedo"),
];

/// Where each value lives in Skate 3's tuning database (`class/key/field`).
pub const PATHS: &[&str] = &[
    "physics_wipeout/default/LinearDrag",
    "physics_wipeout/default/AngularDrag",
    "physics_wipeout/default/TimeToRemoveHook",
    "physics_wipeout/default/TimeToRemoveDrives",
    "physics_wipeout/default/DynamicDriveWeightVelControlled",
    "physics_wipeout/default/DynamicDriveWeightVelCollision",
    "physics_wipeout/default/SkateboardFriction",
    "physics_wipeout/default/SkateboardRestitution",
    "physics_skeleton/default/FrictionRagdoll",
    "physics_skeleton/default/RestitutionRagdoll",
    "physics_skeleton/default/FrictionRagdollHead",
    "physics_skeleton/default/RestitutionRagdollHead",
    "physics_skeleton_joints/default/SwingRagdollScalar",
    "physics_skeleton_joints/default/TwistRagdollScalar",
];
