//! Skating tuning: every number the skating feel depends on, in one place.
//!
//! The built-in values are flatground's own. A rules pack converted from the player's disc
//! (`fg_import rules`) carries Skate 3's stock values for the same quantities, under their attribute
//! paths (`class/key/field`, as the game's tuning database names them), and [`Tuning::apply`] takes them
//! over. Nothing from the disc is compiled in.
//!
//! `rules/tuning` entry: `u32` count, then per value a `u16` name length, the name (ASCII), a `u16` float
//! count and that many `f32`. A curve is its x values followed by its y values. All big-endian.

use alloc::vec::Vec;

/// Most points a curve can hold (Skate 3's largest point graphs have 16).
pub const CURVE_POINTS: usize = 16;

/// A piecewise-linear curve, flat beyond its ends (how Skate 3's point graphs evaluate).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    n: u8,
    x: [f32; CURVE_POINTS],
    y: [f32; CURVE_POINTS],
}

impl Curve {
    /// From `(x, y)` points in increasing x (at most [`CURVE_POINTS`]; extra points are dropped).
    pub const fn new(points: &[(f32, f32)]) -> Curve {
        let mut c = Curve {
            n: 0,
            x: [0.0; CURVE_POINTS],
            y: [0.0; CURVE_POINTS],
        };
        let mut i = 0;
        while i < points.len() && i < CURVE_POINTS {
            c.x[i] = points[i].0;
            c.y[i] = points[i].1;
            i += 1;
        }
        c.n = i as u8;
        c
    }

    pub const fn constant(v: f32) -> Curve {
        Curve::new(&[(0.0, v)])
    }

    /// From a tuning value: x values then y values.
    fn from_values(v: &[f32]) -> Option<Curve> {
        let n = v.len() / 2;
        if n == 0 || !v.len().is_multiple_of(2) || n > CURVE_POINTS {
            return None;
        }
        let mut c = Curve::constant(0.0);
        c.n = n as u8;
        c.x[..n].copy_from_slice(&v[..n]);
        c.y[..n].copy_from_slice(&v[n..]);
        Some(c)
    }

    pub fn eval(&self, input: f32) -> f32 {
        let n = self.n as usize;
        if n == 0 {
            return 0.0;
        }
        if input < self.x[0] || input.is_nan() {
            return self.y[0];
        }
        for i in 1..n {
            if input < self.x[i] {
                let w = self.x[i] - self.x[i - 1];
                if w <= 0.0 {
                    return self.y[i];
                }
                return self.y[i - 1] + (self.y[i] - self.y[i - 1]) * (input - self.x[i - 1]) / w;
            }
        }
        self.y[n - 1]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    // Gravity and slopes.
    pub gravity: f32,
    /// Most the slope can speed the board up or slow it down (m/s^2).
    pub max_slope_accel: f32,

    // Pushing.
    /// No push takes the board faster than this (m/s).
    pub push_max_speed: f32,
    /// Most speed one tick of foot contact can add, at a standstill and at `push_max_speed` (m/s).
    pub push_tick_limit_slow: f32,
    pub push_tick_limit_fast: f32,
    /// Seconds of holding the button that give a full-strength push, against speed.
    pub push_hold_time: Curve,
    /// Speed a push adds (m/s) against how much of `push_hold_time` the button was held (0..1).
    pub push_dv: Curve,
    /// Latest point of a push's recovery (as a share of it, 0..1) where the next push may start, and how
    /// much that point follows speed rather than push strength (0..1). Skate 3's re-push rule.
    pub push_out_max: f32,
    pub push_out_speed_weight: f32,
    /// Most speed a held, repeated push adds (m/s).
    pub push_hold_cap: f32,
    /// How many frames (1/60 s) the push trees' start-speed blends take to cross their whole range,
    /// against the strength blend; and the strength blend's, against speed.
    pub push_blend_speed_frames: Curve,
    pub push_blend_acc_frames: Curve,
    /// A first push this soon after the rider was put on the board pushes as if held (s).
    pub push_teleport_window: f32,
    /// No pushing or foot braking on slopes steeper than this (degrees).
    pub push_slope_limit: f32,

    // Rolling resistance and braking.
    /// Deceleration (m/s^2) against speed, always.
    pub friction: Curve,
    /// Extra deceleration against speed once nothing has been pressed for `no_input_time`.
    pub friction_no_input: Curve,
    pub no_input_time: f32,
    /// Extra deceleration against speed on two wheels.
    pub friction_manual: Curve,
    /// Foot brake and tail brake strength (deceleration = force * `brake_scale` * `braking_scalar`).
    pub foot_brake: f32,
    pub tail_brake: f32,
    /// flatground's own calibration from Skate 3's brake forces to a deceleration (m/s^2 per unit).
    pub brake_scale: f32,
    /// The surface's braking factor (smooth concrete).
    pub braking_scalar: f32,
    /// Below this speed (m/s) a braking board is held still.
    pub brake_slow_speed: f32,
    /// The rolling-resistance corrector moves the speed by at most `friction_max_change *
    /// speed_general_amount` per tick.
    pub friction_max_change: f32,
    pub speed_general_amount: f32,

    // Steering.
    /// How quickly steering follows the stick: 0 never, 1 at once (per 1/60 s).
    pub steer_follow: f32,
    pub steer_scalar: f32,
    /// Deck tilt against stick deflection (0..1).
    pub steer_vs_input: Curve,
    /// Deck tilt scale against speed / `steer_speed_range`.
    pub steer_vs_speed: Curve,
    pub steer_speed_range: f32,
    pub hard_turn_boost: f32,
    /// Steering is weaker while pushing: falls by `push_steer_drop` per tick while a push is on, recovers
    /// by `push_steer_rise`, never below `push_steer_min`.
    pub push_steer_drop: f32,
    pub push_steer_rise: f32,
    pub push_steer_min: f32,
    /// How quickly the deck tilt follows the steering (per 1/60 s).
    pub steer_tilt_blend: f32,
    /// Steering scale on two wheels.
    pub manual_tilt: f32,
    /// Deck pitch while dragging the tail or nose to brake (degrees).
    pub manual_brake_tilt: f32,
    /// Pop strength finalisation (state graphs): at or below `jump_clamp_low` a pop is the minimum hop,
    /// at or above `jump_clamp_high` full height.
    pub jump_clamp_low: f32,
    pub jump_clamp_high: f32,
    /// Sideways wheel friction (damping of sideways speed), and the torque that turns the board back
    /// in line with its travel (per tick per m/s of sideways error), unless the stick turns harder than
    /// `straighten_max_turn`.
    pub side_friction: f32,
    pub straighten: f32,
    pub straighten_max_turn: f32,
    /// Downhill heading torque on slopes: strength, and its scale against slope (0 flat .. 1 vertical)
    /// and against speed / `heading_adjust_max_speed`.
    pub heading_adjust: f32,
    pub heading_adjust_max_speed: f32,
    pub heading_vs_slope: Curve,
    pub heading_vs_speed: Curve,
    /// Extra stick turn on steep faces: strength, against slope, against speed / 8 m/s, and against the
    /// yaw rate already opposing it (rad per tick).
    pub turn_torque: f32,
    pub turn_torque_vs_slope: Curve,
    pub turn_torque_vs_speed: Curve,
    pub turn_torque_vs_neg_ang_vel: Curve,

    // Powerslides.
    /// Least speed to start one (and below which it ends), the least slide amount at first, how long a
    /// slide lasts at least (s), and the trucks' turn during it.
    pub slide_speed_threshold: f32,
    pub slide_min_entry: f32,
    pub well_into_slide_time: f32,
    pub slide_turn_value: f32,
    /// Where a held slide relaxes to (against the stick's slide amount), and how much it relaxes against
    /// speed.
    pub slide_input_remap: Curve,
    pub slide_remap_vs_speed: Curve,
    /// "Not sliding": `slide_speed_factor(speed) * (1 - |cos angle|)` under `not_sliding_threshold(cos
    /// angle)` for longer than `not_sliding_time(cos angle)` ends the slide.
    pub slide_speed_factor: Curve,
    pub not_sliding_threshold: Curve,
    pub not_sliding_time: Curve,

    // Ground contact and walls.
    /// Leaving the ground: the surface further than `natural_air_max_dist` (m) below the board, or more
    /// than `natural_air_min_dist` for `natural_air_time` (s).
    pub natural_air_max_dist: f32,
    pub natural_air_min_dist: f32,
    pub natural_air_time: f32,
    /// How quickly the board's drawn up follows the ground under it (per tick).
    pub ground_normal_blend: f32,
    /// Hitting a wall: the speed it pushes the board away at (m/s), the most it may change the velocity,
    /// and the yaw that turns the board along it (rad per tick, against the angle / pi between travel and
    /// the wall's normal) for `collision_torque_time` s, fading by `collision_torque_fade` per tick.
    pub wall_push_away: f32,
    pub wall_max_vel_delta: f32,
    pub collision_torque: Curve,
    pub collision_torque_time: f32,
    pub collision_torque_fade: f32,
    /// Closing speed into a wall (m/s) that takes the rider off: on the ground and in the air.
    pub wipe_ground_closing: f32,
    pub wipe_air_closing: f32,

    // Vert, wallrides, pumping.
    /// Popping off a steep face: the vertical re-aim (ground normal y below `vert_align_max_ny` and a
    /// launch direction y above `vert_align_min_dir_y`), its lean range (rad) and blend, and the share
    /// of the off-the-face speed kept when rolling off vert without a pop.
    pub vert_align_max_ny: f32,
    pub vert_align_min_dir_y: f32,
    pub vert_align_max_angle: f32,
    pub vert_align_factor: f32,
    pub natural_air_off_verts: f32,
    /// Wallrides: gravity held off (share of g) against seconds on the wall; the most the floor may face
    /// the wall (dot); the height above the floor (m) and look-ahead (s) that end it; the longest ride;
    /// and the jump off it (share of the falling speed kept, push from the wall in m/s).
    pub wallride_antigravity: Curve,
    pub wallride_max_dot_floor: f32,
    pub wallride_jump_height: f32,
    pub wallride_lookahead: f32,
    pub wallride_max_time: f32,
    pub wallride_jump_ydown: f32,
    pub wallride_jump_force: f32,
    /// Pumping: the rider's extension (m per tick) smoothed by `pump_damping`, ignored under
    /// `pump_min_change`, capped at `pump_max_change`, fading with `pump_vs_time`; times the ground's
    /// turn rate along travel (smoothed by `pump_ang_damping`) times `pump_vs_vel(speed)` times
    /// `pump_factor`; at most `pump_max_accel` m/s^2, scaled by `pump_unintentional` unless on purpose.
    pub pump_damping: f32,
    pub pump_min_change: f32,
    pub pump_max_change: f32,
    pub pump_vs_time: Curve,
    pub pump_ang_damping: f32,
    pub pump_vs_vel: Curve,
    pub pump_factor: f32,
    pub pump_max_accel: f32,
    pub pump_unintentional: f32,

    // Jumping.
    /// Centre-of-mass heights (m): the lowest any jump reaches, and the range a flick's strength picks
    /// from (scaled by the curves against speed / `jump_speed_range`).
    pub jump_floor: f32,
    pub jump_low: f32,
    pub jump_high: f32,
    pub jump_low_vs_speed: Curve,
    pub jump_high_vs_speed: Curve,
    pub jump_speed_range: f32,
    pub hippy_low: f32,
    pub hippy_high: f32,
    /// Hippie jump strength (0..1) against seconds wound up.
    pub hippy_windup: Curve,
    /// Pop strength along the surface normal against the slope (0 flat .. 1 vertical), never under
    /// `jump_min_scalar`.
    pub vertical_response: Curve,
    pub jump_min_scalar: f32,
    /// Extra upward speed on near-vertical faces (scale against the normal's y), at most
    /// `jump_y_bonus_max` m/s.
    pub jump_y_vs_normal_y: Curve,
    pub jump_y_bonus_max: f32,
    /// The left stick nudges a pop sideways and forwards (share of the pop speed).
    pub jump_adjust_x: f32,
    pub jump_adjust_z: f32,

    // Air spin.
    /// Spin speed (rad/s) per unit of stick against time in the air.
    pub spin_speed: Curve,
    /// Most the spin speed may change per tick against time in the air (rad/s).
    pub spin_change: Curve,
    /// The same, when the stick asks for the other direction.
    pub spin_reverse_change: f32,
    /// How much a quick wind-up counts against its time relative to take-off (s, negative before).
    pub spin_windup: Curve,
    /// Spin speed without any wind-up, as a share of the full speed.
    pub spin_no_windup: f32,

    // Landing.
    /// Largest board-to-travel angle (rad) that still lands, against speed.
    pub land_angle: Curve,
    /// Hitting the ground faster than this (m/s, downwards) is a bail.
    pub land_max_impact: f32,

    // Flick input.
    /// Samples a flick may stray from its shape before it is dropped.
    pub flick_misses: u8,

    // Camera (riding slow, riding fast, grinding): distance (m), elevation (deg), heading (deg, 180 =
    // straight behind), framing pitch (deg), look-at height above the board (m).
    pub cam_slow: CamShot,
    pub cam_fast: CamShot,
    pub cam_grind: CamShot,
    /// Riding speed (m/s) at which the camera has fully changed from the slow shot to the fast one.
    pub cam_fast_speed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamShot {
    pub distance: f32,
    pub elevation: f32,
    pub heading: f32,
    pub pitch: f32,
    pub board_offset: f32,
}

impl CamShot {
    pub fn lerp(&self, o: &CamShot, k: f32) -> CamShot {
        let l = |a: f32, b: f32| a + (b - a) * k;
        // Headings take the short way round (-160 to 180 is 20 degrees, not 340).
        let mut dh = (o.heading - self.heading) % 360.0;
        if dh > 180.0 {
            dh -= 360.0;
        } else if dh < -180.0 {
            dh += 360.0;
        }
        CamShot {
            distance: l(self.distance, o.distance),
            elevation: l(self.elevation, o.elevation),
            heading: self.heading + dh * k,
            pitch: l(self.pitch, o.pitch),
            board_offset: l(self.board_offset, o.board_offset),
        }
    }
}

impl Default for Tuning {
    fn default() -> Self {
        Self::BUILT_IN
    }
}

/// Where each value lives in Skate 3's tuning database (`class/key/field`). `fg_import rules` exports
/// exactly these paths.
pub const PATHS: &[&str] = &[
    "physics_speed_conservation/default/Gravity",
    "physics_speed_conservation/default/MaxGravityAcceleration",
    "physics_push/default/MaxPushableSpeed",
    "physics_mode/normal/MaxPushDVStart",
    "physics_mode/normal/MaxPushDVEnd",
    "anim_motion/pushing/button_time_max",
    "anim_motion/pushing/button_time_to_dv",
    "anim_motion/pushing/last_push_out_time",
    "anim_motion/pushing/dynamic_out_factor_speed_vs_acc",
    "anim_motion/pushing/max_holding_acc",
    "anim_motion/pushing/blend_speed_over_frames",
    "anim_motion/pushing/blend_acc_over_frames",
    "anim_motion/pushing/pushing_usemaxpushfromteleporttime",
    "anim_motion/pushing/disable_push_brake_at_slope",
    "physics_surfaces/smooth/FrictionVsSpeedNew",
    "physics_friction/default/FrictionVsSpeed_NoInput",
    "physics_friction/default/NoInputTime",
    "physics_friction/default/FrictionVsSpeed_Manual",
    "physics_brakes/default/FootBrakeForce",
    "physics_brakes/default/TailBrakeForce",
    "physics_brakes/default/SlowSpeed",
    "physics_surfaces/smooth/BrakingScalar",
    "physics_surfaces/smooth/Friction_MaxSpeedChange",
    "physics_speed_conservation/default/GeneralAmount",
    "physics_surfaces/smooth/SideFriction_Scalar",
    "physics_surfaces/smooth/StraightenOut_Scalar",
    "physics_heading/default/StraightenMaxTurnInput",
    "physics_heading/default/HeadingAdjustFactor",
    "physics_heading/default/HeadingAdjustMaxSpeed",
    "physics_heading/default/HeadingAdjustVsSlope",
    "physics_heading/default/HeadingAdjustVsSpeed",
    "physics_heading/default/TurnTorqueScalar",
    "physics_heading/default/TurnTorqueVsSlope",
    "physics_heading/default/TurnTorqueVsSpeed",
    "physics_heading/default/TurnTorqueVsNegAngVel",
    "physics_steering/default/SteeringTiltBlending",
    "physics_manual/default/Tilt_TiltScalar",
    "physics_manual/default/BrakeTiltAngle",
    "anim_motion/power_slide/slide_speed_threshold",
    "anim_motion/power_slide/slide_min_entry",
    "anim_motion/power_slide/well_into_slide_time",
    "anim_motion/power_slide/slide_turn_value",
    "anim_motion/power_slide/slide_speed_factor",
    "anim_motion/power_slide/not_sliding_threshold",
    "anim_motion/power_slide/not_sliding_time_thresh",
    "physics_slide/default/slide_input_remap",
    // Slide relaxation against speed (the database has no name for this field).
    "physics_slide/default/Hash_DE162591FD9D91D7",
    "physics_airstates/default/NaturalAirMaxDist",
    "physics_airstates/default/NaturalAirMinDist",
    "physics_airstates/default/NaturalAirTime",
    "physics_reckoning/default/GroundNormalSmoothing",
    "physics_reckoning/default/VertJumpAlignMaxGroundNormalY",
    "physics_reckoning/default/VertJumpAlignMinJumpDirY",
    "physics_reckoning/default/VertJumpAlignMaxAngle",
    "physics_reckoning/default/VertJumpAlignFactor",
    "physics_trajectory/default/NaturalAirOffVertsScalar",
    "physics_collision/default/TargetDisplacementVel",
    "physics_collision/default/MaxVelDelta",
    "physics_collision/default/CollisionTorqueVsAngle",
    "physics_collision/default/CollisionTorqueMaxTime",
    "physics_collision/default/CollisionTorqueFadeoff",
    "physics_mode/normal/Wipeout_GroundXZAcceleration",
    "physics_wipeout/default/Wipeout_AirXZAcceleration",
    "physics_feet/default/WallRideAntiGravityVsTime",
    "physics_feet/default/WallRideMaxDotFloorWall",
    "physics_feet/default/WallRideAutoJumpHeight",
    "physics_feet/default/WallRideVelTimeToConsider",
    "physics_feet/default/WallRideMaxTime",
    "physics_feet/default/WallRideAutoJumpYDownScalar",
    "physics_feet/default/WallRideAutoJumpForce",
    "physics_pumping/default/PumpEffectDamping",
    "physics_pumping/default/MinChangeInCOMBeforePumping",
    "physics_pumping/default/MaxDeltaHeightAllowedPerFrame",
    "physics_pumping/default/PumpVsTime",
    "physics_pumping/default/AngularSpeedDamping",
    "physics_pumping/default/PumpVsVel",
    "physics_mode/normal/PumpEffectFactor",
    // Most pumping acceleration per second (no name in the database).
    "physics_mode/normal/Hash_D77AFD320B6241C5",
    "physics_mode/normal/UnintentionalPumpScalar",
    "physics_steering/default/Damping",
    "physics_steering/default/GeneralScalar",
    "physics_steering/default/SteeringVsInput",
    "physics_steering/default/SteeringVsSpeed",
    "physics_steering/default/SteeringVsSpeedMaxSpeed",
    "physics_steering/default/HardTurnIncrease",
    "physics_steering/default/PushScalarDecriment",
    "physics_steering/default/PushScalarIncriment",
    "physics_steering/default/PushScalarMin",
    "physics_jump/default/AbsoluteMinHeight",
    "anim_motion/jumping/clamp_low_inclusive",
    "anim_motion/jumping/clamp_high_inclusive",
    "physics_mode/normal/JumpMinHeight",
    "physics_mode/normal/JumpMaxHeight",
    "physics_jump/default/MinHeightVsSpeed",
    "physics_jump/default/MaxHeightVsSpeed",
    "physics_jump/default/SpeedResponseMaxSpeed",
    "physics_jump/default/HippyJumpMinHeight",
    "physics_jump/default/HippyJumpMaxHeight",
    "anim_motion/hippy_flip/antic_length_to_hippy_height",
    "physics_jump/default/VerticalResponse",
    "physics_jump/default/MinScalar",
    "physics_jump/default/JumpYScalarVsGroundNormalY",
    "physics_jump/default/JumpYBonusMax",
    "physics_jump/default/JumpAdjustXFactor",
    "physics_jump/default/JumpAdjustZFactor",
    "physics_bodyspin/default/PropBodySpinVsTime",
    "physics_bodyspin/default/MaxDeltaVsTime",
    "physics_bodyspin/default/MaxDeltaOppositeDirection",
    "physics_bodyspin/default/DerivativeBodySpinVsTime",
    "physics_bodyspin/default/MinDerivativeScalar",
    // Landing angle limit against speed (the database has no name for this field).
    "physics_wipeout/default/Hash_4F08D9BAE6831524",
    "physics_wipeout/default/Wipeout_AirMaxSpeedIntoGround",
    "recognizer/right_stick/NumTicksPatternNotInRangeBeforeCulling",
    "camera_shots/chase_flat_slow/PositionDistance",
    "camera_shots/chase_flat_slow/PositionElevation",
    "camera_shots/chase_flat_slow/PositionHeading",
    "camera_shots/chase_flat_slow/FramingPitch",
    "camera_shots/chase_flat_slow/ReferenceBoardOffset",
    "camera_shots/chase_flat_fast/PositionDistance",
    "camera_shots/chase_flat_fast/PositionElevation",
    "camera_shots/chase_flat_fast/PositionHeading",
    "camera_shots/chase_flat_fast/FramingPitch",
    "camera_shots/chase_flat_fast/ReferenceBoardOffset",
    "camera_shots/grind_flat_slow/PositionDistance",
    "camera_shots/grind_flat_slow/PositionElevation",
    "camera_shots/grind_flat_slow/PositionHeading",
    "camera_shots/grind_flat_slow/FramingPitch",
    "camera_shots/grind_flat_slow/ReferenceBoardOffset",
];

impl Tuning {
    /// flatground's own values, used when no rules pack is present.
    pub const BUILT_IN: Tuning = Tuning {
        gravity: 9.8,
        max_slope_accel: 6.5,
        push_max_speed: 8.0,
        push_tick_limit_slow: 0.5,
        push_tick_limit_fast: 0.4,
        push_hold_time: Curve::constant(0.3),
        push_dv: Curve::new(&[(0.0, 1.0), (1.0, 4.0)]),
        push_out_max: 0.6,
        push_out_speed_weight: 0.5,
        push_hold_cap: 3.5,
        push_blend_speed_frames: Curve::constant(6.0),
        push_blend_acc_frames: Curve::constant(6.0),
        push_teleport_window: 0.5,
        push_slope_limit: 40.0,
        friction: Curve::new(&[(0.0, 0.0), (8.0, 0.05), (30.0, 1.0)]),
        friction_no_input: Curve::new(&[(0.0, 0.1), (8.0, 0.45)]),
        no_input_time: 2.5,
        friction_manual: Curve::new(&[(0.0, 0.1), (8.0, 0.45)]),
        foot_brake: 45.0,
        tail_brake: 32.0,
        brake_scale: 0.06,
        braking_scalar: 1.0,
        brake_slow_speed: 0.02,
        friction_max_change: 0.4,
        speed_general_amount: 0.2,
        steer_follow: 0.75,
        steer_scalar: 0.6,
        steer_vs_input: Curve::new(&[(0.0, 0.0), (0.2, 0.02), (1.0, 0.8)]),
        steer_vs_speed: Curve::new(&[(0.0, 1.0), (0.1, 0.9), (0.5, 0.35), (1.0, 0.3)]),
        steer_speed_range: 45.0,
        hard_turn_boost: 1.0,
        push_steer_drop: 0.02,
        push_steer_rise: 0.05,
        push_steer_min: 0.5,
        steer_tilt_blend: 0.25,
        manual_tilt: 0.5,
        manual_brake_tilt: 25.0,
        jump_clamp_low: 0.5,
        jump_clamp_high: 0.9,
        side_friction: 60.0,
        straighten: 0.02,
        straighten_max_turn: 0.75,
        heading_adjust: 0.025,
        heading_adjust_max_speed: 7.5,
        heading_vs_slope: Curve::new(&[(0.0, 0.0), (0.06, 0.0), (0.2, 0.8), (0.3, 1.0)]),
        heading_vs_speed: Curve::new(&[(0.0, 1.0), (0.4, 0.75), (0.7, 0.25), (1.0, 0.0)]),
        turn_torque: 0.06,
        turn_torque_vs_slope: Curve::new(&[
            (0.0, 0.0),
            (0.3, 0.0),
            (0.45, 0.15),
            (0.65, 0.6),
            (0.8, 1.0),
        ]),
        turn_torque_vs_speed: Curve::new(&[(0.0, 1.0), (0.5, 0.8), (1.0, 0.1)]),
        turn_torque_vs_neg_ang_vel: Curve::new(&[
            (0.0, 1.0),
            (0.05, 0.3),
            (0.1, 0.05),
            (0.5, 0.03),
        ]),
        slide_speed_threshold: 1.2,
        slide_min_entry: 0.25,
        well_into_slide_time: 0.25,
        slide_turn_value: 0.35,
        slide_input_remap: Curve::new(&[(0.0, 0.0), (0.5, 0.12), (0.75, 0.35), (1.0, 1.0)]),
        slide_remap_vs_speed: Curve::new(&[(0.0, 0.0), (3.0, 0.0), (7.0, 0.8), (9.0, 1.0)]),
        slide_speed_factor: Curve::new(&[(0.0, 0.0), (1.2, 1.0)]),
        not_sliding_threshold: Curve::new(&[(-1.0, 0.5), (0.0, 0.04), (1.0, 0.04)]),
        not_sliding_time: Curve::constant(0.12),
        natural_air_max_dist: 0.1,
        natural_air_min_dist: 0.05,
        natural_air_time: 0.25,
        ground_normal_blend: 0.3,
        wall_push_away: 0.8,
        wall_max_vel_delta: 10.0,
        collision_torque: Curve::new(&[
            (0.0, 0.0),
            (0.3, 0.0),
            (0.5, 0.03),
            (0.7, 0.05),
            (0.8, 0.05),
            (0.85, 0.0),
        ]),
        collision_torque_time: 0.4,
        collision_torque_fade: 0.92,
        wipe_ground_closing: 7.5,
        wipe_air_closing: 3.5,
        vert_align_max_ny: 0.25,
        vert_align_min_dir_y: 0.75,
        vert_align_max_angle: 0.1,
        vert_align_factor: 1.0,
        natural_air_off_verts: 0.5,
        wallride_antigravity: Curve::new(&[(0.0, 0.12), (0.05, 0.1), (0.12, 0.0)]),
        wallride_max_dot_floor: 0.45,
        wallride_jump_height: 0.5,
        wallride_lookahead: 0.3,
        wallride_max_time: 1.2,
        wallride_jump_ydown: 0.6,
        wallride_jump_force: 2.5,
        pump_damping: 0.15,
        pump_min_change: 0.003,
        pump_max_change: 0.008,
        pump_vs_time: Curve::new(&[(0.0, 1.0), (0.4, 1.0), (0.7, 0.4), (1.0, 0.1)]),
        pump_ang_damping: 0.25,
        pump_vs_vel: Curve::new(&[(0.0, 3.0), (3.0, 1.8), (6.0, 1.0), (10.0, 0.7), (13.0, 0.1)]),
        pump_factor: 15.0,
        pump_max_accel: 8.0,
        pump_unintentional: 0.6,
        jump_floor: 0.9,
        jump_low: 1.3,
        jump_high: 1.7,
        jump_low_vs_speed: Curve::constant(1.0),
        jump_high_vs_speed: Curve::new(&[(0.0, 1.0), (0.5, 0.8), (1.0, 0.5)]),
        jump_speed_range: 25.0,
        hippy_low: 1.3,
        hippy_high: 1.6,
        hippy_windup: Curve::new(&[(0.0, 0.0), (0.25, 0.2), (0.5, 1.0)]),
        vertical_response: Curve::new(&[
            (0.0, 1.0),
            (0.5, 1.0),
            (0.7, 0.5),
            (0.85, 0.15),
            (1.0, 0.08),
        ]),
        jump_min_scalar: 0.15,
        jump_y_vs_normal_y: Curve::new(&[(0.0, 1.4), (0.1, 1.3), (0.2, 1.05), (0.25, 1.0)]),
        jump_y_bonus_max: 1.2,
        jump_adjust_x: 0.15,
        jump_adjust_z: 0.15,
        spin_speed: Curve::new(&[(0.0, 3.0), (0.4, 7.5), (1.0, 6.0), (2.0, 3.5)]),
        spin_change: Curve::new(&[(0.0, 1.0), (0.4, 0.5), (1.0, 0.15)]),
        spin_reverse_change: 0.2,
        spin_windup: Curve::new(&[(-0.5, 0.0), (-0.3, 1.0), (0.1, 1.0), (0.5, 0.3)]),
        spin_no_windup: 0.45,
        land_angle: Curve::new(&[(0.0, 1.5), (10.0, 1.4), (28.0, 0.3)]),
        land_max_impact: 11.0,
        flick_misses: 8,
        cam_slow: CamShot {
            distance: 2.0,
            elevation: 8.0,
            heading: -165.0,
            pitch: -4.0,
            board_offset: 0.85,
        },
        cam_fast: CamShot {
            distance: 2.6,
            elevation: 6.0,
            heading: -165.0,
            pitch: -2.0,
            board_offset: 0.85,
        },
        cam_grind: CamShot {
            distance: 1.7,
            elevation: 8.0,
            heading: -165.0,
            pitch: -8.0,
            board_offset: 0.65,
        },
        cam_fast_speed: 8.0,
    };

    /// Takes over every value a `rules/tuning` entry carries. Unknown names and malformed values are
    /// skipped (the built-in value stays). Returns how many values were used.
    pub fn apply(&mut self, data: &[u8]) -> usize {
        let mut used = 0;
        for (name, v) in parse(data).unwrap_or_default() {
            if self.set(name, &v) {
                used += 1;
            }
        }
        used
    }

    fn set(&mut self, name: &str, v: &[f32]) -> bool {
        let f = v.first().copied().filter(|f| f.is_finite());
        let c =
            || Curve::from_values(v).filter(|c| c.y[..c.n as usize].iter().all(|y| y.is_finite()));
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
        macro_rules! curve {
            ($field:expr) => {
                match c() {
                    Some(c) => {
                        $field = c;
                        true
                    }
                    None => false,
                }
            };
        }
        let field = name.rsplit('/').next().unwrap_or("");
        let shot = name.split('/').nth(1).unwrap_or("");
        if name.starts_with("camera_shots/") {
            let s = match shot {
                "chase_flat_slow" => &mut self.cam_slow,
                "chase_flat_fast" => &mut self.cam_fast,
                "grind_flat_slow" => &mut self.cam_grind,
                _ => return false,
            };
            return match field {
                "PositionDistance" => scalar!(s.distance),
                "PositionElevation" => scalar!(s.elevation),
                "PositionHeading" => scalar!(s.heading),
                "FramingPitch" => scalar!(s.pitch),
                "ReferenceBoardOffset" => scalar!(s.board_offset),
                _ => false,
            };
        }
        match name {
            "physics_speed_conservation/default/Gravity" => scalar!(self.gravity),
            "physics_speed_conservation/default/MaxGravityAcceleration" => {
                scalar!(self.max_slope_accel)
            }
            "physics_push/default/MaxPushableSpeed" => scalar!(self.push_max_speed),
            "physics_mode/normal/MaxPushDVStart" => scalar!(self.push_tick_limit_slow),
            "physics_mode/normal/MaxPushDVEnd" => scalar!(self.push_tick_limit_fast),
            "anim_motion/pushing/button_time_max" => curve!(self.push_hold_time),
            "anim_motion/pushing/button_time_to_dv" => curve!(self.push_dv),
            "anim_motion/pushing/last_push_out_time" => scalar!(self.push_out_max),
            "anim_motion/pushing/dynamic_out_factor_speed_vs_acc" => {
                scalar!(self.push_out_speed_weight)
            }
            "anim_motion/pushing/max_holding_acc" => scalar!(self.push_hold_cap),
            "anim_motion/pushing/blend_speed_over_frames" => curve!(self.push_blend_speed_frames),
            "anim_motion/pushing/blend_acc_over_frames" => curve!(self.push_blend_acc_frames),
            "anim_motion/pushing/pushing_usemaxpushfromteleporttime" => {
                scalar!(self.push_teleport_window)
            }
            "anim_motion/pushing/disable_push_brake_at_slope" => scalar!(self.push_slope_limit),
            "physics_brakes/default/SlowSpeed" => scalar!(self.brake_slow_speed),
            "physics_surfaces/smooth/BrakingScalar" => scalar!(self.braking_scalar),
            "physics_surfaces/smooth/Friction_MaxSpeedChange" => scalar!(self.friction_max_change),
            "physics_speed_conservation/default/GeneralAmount" => {
                scalar!(self.speed_general_amount)
            }
            "physics_surfaces/smooth/SideFriction_Scalar" => scalar!(self.side_friction),
            "physics_surfaces/smooth/StraightenOut_Scalar" => scalar!(self.straighten),
            "physics_heading/default/StraightenMaxTurnInput" => scalar!(self.straighten_max_turn),
            "physics_heading/default/HeadingAdjustFactor" => scalar!(self.heading_adjust),
            "physics_heading/default/HeadingAdjustMaxSpeed" => {
                scalar!(self.heading_adjust_max_speed)
            }
            "physics_heading/default/HeadingAdjustVsSlope" => curve!(self.heading_vs_slope),
            "physics_heading/default/HeadingAdjustVsSpeed" => curve!(self.heading_vs_speed),
            "physics_heading/default/TurnTorqueScalar" => scalar!(self.turn_torque),
            "physics_heading/default/TurnTorqueVsSlope" => curve!(self.turn_torque_vs_slope),
            "physics_heading/default/TurnTorqueVsSpeed" => curve!(self.turn_torque_vs_speed),
            "physics_heading/default/TurnTorqueVsNegAngVel" => {
                curve!(self.turn_torque_vs_neg_ang_vel)
            }
            "physics_steering/default/SteeringTiltBlending" => scalar!(self.steer_tilt_blend),
            "physics_manual/default/Tilt_TiltScalar" => scalar!(self.manual_tilt),
            "physics_manual/default/BrakeTiltAngle" => scalar!(self.manual_brake_tilt),
            "anim_motion/jumping/clamp_low_inclusive" => scalar!(self.jump_clamp_low),
            "anim_motion/jumping/clamp_high_inclusive" => scalar!(self.jump_clamp_high),
            "anim_motion/power_slide/slide_speed_threshold" => scalar!(self.slide_speed_threshold),
            "anim_motion/power_slide/slide_min_entry" => scalar!(self.slide_min_entry),
            "anim_motion/power_slide/well_into_slide_time" => scalar!(self.well_into_slide_time),
            "anim_motion/power_slide/slide_turn_value" => scalar!(self.slide_turn_value),
            "anim_motion/power_slide/slide_speed_factor" => curve!(self.slide_speed_factor),
            "anim_motion/power_slide/not_sliding_threshold" => curve!(self.not_sliding_threshold),
            "anim_motion/power_slide/not_sliding_time_thresh" => curve!(self.not_sliding_time),
            "physics_slide/default/slide_input_remap" => curve!(self.slide_input_remap),
            "physics_slide/default/Hash_DE162591FD9D91D7" => curve!(self.slide_remap_vs_speed),
            "physics_airstates/default/NaturalAirMaxDist" => scalar!(self.natural_air_max_dist),
            "physics_airstates/default/NaturalAirMinDist" => scalar!(self.natural_air_min_dist),
            "physics_airstates/default/NaturalAirTime" => scalar!(self.natural_air_time),
            "physics_reckoning/default/GroundNormalSmoothing" => scalar!(self.ground_normal_blend),
            "physics_reckoning/default/VertJumpAlignMaxGroundNormalY" => {
                scalar!(self.vert_align_max_ny)
            }
            "physics_reckoning/default/VertJumpAlignMinJumpDirY" => {
                scalar!(self.vert_align_min_dir_y)
            }
            "physics_reckoning/default/VertJumpAlignMaxAngle" => scalar!(self.vert_align_max_angle),
            "physics_reckoning/default/VertJumpAlignFactor" => scalar!(self.vert_align_factor),
            "physics_trajectory/default/NaturalAirOffVertsScalar" => {
                scalar!(self.natural_air_off_verts)
            }
            "physics_collision/default/TargetDisplacementVel" => scalar!(self.wall_push_away),
            "physics_collision/default/MaxVelDelta" => scalar!(self.wall_max_vel_delta),
            "physics_collision/default/CollisionTorqueVsAngle" => curve!(self.collision_torque),
            "physics_collision/default/CollisionTorqueMaxTime" => {
                scalar!(self.collision_torque_time)
            }
            "physics_collision/default/CollisionTorqueFadeoff" => {
                scalar!(self.collision_torque_fade)
            }
            "physics_mode/normal/Wipeout_GroundXZAcceleration" => {
                scalar!(self.wipe_ground_closing)
            }
            "physics_wipeout/default/Wipeout_AirXZAcceleration" => scalar!(self.wipe_air_closing),
            "physics_feet/default/WallRideAntiGravityVsTime" => curve!(self.wallride_antigravity),
            "physics_feet/default/WallRideMaxDotFloorWall" => {
                scalar!(self.wallride_max_dot_floor)
            }
            "physics_feet/default/WallRideAutoJumpHeight" => scalar!(self.wallride_jump_height),
            "physics_feet/default/WallRideVelTimeToConsider" => scalar!(self.wallride_lookahead),
            "physics_feet/default/WallRideMaxTime" => scalar!(self.wallride_max_time),
            "physics_feet/default/WallRideAutoJumpYDownScalar" => {
                scalar!(self.wallride_jump_ydown)
            }
            "physics_feet/default/WallRideAutoJumpForce" => scalar!(self.wallride_jump_force),
            "physics_pumping/default/PumpEffectDamping" => scalar!(self.pump_damping),
            "physics_pumping/default/MinChangeInCOMBeforePumping" => {
                scalar!(self.pump_min_change)
            }
            "physics_pumping/default/MaxDeltaHeightAllowedPerFrame" => {
                scalar!(self.pump_max_change)
            }
            "physics_pumping/default/PumpVsTime" => curve!(self.pump_vs_time),
            "physics_pumping/default/AngularSpeedDamping" => scalar!(self.pump_ang_damping),
            "physics_pumping/default/PumpVsVel" => curve!(self.pump_vs_vel),
            "physics_mode/normal/PumpEffectFactor" => scalar!(self.pump_factor),
            "physics_mode/normal/Hash_D77AFD320B6241C5" => scalar!(self.pump_max_accel),
            "physics_mode/normal/UnintentionalPumpScalar" => scalar!(self.pump_unintentional),
            "anim_motion/hippy_flip/antic_length_to_hippy_height" => curve!(self.hippy_windup),
            "physics_jump/default/VerticalResponse" => curve!(self.vertical_response),
            "physics_jump/default/MinScalar" => scalar!(self.jump_min_scalar),
            "physics_jump/default/JumpYScalarVsGroundNormalY" => curve!(self.jump_y_vs_normal_y),
            "physics_jump/default/JumpYBonusMax" => scalar!(self.jump_y_bonus_max),
            "physics_jump/default/JumpAdjustXFactor" => scalar!(self.jump_adjust_x),
            "physics_jump/default/JumpAdjustZFactor" => scalar!(self.jump_adjust_z),
            "physics_surfaces/smooth/FrictionVsSpeedNew" => curve!(self.friction),
            "physics_friction/default/FrictionVsSpeed_NoInput" => curve!(self.friction_no_input),
            "physics_friction/default/NoInputTime" => scalar!(self.no_input_time),
            "physics_friction/default/FrictionVsSpeed_Manual" => curve!(self.friction_manual),
            "physics_brakes/default/FootBrakeForce" => scalar!(self.foot_brake),
            "physics_brakes/default/TailBrakeForce" => scalar!(self.tail_brake),
            "physics_steering/default/Damping" => scalar!(self.steer_follow),
            "physics_steering/default/GeneralScalar" => scalar!(self.steer_scalar),
            "physics_steering/default/SteeringVsInput" => curve!(self.steer_vs_input),
            "physics_steering/default/SteeringVsSpeed" => curve!(self.steer_vs_speed),
            "physics_steering/default/SteeringVsSpeedMaxSpeed" => scalar!(self.steer_speed_range),
            "physics_steering/default/HardTurnIncrease" => scalar!(self.hard_turn_boost),
            "physics_steering/default/PushScalarDecriment" => scalar!(self.push_steer_drop),
            "physics_steering/default/PushScalarIncriment" => scalar!(self.push_steer_rise),
            "physics_steering/default/PushScalarMin" => scalar!(self.push_steer_min),
            "physics_jump/default/AbsoluteMinHeight" => scalar!(self.jump_floor),
            "physics_mode/normal/JumpMinHeight" => scalar!(self.jump_low),
            "physics_mode/normal/JumpMaxHeight" => scalar!(self.jump_high),
            "physics_jump/default/MinHeightVsSpeed" => curve!(self.jump_low_vs_speed),
            "physics_jump/default/MaxHeightVsSpeed" => curve!(self.jump_high_vs_speed),
            "physics_jump/default/SpeedResponseMaxSpeed" => scalar!(self.jump_speed_range),
            "physics_jump/default/HippyJumpMinHeight" => scalar!(self.hippy_low),
            "physics_jump/default/HippyJumpMaxHeight" => scalar!(self.hippy_high),
            "physics_bodyspin/default/PropBodySpinVsTime" => curve!(self.spin_speed),
            "physics_bodyspin/default/MaxDeltaVsTime" => curve!(self.spin_change),
            "physics_bodyspin/default/MaxDeltaOppositeDirection" => {
                scalar!(self.spin_reverse_change)
            }
            "physics_bodyspin/default/DerivativeBodySpinVsTime" => curve!(self.spin_windup),
            "physics_bodyspin/default/MinDerivativeScalar" => scalar!(self.spin_no_windup),
            "physics_wipeout/default/Hash_4F08D9BAE6831524" => curve!(self.land_angle),
            "physics_wipeout/default/Wipeout_AirMaxSpeedIntoGround" => {
                scalar!(self.land_max_impact)
            }
            "recognizer/right_stick/NumTicksPatternNotInRangeBeforeCulling" => match f {
                Some(f) if (0.0..=63.0).contains(&f) => {
                    self.flick_misses = f as u8;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Steering strength for the current speed and stick (the deck tilt, 0..about 1).
    pub fn steer_tilt(&self, input: f32, speed: f32, hard: f32, push_scalar: f32) -> f32 {
        let speed_k = self
            .steer_vs_speed
            .eval((speed.abs() / self.steer_speed_range).clamp(0.0, 1.0));
        // Skate 3 multiplies by the signed turn value itself (not just its sign): half a stick tilts the
        // deck far less than half of a full one.
        self.steer_scalar
            * self.steer_vs_input.eval(input.abs())
            * speed_k
            * push_scalar
            * (1.0 + hard.abs() * self.hard_turn_boost)
            * input.clamp(-1.0, 1.0)
    }

    /// The speed a push adds when the button was held for `held` seconds at `speed`.
    pub fn push_gain(&self, held: f32, speed: f32) -> f32 {
        let full = self.push_hold_time.eval(speed).max(0.01);
        self.push_dv.eval((held / full).clamp(0.0, 1.0))
    }

    /// Most the rolling-resistance corrector may change the speed (m/s^2).
    pub fn corrector_limit(&self) -> f32 {
        self.friction_max_change * self.speed_general_amount * 60.0
    }

    /// Most speed one tick of push contact may add at `speed`.
    pub fn push_tick_limit(&self, speed: f32) -> f32 {
        let k = (speed / self.push_max_speed.max(0.01)).clamp(0.0, 1.0);
        self.push_tick_limit_slow + (self.push_tick_limit_fast - self.push_tick_limit_slow) * k
    }

    /// Centre-of-mass height a jump reaches, for a flick `strength` (0..1) at `speed`.
    pub fn jump_height(&self, strength: f32, speed: f32) -> f32 {
        let k = (speed.abs() / self.jump_speed_range.max(0.01)).clamp(0.0, 1.0);
        let lo =
            self.jump_floor + self.jump_low_vs_speed.eval(k) * (self.jump_low - self.jump_floor);
        let hi =
            self.jump_floor + self.jump_high_vs_speed.eval(k) * (self.jump_high - self.jump_floor);
        lo + (hi - lo) * strength.clamp(0.0, 1.0)
    }
}

/// Encodes named values as a `rules/tuning` entry.
pub fn encode(values: &[(&str, Vec<f32>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(values.len() as u32).to_be_bytes());
    for (name, v) in values {
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(v.len() as u16).to_be_bytes());
        for f in v {
            out.extend_from_slice(&f.to_be_bytes());
        }
    }
    out
}

/// Parses a `rules/tuning` entry. `None` if it is malformed.
pub fn parse(data: &[u8]) -> Option<Vec<(&str, Vec<f32>)>> {
    let u16_at = |at: usize| Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?));
    let n = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?) as usize;
    let mut at = 4;
    let mut out = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        let len = u16_at(at)? as usize;
        let name = core::str::from_utf8(data.get(at + 2..at + 2 + len)?).ok()?;
        at += 2 + len;
        let count = u16_at(at)? as usize;
        at += 2;
        let mut v = Vec::with_capacity(count);
        for k in 0..count {
            let b = data.get(at + 4 * k..at + 4 * k + 4)?;
            v.push(f32::from_be_bytes(b.try_into().ok()?));
        }
        at += 4 * count;
        out.push((name, v));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn curves_interpolate_and_hold_their_ends() {
        let c = Curve::new(&[(0.0, 1.0), (2.0, 3.0), (4.0, 3.0)]);
        assert_eq!(c.eval(-1.0), 1.0);
        assert_eq!(c.eval(1.0), 2.0);
        assert_eq!(c.eval(3.0), 3.0);
        assert_eq!(c.eval(9.0), 3.0);
        assert_eq!(Curve::constant(0.5).eval(100.0), 0.5);
    }

    #[test]
    fn a_rules_entry_overrides_known_values_only() {
        let data = encode(&[
            ("physics_push/default/MaxPushableSpeed", vec![9.5]),
            (
                "anim_motion/pushing/button_time_to_dv",
                vec![0.0, 1.0, 2.0, 5.0],
            ),
            ("camera_shots/chase_flat_fast/PositionDistance", vec![3.25]),
            (
                "recognizer/right_stick/NumTicksPatternNotInRangeBeforeCulling",
                vec![4.0],
            ),
            ("nothing/we/know", vec![1.0]),
            (
                "physics_steering/default/SteeringVsInput",
                vec![1.0, 2.0, 3.0],
            ), // odd: not a curve
        ]);
        let mut t = Tuning::BUILT_IN;
        assert_eq!(t.apply(&data), 4);
        assert_eq!(t.push_max_speed, 9.5);
        assert_eq!(t.push_dv.eval(0.5), 3.5);
        assert_eq!(t.cam_fast.distance, 3.25);
        assert_eq!(t.flick_misses, 4);
        assert_eq!(t.steer_vs_input, Tuning::BUILT_IN.steer_vs_input);
        assert!(parse(&data[..data.len() - 1]).is_none(), "truncated");
    }

    #[test]
    fn every_path_is_one_the_tuning_reads() {
        for p in PATHS {
            let mut t = Tuning::BUILT_IN;
            // A two-point curve also counts as a scalar (its first value).
            assert!(t.set(p, &[0.0, 1.0, 1.0, 1.0]) || t.set(p, &[1.0]), "{p}");
        }
    }

    #[test]
    fn camera_headings_blend_the_short_way() {
        let a = CamShot {
            heading: -160.0,
            ..Tuning::BUILT_IN.cam_slow
        };
        let b = CamShot {
            heading: 180.0,
            ..a
        };
        assert!((a.lerp(&b, 0.5).heading + 170.0).abs() < 1e-4);
    }

    #[test]
    fn stronger_flicks_and_slower_riding_jump_higher() {
        let t = Tuning::BUILT_IN;
        assert!(t.jump_height(1.0, 3.0) > t.jump_height(0.0, 3.0));
        assert!(t.jump_height(1.0, 3.0) >= t.jump_height(1.0, 20.0));
        assert!(t.jump_height(0.0, 3.0) > t.jump_floor);
    }
}
