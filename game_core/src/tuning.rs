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

pub mod adjust;
pub mod biped;
pub mod camera;
pub mod grind;
pub mod handplant;
pub mod moves;
pub mod plant;
pub mod possession;
pub mod wobble;
pub use camera::CamTuning;
pub use grind::GrindTuning;

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

    /// The last point's x (0 for an empty curve).
    pub fn last_x(&self) -> f32 {
        if self.n == 0 {
            0.0
        } else {
            self.x[self.n as usize - 1]
        }
    }

    pub const fn constant(v: f32) -> Curve {
        Curve::new(&[(0.0, v)])
    }

    /// From a tuning value: x values then y values.
    pub(crate) fn from_values(v: &[f32]) -> Option<Curve> {
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
    /// flatground's own calibration from Skate 3's brake forces to a deceleration (m/s^2 per unit),
    /// measured against SK8's simulation: its foot brake (force 50 on smooth ground) slows the board
    /// 4.08 m/s^2 once the foot drags.
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
    /// The trucks' tightness (0 loose .. 1 tight; Skate 3's equipment setting, SK8's default 0.7) and how
    /// much tight trucks steer (`SteeringScalar_TightTrucks`): the tilt is scaled by
    /// `tight_trucks_scalar * truck_tightness + 1 - truck_tightness`.
    pub truck_tightness: f32,
    pub tight_trucks_scalar: f32,
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
    /// The manual balance servo (SK8's manual controller): the deck pitch aims at
    /// (0.25 + 0.75 lean) x `manual_max_tilt` degrees plus a gentle wobble, and a PID correction (gains
    /// per 1/60 s tick; the derivative term is limited to `manual_d_limit`, the error to
    /// `manual_max_error` rad) turns the deck towards it.
    pub manual_max_tilt: f32,
    pub manual_p: f32,
    pub manual_i: f32,
    pub manual_d: f32,
    pub manual_d_limit: f32,
    pub manual_max_error: f32,
    /// Wobble: sin(2 pi `manual_noise_freq` t) x `manual_noise_scale` rad, scaled by
    /// `manual_noise_vs_speed` (against speed in m/s).
    pub manual_noise_freq: f32,
    pub manual_noise_scale: f32,
    pub manual_noise_vs_speed: Curve,
    /// The physics mode's manual corrective force (`Hash_F8CBC0F5FEF2240E`; on in Skate 3's Normal and
    /// Hardcore, off in Easy). Off, dragging the tail (or nose) while the board rolls against the
    /// manual's balance direction asks the graph to end the manual (`PhysicsWantsManualExit`; SK8
    /// manual/controller.rs `apply_correction`).
    pub manual_corrective: bool,
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
    /// How the ground presses the rider down (SK8 pumping/controller.rs `calculate`): the crouch the
    /// physics asks of the animation against the ground's angle (quarter turns), the least crouch,
    /// and the deck's angle's; times their scalars (negative: down into the board).
    pub compression_vs_ground_angle: Curve,
    pub min_crouch_vs_ground_angle: Curve,
    pub compression_vs_deck_angle: Curve,
    pub compression_ground_scale: f32,
    pub compression_deck_scale: f32,

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
    /// Most the spin speed may change per tick while the landing assist turns the body (rad/s, against
    /// time in the air).
    pub spin_auto_change: Curve,
    /// Without a known landing: the stick's spin (`max_spin_speed` deg/s per unit) scaled by this against
    /// twice the time in the air.
    pub spin_input_filter: Curve,
    pub max_spin_speed: f32,
    /// The landing assist's spin: never faster than `max_auto_spin` (rad/s), halved under
    /// `min_auto_spin`; turned the other way round (land fakie or switch) past `heading_adjust_vs_up`
    /// degrees from the landing direction (against the body's up y); only with a landing direction
    /// faster than `min_heading_vel` (m/s).
    pub max_auto_spin: f32,
    pub min_auto_spin: f32,
    pub heading_adjust_vs_up: Curve,
    pub min_heading_vel: f32,

    // Attitude in the air.
    /// Most the board's up may turn per tick (rad) towards the landing surface, against the centre of
    /// mass's distance to the deck (m).
    pub air_up_max_delta: Curve,
    /// Without a known landing: the share of the angle to the surface below turned per tick, and the
    /// angle (rad) past which the board doesn't turn to it at all.
    pub phys_air_align: f32,
    pub phys_air_dont_align: f32,

    // Landing prediction and flight assist (the trajectory selector).
    /// The candidate cone around the launch velocity: sideways and along-travel angles (degrees, the
    /// latter scaled by `traj_cone_z_vs_speed` against speed / 20 m/s), the second pass's angles, and
    /// the cone's speed range (m/s).
    pub traj_cone_x: f32,
    pub traj_cone_z: f32,
    pub traj_cone_z_vs_speed: Curve,
    pub traj_cone_x2: f32,
    pub traj_cone_z2: f32,
    pub traj_speed_min: f32,
    pub traj_speed_max: f32,
    /// How far ahead a flight is followed: seconds and metres of drop.
    pub traj_max_time: f32,
    pub traj_max_drop: f32,
    /// Scores: a bonus against flight time, the landing force and direction terms (scaled by
    /// `traj_force_scalar` against the speed into the surface), the middle candidate's bonus, the
    /// transition term on vert, unrideable and don't-align surfaces.
    pub traj_time_bonus: Curve,
    pub traj_force_scalar: Curve,
    pub traj_score_force: f32,
    pub traj_score_direction: f32,
    pub traj_score_middle: f32,
    pub traj_score_transition: f32,
    pub traj_score_unrideable: f32,
    pub traj_score_dont_align: f32,
    /// A landing sooner than this (s) isn't a known landing; within `traj_min_after_apex` of the apex
    /// a vert landing is penalised; a second, narrower pass runs when the landing surface is more than
    /// `traj_second_pass_delta` rad off the ground left.
    pub traj_min_valid_time: f32,
    pub traj_min_after_apex: f32,
    pub traj_second_pass_delta: f32,
    /// Most a candidate is nudged (m/s), and how long (s) the start error takes to blend away.
    pub traj_max_adjust: f32,
    pub traj_error_blend: f32,
    /// Wallride candidates: probe distance off the wall, least height over the floor, score per metre,
    /// the slope from launch to wall that still counts, and the upward nudge against height.
    pub traj_wall_test_dist: f32,
    pub traj_wall_min_height: f32,
    pub traj_wall_score: f32,
    pub traj_wall_land_angle: f32,
    pub traj_wall_boost: Curve,

    // Landing.
    /// Largest board-to-travel angle (rad) that still lands, against speed.
    pub land_angle: Curve,
    /// Hitting the ground faster than this (m/s, downwards) is a bail; onto stairs, onto a rail.
    pub land_max_impact: f32,
    pub land_max_stairs: f32,
    pub land_max_grind: f32,
    /// Landing while the clips say the board isn't caught: a bail past this speed into the ground
    /// (m/s), once the air is older than `danger_ignore_ticks` ticks.
    pub danger_max_impact: f32,
    pub danger_ignore_ticks: f32,
    /// The tangential speed kept on landing, against the surface's normal y (transitions gain).
    pub landing_speed_vs_ny: Curve,
    /// Landing quality magnitudes: against the side speed and the spin speed (both remapped to 0..1).
    pub land_sketchy_side: Curve,
    pub land_sketchy_twist: Curve,

    // After a bail.
    /// Automatic reset: after `reset_min_time` s with the body settled (under `settled_speed` m/s, after
    /// `settled_min_time` s) for `reset_settled` s, or after `reset_max_time` s; the screen fades for
    /// `reset_fade` s first.
    pub reset_min_time: f32,
    pub reset_settled: f32,
    pub reset_max_time: f32,
    pub reset_fade: f32,
    pub settled_speed: f32,
    pub settled_min_time: f32,
    /// Checkpoints: the ground under one no further than `respawn_drop` m down and facing up at least
    /// `respawn_normal_y`, with `respawn_height` m clear over a `respawn_radius` m capsule.
    pub respawn_height: f32,
    pub respawn_radius: f32,
    pub respawn_drop: f32,
    pub respawn_normal_y: f32,

    // Reverts (a servo on the board's yaw).
    /// Target yaw rate (rad/s) against the angle left (rad), the gain per tick, the angle (degrees,
    /// against the surface normal's y) that counts as done, and the longest a revert lasts (s).
    pub revert_speed: Curve,
    pub revert_gain: f32,
    pub revert_tolerance: Curve,
    pub revert_duration: f32,

    // Air animation (the state graphs' air behaviours).
    /// The arms' reach in the air (`EXTEND`) against seconds coming down.
    pub air_arm_extend: Curve,
    /// ControlAirLegExtension's centre of mass over the board (`disttocog`, SK8 air_leg_extension.rs):
    /// how fast it closes going up and coming down (m/s), its least, and before the landing how fast it
    /// extends to how far.
    pub air_com_up_speed: f32,
    pub air_com_down_speed: f32,
    pub air_com_min: f32,
    pub preland_com_speed: f32,
    pub preland_com: f32,
    /// The body spin channels: influence against the spin intent, its blend in and out per update, and
    /// its rate and acceleration limits.
    pub spin_map: Curve,
    pub spin_blend_in: f32,
    pub spin_blend_out: f32,
    pub spin_max_delta: f32,
    pub spin_max_acc: f32,

    // Flick input.
    /// Samples a flick may stray from its shape before it is dropped.
    pub flick_misses: u8,

    // The rider's riding animation (what the state graphs' riding behaviours feed the blend trees).
    pub anim: AnimTuning,

    // Grinds (`tuning::grind`).
    pub grind: GrindTuning,

    // The camera's trackers, collision and predictors (`tuning::camera`; the shots are their own entry).
    pub cam: CamTuning,

    // Body flips, the pump channel, bumps, the powerslide's and the wipeout's pose (`tuning::moves`).
    pub moves: moves::MoveTuning,
    /// Speed wobble (`tuning::wobble`).
    pub wobble: wobble::WobbleTuning,
    /// Board adjust in the air (`tuning::adjust`; measured from Skate 3's clips, no rules-pack values).
    pub adjust: adjust::AdjustTuning,
    /// Foot plants: the FootJump launch and the air foot plant (`tuning::plant`).
    pub plant: plant::PlantTuning,
    /// Hand plants (inverts): the coping search and the plant (`tuning::handplant`).
    pub handplant: handplant::HandPlantTuning,

    // Walking off the board (`tuning::biped`).
    pub biped: biped::BipedTuning,
    /// The board off the feet: dropped, thrown, picked up, landed on (`tuning::possession`).
    pub possession: possession::PossessionTuning,
}

/// What the state graphs' riding behaviours (`graph::riding`, ports of SK8's SetTurning, the physics'
/// animation turn conditioner, Crouching, SettingBodyTilt and SetManualAngle) need: Skate 3's
/// `anim_carving`, `physics_animation`, `anim_motion/crouching`, `anim_motion/auto_pump`,
/// `anim_motion/body_tilt` and `anim_motion/manual` values from the rules pack, or flatground's own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimTuning {
    /// Carving lean (SetTurning): the turn vector remapped by magnitude and angle curves (against the
    /// vector's angle, 0..pi) plus an angle offset, for quick turns and for slow turns, mixed by
    /// quickness.
    pub quick_mag: Curve,
    pub quick_angle: Curve,
    pub quick_offset: f32,
    pub slow_mag: Curve,
    pub slow_angle: Curve,
    pub slow_offset: f32,
    /// Speed tuck against speed over `tuck_start` (m/s).
    pub speed_tuck: Curve,
    pub tuck_start: f32,
    /// Lean blend against seconds since the lean's mode (carve, slide left, slide right) changed, and
    /// the most the lean moves per update.
    pub lean_in: Curve,
    pub lean_max_delta: f32,
    /// The animation turn conditioner: three filters' coefficients (slow lean, lean, direction).
    pub filter_lean_slow: [f32; 4],
    pub filter_lean: [f32; 4],
    pub filter_dir: [f32; 4],
    /// Turn input scale against speed (as a share of `cond[11]`), the quickness remap, quickness against
    /// speed (m/s), and the lean smoothing against speed wobble.
    pub turn_vs_speed: Curve,
    pub quickness_curve: Curve,
    pub quickness_vs_speed: Curve,
    pub wobble_blend: Curve,
    /// The conditioner's scalars, in [`COND_NAMES`] order.
    pub cond: [f32; 13],
    /// Crouching: centre-of-mass heights over the board (m), the highest against ground speed (m/s).
    pub crouch_min_height: f32,
    pub crouch_max_height: Curve,
    pub crouch_max_ratio: f32,
    pub crouch_max_delta: f32,
    pub crouch_max_delta_delta: f32,
    pub crouch_blend_input: f32,
    /// Absorption: the restoring force against the crouch, the rising speed limit against it (times
    /// `crouch_pump_speed_y`), and how the board's push reaches the rider.
    pub crouch_upforce: Curve,
    pub crouch_pump_speed: Curve,
    pub crouch_pump_speed_y: f32,
    pub crouch_from_deck: f32,
    pub crouch_board_damping: f32,
    pub crouch_max_force: f32,
    pub crouch_max_ground_force: f32,
    pub crouch_absorption: f32,
    /// Auto pump: the most crouch against ground speed, and the scalars in [`AUTO_PUMP_NAMES`] order.
    pub auto_pump_max_crouch: Curve,
    pub auto_pump: [f32; 10],
    /// Body tilt (`TILT_X`): its scale against body spin speed, and its rate limits per update on the
    /// ground and in the air.
    pub tilt_spin_factor: Curve,
    pub tilt_ground_vel: f32,
    pub tilt_ground_acc: f32,
    pub tilt_air_vel: f32,
    pub tilt_air_acc: f32,
    /// Manual pose (`MANUAL_ANGLE`): the angle against the C-stick, and its rate limits per update.
    pub manual_balance: Curve,
    pub manual_vel: f32,
    pub manual_acc: f32,
}

/// The turn conditioner's scalars (`physics_animation/default/<name>`), as [`AnimTuning::cond`] holds
/// them.
pub const COND_NAMES: [&str; 13] = [
    "QuicknessMaxDelta",
    "InputQuicknessMax",
    "InputQuicknessBlendVal",
    "HoldingMinVal",
    "HoldingMaxAcceleration",
    "HoldingIntegralBlend",
    "HoldingBlendUp",
    "HoldingBlendDown",
    "AnimationTurnWobbleScalar",
    "AnimationTurnPhysicsWeight",
    "AnimationTurnMaxLean",
    "AnimationTurnGraphX",
    "AnimationTurnDirMax",
];

/// The auto pump's scalars (`anim_motion/auto_pump/<name>`), as [`AnimTuning::auto_pump`] holds them.
pub const AUTO_PUMP_NAMES: [&str; 10] = [
    "auto_pump_sufficient_crouch",
    "auto_pump_standing_thresh",
    "auto_pump_rise_speed",
    "auto_pump_pump_speed",
    "auto_pump_potential_thresh",
    "auto_pump_potential_blend",
    "auto_pump_intent_mag_start",
    "auto_pump_intent_angle_region",
    "auto_pump_crouch_time",
    "auto_pump_crouch_speed",
];

impl AnimTuning {
    /// flatground's own values.
    pub const BUILT_IN: AnimTuning = AnimTuning {
        quick_mag: Curve::constant(1.0),
        quick_angle: Curve::new(&[(0.0, 0.0), (3.15, 3.15)]),
        quick_offset: 0.0,
        slow_mag: Curve::constant(1.0),
        slow_angle: Curve::new(&[(0.0, 0.0), (3.15, 3.15)]),
        slow_offset: 0.0,
        speed_tuck: Curve::new(&[(0.0, 0.0), (8.0, 1.0)]),
        tuck_start: 10.0,
        lean_in: Curve::new(&[(0.0, 0.0), (0.5, 1.0)]),
        lean_max_delta: 0.5,
        filter_lean_slow: [0.03, 0.04, 0.0, 0.1],
        filter_lean: [0.15, 0.2, 0.0, 0.1],
        filter_dir: [0.02, 0.03, 0.0, 0.05],
        turn_vs_speed: Curve::new(&[(0.0, 0.0), (0.25, 0.5), (0.6, 0.9), (1.0, 1.0)]),
        quickness_curve: Curve::new(&[(0.0, 0.0), (0.5, 0.2), (1.0, 1.0)]),
        quickness_vs_speed: Curve::new(&[(0.0, 1.0), (10.0, 1.0), (20.0, 0.0)]),
        wobble_blend: Curve::new(&[(0.0, 1.0), (1.0, 0.25)]),
        cond: [
            0.005, 0.04, 0.03, 0.03, 0.03, 0.12, 0.03, 0.1, 1.0, 0.5, 0.8, 15.0, 0.1,
        ],
        crouch_min_height: 0.58,
        crouch_max_height: Curve::new(&[(0.0, 0.96), (10.0, 0.96), (30.0, 0.6)]),
        crouch_max_ratio: 1.0,
        crouch_max_delta: 0.06,
        crouch_max_delta_delta: 0.03,
        crouch_blend_input: 0.1,
        crouch_upforce: Curve::new(&[(0.0, 0.4), (1.0, 0.0)]),
        crouch_pump_speed: Curve::new(&[(0.0, 1.0), (1.0, 0.2)]),
        crouch_pump_speed_y: 0.12,
        crouch_from_deck: 0.8,
        crouch_board_damping: 0.85,
        crouch_max_force: 12.0,
        crouch_max_ground_force: 12.0,
        crouch_absorption: 0.06,
        auto_pump_max_crouch: Curve::new(&[(0.0, 0.2), (8.0, 0.9)]),
        auto_pump: [0.6, 0.15, 0.03, 0.08, 0.25, 0.1, 0.85, 0.9, 0.7, 0.12],
        tilt_spin_factor: Curve::new(&[(0.0, 1.0), (3.0, 0.6), (6.0, 0.0)]),
        tilt_ground_vel: 0.025,
        tilt_ground_acc: 0.005,
        tilt_air_vel: 0.04,
        tilt_air_acc: 0.04,
        manual_balance: Curve::new(&[(0.0, 0.0), (0.7, 0.0), (1.0, 1.0)]),
        manual_vel: 0.05,
        manual_acc: 0.025,
    };

    fn set(&mut self, name: &str, f: Option<f32>, c: Option<Curve>, v: &[f32]) -> bool {
        let field = name.rsplit('/').next().unwrap_or("");
        let four = |v: &[f32]| -> Option<[f32; 4]> {
            (v.len() == 4 && v.iter().all(|x| x.is_finite())).then(|| [v[0], v[1], v[2], v[3]])
        };
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
                match c {
                    Some(c) => {
                        $field = c;
                        true
                    }
                    None => false,
                }
            };
        }
        macro_rules! vec4 {
            ($field:expr) => {
                match four(v) {
                    Some(x) => {
                        $field = x;
                        true
                    }
                    None => false,
                }
            };
        }
        if name.starts_with("physics_animation/default/")
            && let Some(i) = COND_NAMES.iter().position(|n| *n == field)
        {
            return scalar!(self.cond[i]);
        }
        if name.starts_with("anim_motion/auto_pump/")
            && let Some(i) = AUTO_PUMP_NAMES.iter().position(|n| *n == field)
        {
            return scalar!(self.auto_pump[i]);
        }
        match name {
            "anim_carving/default/quickMagMap" => curve!(self.quick_mag),
            "anim_carving/default/quickAngleMap" => curve!(self.quick_angle),
            "anim_carving/default/quickAngleOffset" => scalar!(self.quick_offset),
            "anim_carving/default/slowMagMap" => curve!(self.slow_mag),
            "anim_carving/default/slowAngleMap" => curve!(self.slow_angle),
            "anim_carving/default/slowAngleOffset" => scalar!(self.slow_offset),
            "anim_carving/default/speed_tuck" => curve!(self.speed_tuck),
            "anim_carving/default/speed_tuck_start" => scalar!(self.tuck_start),
            "anim_carving/default/blend_lean_in" => curve!(self.lean_in),
            "anim_carving/default/clamp_curr_lean" => scalar!(self.lean_max_delta),
            "physics_animation/default/AnimationTurnLeanFilterSlow" => {
                vec4!(self.filter_lean_slow)
            }
            "physics_animation/default/AnimationTurnLeanFilter" => vec4!(self.filter_lean),
            "physics_animation/default/AnimationTurnDirFilter" => vec4!(self.filter_dir),
            "physics_animation/default/AnimationTurnSpeedGraph" => curve!(self.turn_vs_speed),
            "physics_animation/default/QuicknessVsQuickness" => curve!(self.quickness_curve),
            "physics_animation/default/ScaleQuicknessAtSpeed" => curve!(self.quickness_vs_speed),
            "physics_animation/default/SpeedWobbleTurnBlend" => curve!(self.wobble_blend),
            "anim_motion/crouching/crouching_min_height" => scalar!(self.crouch_min_height),
            "anim_motion/crouching/crouching_max_height" => curve!(self.crouch_max_height),
            "anim_motion/crouching/crouch_max_ratio" => scalar!(self.crouch_max_ratio),
            "anim_motion/crouching/crouch_max_delta" => scalar!(self.crouch_max_delta),
            "anim_motion/crouching/crouch_max_delta_delta" => {
                scalar!(self.crouch_max_delta_delta)
            }
            "anim_motion/crouching/crouch_blend_input" => scalar!(self.crouch_blend_input),
            "anim_motion/crouching/absorption_upforce" => curve!(self.crouch_upforce),
            "anim_motion/crouching/pump_maxspeed" => curve!(self.crouch_pump_speed),
            "anim_motion/crouching/pump_maxspeed_yaxis" => scalar!(self.crouch_pump_speed_y),
            "anim_motion/crouching/MaxCrouchFromDeckAngle" => scalar!(self.crouch_from_deck),
            "anim_motion/crouching/absorption_skateboard_damping" => {
                scalar!(self.crouch_board_damping)
            }
            "anim_motion/crouching/absorption_maxforce" => scalar!(self.crouch_max_force),
            "anim_motion/crouching/absorption_ground_maxforce" => {
                scalar!(self.crouch_max_ground_force)
            }
            "anim_motion/crouching/absorption_factor" => scalar!(self.crouch_absorption),
            "anim_motion/auto_pump/auto_pump_max_crouch" => curve!(self.auto_pump_max_crouch),
            "anim_motion/body_tilt/body_tilt_bodyspin_factor" => curve!(self.tilt_spin_factor),
            "anim_motion/body_tilt/body_tilt_ground_clamp_vel" => scalar!(self.tilt_ground_vel),
            "anim_motion/body_tilt/body_tilt_ground_clamp_acc" => scalar!(self.tilt_ground_acc),
            "anim_motion/body_tilt/body_tilt_air_clamp_vel" => scalar!(self.tilt_air_vel),
            "anim_motion/body_tilt/body_tilt_air_clamp_acc" => scalar!(self.tilt_air_acc),
            "anim_motion/manual/manual_balance" => curve!(self.manual_balance),
            "anim_motion/manual/manual_clamp_vel" => scalar!(self.manual_vel),
            "anim_motion/manual/manual_clamp_acc" => scalar!(self.manual_acc),
            _ => false,
        }
    }
}

impl Default for Tuning {
    fn default() -> Self {
        Self::BUILT_IN
    }
}

/// Every path the tuning reads: [`PATHS`] and the grinds' ([`grind::PATHS`]). `fg_import rules` exports
/// exactly these.
pub fn all_paths() -> impl Iterator<Item = &'static str> {
    PATHS
        .iter()
        .chain(grind::PATHS.iter())
        .chain(camera::PATHS.iter())
        .chain(moves::PATHS.iter())
        .chain(wobble::PATHS.iter())
        .chain(biped::PATHS.iter())
        .chain(possession::PATHS.iter())
        .chain(plant::PATHS.iter())
        .chain(handplant::PATHS.iter())
        .copied()
}

/// Where each value lives in Skate 3's tuning database (`class/key/field`).
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
    "physics_steering/default/SteeringScalar_TightTrucks",
    "physics_manual/default/Tilt_TiltScalar",
    "physics_manual/default/BrakeTiltAngle",
    "physics_manual/default/MaxTiltAngle",
    "physics_manual/default/ManualScalar_P",
    "physics_manual/default/ManualScalar_I",
    "physics_manual/default/ManualScalar_D",
    "physics_manual/default/D_Scalar_Limit",
    "physics_manual/default/MaxAngleDiff",
    "physics_manual/default/ProceduralNoiseFreq",
    "physics_manual/default/ProceduralNoiseScalar",
    "physics_manual/default/NoiseVsSpeed",
    "physics_mode/normal/Hash_F8CBC0F5FEF2240E",
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
    "physics_pumping/default/CompressionVsGroundAngle",
    "physics_pumping/default/MinCrouchVsGroundAngle",
    "physics_pumping/default/CompressionVsDeckAngle",
    "physics_pumping/default/CompressionGroundScalar",
    "physics_pumping/default/CompressionDeckScalar",
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
    "physics_bodyspin/default/MaxDeltaForAutoVsTime",
    "physics_airstates/default/BodySpinInputFilter",
    "physics_airstates/default/MaxSpinSpeed",
    "physics_mode/normal/MaxAutoBodySpinSpeed",
    "physics_airstates/default/MinAutoBodySpeed",
    "physics_airstates/default/MaxHeadingAdjustVsUpY",
    "physics_airstates/default/MinTargetHeadingVel",
    "physics_reckoning/default/Air_MaxUpVectAngleDelta",
    "physics_airstates/default/SpeedToAlignToGround_PhysAir",
    "physics_airstates/default/DontAlignAnglePhysicsAir",
    "physics_trajectory/default/ConeAngleX",
    "physics_trajectory/default/ConeAngleZ",
    "physics_trajectory/default/ConeAngleZVsSpeed",
    "physics_trajectory/default/ConeAngleXSecondPass",
    "physics_trajectory/default/ConeAngleZSecondPass",
    "physics_trajectory/default/SpeedFactorMin",
    "physics_trajectory/default/SpeedFactorMax",
    "physics_trajectory/default/TrajectoryMaxTime",
    "physics_trajectory/default/TrajectoryMaxDrop",
    "physics_trajectory/default/LandingTimeBonus",
    "physics_trajectory/default/LandingForceScalarVsDPVelNorm",
    "physics_trajectory/default/ScoreLandingForce",
    "physics_trajectory/default/ScoreLandingDirection",
    "physics_trajectory/default/ScoreMiddleBonus",
    "physics_trajectory/default/ScoreTransition",
    "physics_trajectory/default/ScoreSurface_Unrideable",
    "physics_trajectory/default/ScoreSurface_DontAlign",
    "physics_trajectory/default/MinTimeForTrajectoryToBeValid",
    "physics_trajectory/default/MinTimeAfterApexForTransition",
    "physics_trajectory/default/MinNormalDeltaForSecondPass",
    "physics_trajectory/default/MaxTrajectoryAdjust",
    "physics_airstates/default/TrajErrorBlendAwayTime",
    "physics_trajectory/default/WallRideTestDist",
    "physics_trajectory/default/WallRideMinHeightFromGround",
    "physics_trajectory/default/ScoreWallrideHeight",
    "physics_trajectory/default/WallRideAngleToAllowLanding",
    "physics_trajectory/default/WallRideBoost",
    // Landing angle limit against speed (the database has no name for this field).
    "physics_wipeout/default/Hash_4F08D9BAE6831524",
    "physics_wipeout/default/Wipeout_AirMaxSpeedIntoGround",
    "physics_wipeout/default/Wipeout_AirMaxSpeedIntoStairs",
    "physics_wipeout/default/Wipeout_AirMaxSpeedIntoCollisionNearGrind",
    "physics_wipeout/default/Wipeout_AirYTrick",
    "physics_wipeout/default/FramesAtStartOfAirToIgnoreDangerZone",
    "physics_airstates/default/LandingSpeedScalarVsGroundNormalY",
    "physics_animation/default/LandingSketchySideSpeed",
    "physics_animation/default/LandingSketchyTwistSpin",
    "physics_wipeout/default/TeleportMinTimeForAutoReset",
    "physics_wipeout/default/TeleportMinTimeAfterSettlingForAutoReset",
    "physics_wipeout/default/TeleportMaxTime",
    "physics_wipeout/default/TeleportAutoResetFadeOutTime",
    "physics_wipeout/default/WipeoutOverSpeed",
    "physics_wipeout/default/WipeoutOverMinTime",
    // The checkpoint manager's ground and clearance checks (no names in the database).
    "Hash_12B64C0E804B0853/default/Hash_C0526C883AF0ECCA",
    "Hash_12B64C0E804B0853/default/Hash_CEB092E418A5B001",
    "Hash_12B64C0E804B0853/default/Hash_8ABE098D3806D273",
    "Hash_12B64C0E804B0853/default/Hash_ADD032CACF6A1C15",
    // The revert servo: speed against angle left, gain, tolerance against normal y, longest time.
    "Hash_018E8A2E5028AB3F/default/Hash_DAA990E803A782A1",
    "Hash_018E8A2E5028AB3F/default/Hash_0E5B939FD4ECDBAA",
    "Hash_018E8A2E5028AB3F/default/Hash_3387562E9BEA0AF6",
    "Hash_018E8A2E5028AB3F/default/Hash_035D9805EB9E2EE2",
    "anim_motion/inair_disttocog/lo_air_arm_extend",
    "anim_motion/inair_disttocog/goingup_disttocog_speed",
    "anim_motion/inair_disttocog/goingdown_disttocog_speed",
    "anim_motion/inair_disttocog/min_inair_disttocog",
    "anim_motion/inair_disttocog/preland_disttocom_vel",
    "anim_motion/inair_disttocog/preland_final_disttocom",
    "anim_motion/body_spin/spin_map",
    "anim_motion/body_spin/spin_influence_blendin",
    "anim_motion/body_spin/spin_influence_blendout",
    "anim_motion/body_spin/spin_clamp_influence_delta",
    "anim_motion/body_spin/spin_clamp_influence_deltadelta",
    "recognizer/right_stick/NumTicksPatternNotInRangeBeforeCulling",
    "anim_carving/default/quickMagMap",
    "anim_carving/default/quickAngleMap",
    "anim_carving/default/quickAngleOffset",
    "anim_carving/default/slowMagMap",
    "anim_carving/default/slowAngleMap",
    "anim_carving/default/slowAngleOffset",
    "anim_carving/default/speed_tuck",
    "anim_carving/default/speed_tuck_start",
    "anim_carving/default/blend_lean_in",
    "anim_carving/default/clamp_curr_lean",
    "physics_animation/default/AnimationTurnLeanFilterSlow",
    "physics_animation/default/AnimationTurnLeanFilter",
    "physics_animation/default/AnimationTurnDirFilter",
    "physics_animation/default/AnimationTurnSpeedGraph",
    "physics_animation/default/QuicknessVsQuickness",
    "physics_animation/default/ScaleQuicknessAtSpeed",
    "physics_animation/default/SpeedWobbleTurnBlend",
    "physics_animation/default/QuicknessMaxDelta",
    "physics_animation/default/InputQuicknessMax",
    "physics_animation/default/InputQuicknessBlendVal",
    "physics_animation/default/HoldingMinVal",
    "physics_animation/default/HoldingMaxAcceleration",
    "physics_animation/default/HoldingIntegralBlend",
    "physics_animation/default/HoldingBlendUp",
    "physics_animation/default/HoldingBlendDown",
    "physics_animation/default/AnimationTurnWobbleScalar",
    "physics_animation/default/AnimationTurnPhysicsWeight",
    "physics_animation/default/AnimationTurnMaxLean",
    "physics_animation/default/AnimationTurnGraphX",
    "physics_animation/default/AnimationTurnDirMax",
    "anim_motion/crouching/crouching_min_height",
    "anim_motion/crouching/crouching_max_height",
    "anim_motion/crouching/crouch_max_ratio",
    "anim_motion/crouching/crouch_max_delta",
    "anim_motion/crouching/crouch_max_delta_delta",
    "anim_motion/crouching/crouch_blend_input",
    "anim_motion/crouching/absorption_upforce",
    "anim_motion/crouching/pump_maxspeed",
    "anim_motion/crouching/pump_maxspeed_yaxis",
    "anim_motion/crouching/MaxCrouchFromDeckAngle",
    "anim_motion/crouching/absorption_skateboard_damping",
    "anim_motion/crouching/absorption_maxforce",
    "anim_motion/crouching/absorption_ground_maxforce",
    "anim_motion/crouching/absorption_factor",
    "anim_motion/auto_pump/auto_pump_max_crouch",
    "anim_motion/auto_pump/auto_pump_sufficient_crouch",
    "anim_motion/auto_pump/auto_pump_standing_thresh",
    "anim_motion/auto_pump/auto_pump_rise_speed",
    "anim_motion/auto_pump/auto_pump_pump_speed",
    "anim_motion/auto_pump/auto_pump_potential_thresh",
    "anim_motion/auto_pump/auto_pump_potential_blend",
    "anim_motion/auto_pump/auto_pump_intent_mag_start",
    "anim_motion/auto_pump/auto_pump_intent_angle_region",
    "anim_motion/auto_pump/auto_pump_crouch_time",
    "anim_motion/auto_pump/auto_pump_crouch_speed",
    "anim_motion/body_tilt/body_tilt_bodyspin_factor",
    "anim_motion/body_tilt/body_tilt_ground_clamp_vel",
    "anim_motion/body_tilt/body_tilt_ground_clamp_acc",
    "anim_motion/body_tilt/body_tilt_air_clamp_vel",
    "anim_motion/body_tilt/body_tilt_air_clamp_acc",
    "anim_motion/manual/manual_balance",
    "anim_motion/manual/manual_clamp_vel",
    "anim_motion/manual/manual_clamp_acc",
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
        // Skate 3's smooth concrete (`physics_surfaces/smooth/FrictionVsSpeedNew`): no rolling resistance
        // below about 8 m/s.
        friction: Curve::new(&[
            (0.0, 0.0),
            (8.120_522, 0.0),
            (10.421_34, 0.03),
            (12.045_44, 0.162_857),
            (12.857_49, 0.6),
            (14.332_25, 0.904_054),
            (18.403_91, 1.05),
            (26.978_18, 1.2),
        ]),
        friction_no_input: Curve::new(&[
            (0.0, 0.128_571),
            (1.145_145, 0.151_786),
            (2.282_282, 0.207_143),
            (3.331_331, 0.292_857),
            (4.196_196, 0.385_714),
            (5.157_157, 0.441_071),
            (6.526_526, 0.480_357),
            (8.0, 0.5),
        ]),
        no_input_time: 3.0,
        friction_manual: Curve::new(&[(0.0, 0.1), (8.0, 0.45)]),
        foot_brake: 50.0,
        tail_brake: 32.0,
        brake_scale: 0.0816,
        braking_scalar: 1.0,
        brake_slow_speed: 0.02,
        friction_max_change: 0.3,
        speed_general_amount: 0.25,
        steer_follow: 0.7,
        steer_scalar: 0.6,
        steer_vs_input: Curve::new(&[
            (0.0, 0.0),
            (0.058_594, 0.0),
            (0.25, 0.033_784),
            (0.375, 0.077_703),
            (0.508_789, 0.172_297),
            (0.690_43, 0.334_46),
            (0.857_715, 0.564_189),
            (1.0, 0.834_459),
        ]),
        steer_vs_speed: Curve::new(&[
            (0.056_955, 1.0),
            (0.123_768, 0.969_595),
            (0.173_056, 0.847_973),
            (0.219_058, 0.608_108),
            (0.297_919, 0.395_27),
            (0.4, 0.3),
            (0.5, 0.28),
            (1.0, 0.27),
        ]),
        steer_speed_range: 50.0,
        hard_turn_boost: 1.0,
        truck_tightness: 0.7,
        tight_trucks_scalar: 0.7,
        push_steer_drop: 0.02,
        push_steer_rise: 0.05,
        push_steer_min: 0.5,
        steer_tilt_blend: 0.2,
        manual_tilt: 0.5,
        manual_brake_tilt: 30.0,
        manual_max_tilt: 24.0,
        manual_p: 0.06,
        manual_i: 0.02,
        manual_d: -0.75,
        manual_d_limit: 0.05,
        manual_max_error: 0.12,
        manual_noise_freq: 1.2,
        manual_noise_scale: 0.04,
        manual_noise_vs_speed: Curve::new(&[(0.0, 0.3), (8.0, 1.0)]),
        // Skate 3's Normal mode.
        manual_corrective: true,
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
        pump_damping: 0.1,
        pump_min_change: 0.004,
        pump_max_change: 0.006,
        pump_vs_time: Curve::new(&[
            (0.0, 1.0),
            (0.402, 1.0),
            (0.507, 0.896),
            (0.572, 0.732),
            (0.653, 0.432),
            (0.741, 0.246),
            (0.862, 0.114),
            (0.995, 0.096),
        ]),
        pump_ang_damping: 0.2,
        pump_vs_vel: Curve::new(&[
            (0.0, 4.0),
            (2.45, 2.443),
            (4.56, 1.1),
            (7.01, 1.0),
            (8.81, 0.964),
            (10.68, 0.571),
            (12.2, 0.157),
            (14.2, 0.007),
        ]),
        pump_factor: 18.0,
        pump_max_accel: 10.0,
        pump_unintentional: 0.7,
        compression_vs_ground_angle: Curve::new(&[
            (0.0, 0.0),
            (0.072, 0.044),
            (0.147, 0.196),
            (0.238, 0.321),
            (0.381, 0.372),
            (0.793, 0.48),
            (0.938, 0.757),
            (1.0, 1.0),
        ]),
        min_crouch_vs_ground_angle: Curve::new(&[
            (0.0, 0.0),
            (0.09, 0.0),
            (0.161, 0.03),
            (0.34, 0.365),
            (0.461, 0.537),
            (0.607, 0.666),
            (0.792, 0.75),
            (1.0, 0.794),
        ]),
        compression_vs_deck_angle: Curve::new(&[
            (0.0, 0.0),
            (0.061, 0.253),
            (0.112, 0.419),
            (0.163, 0.601),
            (0.239, 0.767),
            (0.313, 0.878),
            (0.371, 0.929),
            (0.528, 1.0),
        ]),
        compression_ground_scale: -8.0,
        compression_deck_scale: -8.0,
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
        spin_auto_change: Curve::new(&[(0.0, 0.8), (0.3, 0.55), (1.0, 0.4)]),
        spin_input_filter: Curve::new(&[(0.0, 0.0), (0.05, 1.0), (0.3, 0.8), (1.0, 0.45)]),
        max_spin_speed: 540.0,
        max_auto_spin: 7.0,
        min_auto_spin: 0.12,
        heading_adjust_vs_up: Curve::new(&[(0.0, 130.0), (0.8, 125.0), (0.92, 95.0), (1.0, 90.0)]),
        min_heading_vel: 0.8,
        air_up_max_delta: Curve::new(&[(0.5, 0.15), (0.75, 0.1), (1.0, 0.07)]),
        phys_air_align: 0.12,
        phys_air_dont_align: 1.5,
        traj_cone_x: 12.0,
        traj_cone_z: 35.0,
        traj_cone_z_vs_speed: Curve::new(&[(0.0, 0.35), (0.5, 0.45), (1.0, 0.55)]),
        traj_cone_x2: 6.0,
        traj_cone_z2: 12.0,
        traj_speed_min: 2.0,
        traj_speed_max: 3.5,
        traj_max_time: 3.5,
        traj_max_drop: 35.0,
        traj_time_bonus: Curve::new(&[(0.0, 0.0), (0.1, 0.0), (0.25, 500.0), (0.45, 1000.0)]),
        traj_force_scalar: Curve::new(&[(0.0, 0.0), (0.1, 0.3), (0.3, 1.0)]),
        traj_score_force: 25.0,
        traj_score_direction: 150.0,
        traj_score_middle: 50.0,
        traj_score_transition: 400.0,
        traj_score_unrideable: -150_000.0,
        traj_score_dont_align: -80_000.0,
        traj_min_valid_time: 0.22,
        traj_min_after_apex: 0.12,
        traj_second_pass_delta: 0.12,
        traj_max_adjust: 1.2,
        traj_error_blend: 0.25,
        traj_wall_test_dist: 0.06,
        traj_wall_min_height: 0.5,
        traj_wall_score: 400.0,
        traj_wall_land_angle: 0.0,
        traj_wall_boost: Curve::new(&[(0.5, 0.4), (1.5, 0.0)]),
        land_angle: Curve::new(&[(0.0, 1.7), (10.0, 1.5), (20.0, 0.6), (28.0, 0.3)]),
        land_max_impact: 11.0,
        land_max_stairs: 5.5,
        land_max_grind: 7.5,
        danger_max_impact: 0.8,
        danger_ignore_ticks: 5.0,
        landing_speed_vs_ny: Curve::new(&[(0.4, 1.0), (0.55, 1.12), (0.7, 1.0)]),
        land_sketchy_side: Curve::new(&[(0.0, 0.0), (0.3, 0.3), (0.6, 0.8), (0.9, 1.0)]),
        land_sketchy_twist: Curve::new(&[(0.0, 0.0), (0.3, 0.5), (0.6, 0.85), (1.0, 1.0)]),
        reset_min_time: 3.0,
        reset_settled: 2.0,
        reset_max_time: 18.0,
        reset_fade: 0.4,
        settled_speed: 0.6,
        settled_min_time: 0.4,
        respawn_height: 0.5,
        respawn_radius: 0.4,
        respawn_drop: 1.2,
        respawn_normal_y: 0.8,
        revert_speed: Curve::new(&[(0.0, 4.0), (0.3, 7.0), (1.2, 9.0), (2.6, 9.0), (3.1, 8.0)]),
        revert_gain: 0.2,
        revert_tolerance: Curve::new(&[(0.0, 35.0), (0.7, 8.0), (1.0, 8.0)]),
        revert_duration: 1.1,
        air_arm_extend: Curve::new(&[(0.0, 0.0), (0.3, 0.6), (0.6, 1.0)]),
        air_com_up_speed: 0.2,
        air_com_down_speed: 0.4,
        air_com_min: 0.5,
        preland_com_speed: 2.0,
        preland_com: 1.0,
        spin_map: Curve::new(&[(0.0, 0.0), (0.3, 0.2), (1.0, 1.0)]),
        spin_blend_in: 0.2,
        spin_blend_out: 0.1,
        spin_max_delta: 0.08,
        spin_max_acc: 0.02,
        flick_misses: 8,
        anim: AnimTuning::BUILT_IN,
        grind: GrindTuning::BUILT_IN,
        cam: CamTuning::BUILT_IN,
        moves: moves::MoveTuning::BUILT_IN,
        wobble: wobble::WobbleTuning::BUILT_IN,
        adjust: adjust::AdjustTuning::BUILT_IN,
        plant: plant::PlantTuning::BUILT_IN,
        handplant: handplant::HandPlantTuning::BUILT_IN,
        biped: biped::BipedTuning::BUILT_IN,
        possession: possession::PossessionTuning::BUILT_IN,
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
            "physics_manual/default/MaxTiltAngle" => scalar!(self.manual_max_tilt),
            "physics_manual/default/ManualScalar_P" => scalar!(self.manual_p),
            "physics_manual/default/ManualScalar_I" => scalar!(self.manual_i),
            "physics_manual/default/ManualScalar_D" => scalar!(self.manual_d),
            "physics_manual/default/D_Scalar_Limit" => scalar!(self.manual_d_limit),
            "physics_manual/default/MaxAngleDiff" => scalar!(self.manual_max_error),
            "physics_manual/default/ProceduralNoiseFreq" => scalar!(self.manual_noise_freq),
            "physics_manual/default/ProceduralNoiseScalar" => scalar!(self.manual_noise_scale),
            "physics_manual/default/NoiseVsSpeed" => curve!(self.manual_noise_vs_speed),
            // A flag byte: any non-zero value is on.
            "physics_mode/normal/Hash_F8CBC0F5FEF2240E" => match v.first() {
                Some(b) => {
                    self.manual_corrective = *b != 0.0;
                    true
                }
                None => false,
            },
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
            "physics_pumping/default/CompressionVsGroundAngle" => {
                curve!(self.compression_vs_ground_angle)
            }
            "physics_pumping/default/MinCrouchVsGroundAngle" => {
                curve!(self.min_crouch_vs_ground_angle)
            }
            "physics_pumping/default/CompressionVsDeckAngle" => {
                curve!(self.compression_vs_deck_angle)
            }
            "physics_pumping/default/CompressionGroundScalar" => {
                scalar!(self.compression_ground_scale)
            }
            "physics_pumping/default/CompressionDeckScalar" => scalar!(self.compression_deck_scale),
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
            "physics_steering/default/SteeringScalar_TightTrucks" => {
                scalar!(self.tight_trucks_scalar)
            }
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
            "physics_bodyspin/default/MaxDeltaForAutoVsTime" => curve!(self.spin_auto_change),
            "physics_airstates/default/BodySpinInputFilter" => curve!(self.spin_input_filter),
            "physics_airstates/default/MaxSpinSpeed" => scalar!(self.max_spin_speed),
            "physics_mode/normal/MaxAutoBodySpinSpeed" => scalar!(self.max_auto_spin),
            "physics_airstates/default/MinAutoBodySpeed" => scalar!(self.min_auto_spin),
            "physics_airstates/default/MaxHeadingAdjustVsUpY" => curve!(self.heading_adjust_vs_up),
            "physics_airstates/default/MinTargetHeadingVel" => scalar!(self.min_heading_vel),
            "physics_reckoning/default/Air_MaxUpVectAngleDelta" => curve!(self.air_up_max_delta),
            "physics_airstates/default/SpeedToAlignToGround_PhysAir" => {
                scalar!(self.phys_air_align)
            }
            "physics_airstates/default/DontAlignAnglePhysicsAir" => {
                scalar!(self.phys_air_dont_align)
            }
            "physics_trajectory/default/ConeAngleX" => scalar!(self.traj_cone_x),
            "physics_trajectory/default/ConeAngleZ" => scalar!(self.traj_cone_z),
            "physics_trajectory/default/ConeAngleZVsSpeed" => curve!(self.traj_cone_z_vs_speed),
            "physics_trajectory/default/ConeAngleXSecondPass" => scalar!(self.traj_cone_x2),
            "physics_trajectory/default/ConeAngleZSecondPass" => scalar!(self.traj_cone_z2),
            "physics_trajectory/default/SpeedFactorMin" => scalar!(self.traj_speed_min),
            "physics_trajectory/default/SpeedFactorMax" => scalar!(self.traj_speed_max),
            "physics_trajectory/default/TrajectoryMaxTime" => scalar!(self.traj_max_time),
            "physics_trajectory/default/TrajectoryMaxDrop" => scalar!(self.traj_max_drop),
            "physics_trajectory/default/LandingTimeBonus" => curve!(self.traj_time_bonus),
            "physics_trajectory/default/LandingForceScalarVsDPVelNorm" => {
                curve!(self.traj_force_scalar)
            }
            "physics_trajectory/default/ScoreLandingForce" => scalar!(self.traj_score_force),
            "physics_trajectory/default/ScoreLandingDirection" => {
                scalar!(self.traj_score_direction)
            }
            "physics_trajectory/default/ScoreMiddleBonus" => scalar!(self.traj_score_middle),
            "physics_trajectory/default/ScoreTransition" => scalar!(self.traj_score_transition),
            "physics_trajectory/default/ScoreSurface_Unrideable" => {
                scalar!(self.traj_score_unrideable)
            }
            "physics_trajectory/default/ScoreSurface_DontAlign" => {
                scalar!(self.traj_score_dont_align)
            }
            "physics_trajectory/default/MinTimeForTrajectoryToBeValid" => {
                scalar!(self.traj_min_valid_time)
            }
            "physics_trajectory/default/MinTimeAfterApexForTransition" => {
                scalar!(self.traj_min_after_apex)
            }
            "physics_trajectory/default/MinNormalDeltaForSecondPass" => {
                scalar!(self.traj_second_pass_delta)
            }
            "physics_trajectory/default/MaxTrajectoryAdjust" => scalar!(self.traj_max_adjust),
            "physics_airstates/default/TrajErrorBlendAwayTime" => scalar!(self.traj_error_blend),
            "physics_trajectory/default/WallRideTestDist" => scalar!(self.traj_wall_test_dist),
            "physics_trajectory/default/WallRideMinHeightFromGround" => {
                scalar!(self.traj_wall_min_height)
            }
            "physics_trajectory/default/ScoreWallrideHeight" => scalar!(self.traj_wall_score),
            "physics_trajectory/default/WallRideAngleToAllowLanding" => {
                scalar!(self.traj_wall_land_angle)
            }
            "physics_trajectory/default/WallRideBoost" => curve!(self.traj_wall_boost),
            "physics_wipeout/default/Wipeout_AirMaxSpeedIntoStairs" => {
                scalar!(self.land_max_stairs)
            }
            "physics_wipeout/default/Wipeout_AirMaxSpeedIntoCollisionNearGrind" => {
                scalar!(self.land_max_grind)
            }
            "physics_wipeout/default/Wipeout_AirYTrick" => scalar!(self.danger_max_impact),
            // An integer in the database: a 4-byte one arrives as its bits read as a float.
            "physics_wipeout/default/FramesAtStartOfAirToIgnoreDangerZone" => match f {
                Some(f) if f.to_bits() < 0x0080_0000 => {
                    self.danger_ignore_ticks = f.to_bits() as f32;
                    true
                }
                Some(f) if (0.0..=600.0).contains(&f) => {
                    self.danger_ignore_ticks = f;
                    true
                }
                _ => false,
            },
            "physics_airstates/default/LandingSpeedScalarVsGroundNormalY" => {
                curve!(self.landing_speed_vs_ny)
            }
            "physics_animation/default/LandingSketchySideSpeed" => curve!(self.land_sketchy_side),
            "physics_animation/default/LandingSketchyTwistSpin" => {
                curve!(self.land_sketchy_twist)
            }
            "physics_wipeout/default/TeleportMinTimeForAutoReset" => scalar!(self.reset_min_time),
            "physics_wipeout/default/TeleportMinTimeAfterSettlingForAutoReset" => {
                scalar!(self.reset_settled)
            }
            "physics_wipeout/default/TeleportMaxTime" => scalar!(self.reset_max_time),
            "physics_wipeout/default/TeleportAutoResetFadeOutTime" => scalar!(self.reset_fade),
            "physics_wipeout/default/WipeoutOverSpeed" => scalar!(self.settled_speed),
            "physics_wipeout/default/WipeoutOverMinTime" => scalar!(self.settled_min_time),
            "Hash_12B64C0E804B0853/default/Hash_C0526C883AF0ECCA" => scalar!(self.respawn_height),
            "Hash_12B64C0E804B0853/default/Hash_CEB092E418A5B001" => scalar!(self.respawn_radius),
            "Hash_12B64C0E804B0853/default/Hash_8ABE098D3806D273" => scalar!(self.respawn_drop),
            "Hash_12B64C0E804B0853/default/Hash_ADD032CACF6A1C15" => {
                scalar!(self.respawn_normal_y)
            }
            "Hash_018E8A2E5028AB3F/default/Hash_DAA990E803A782A1" => curve!(self.revert_speed),
            "Hash_018E8A2E5028AB3F/default/Hash_0E5B939FD4ECDBAA" => scalar!(self.revert_gain),
            "Hash_018E8A2E5028AB3F/default/Hash_3387562E9BEA0AF6" => {
                curve!(self.revert_tolerance)
            }
            "Hash_018E8A2E5028AB3F/default/Hash_035D9805EB9E2EE2" => {
                scalar!(self.revert_duration)
            }
            "anim_motion/inair_disttocog/lo_air_arm_extend" => curve!(self.air_arm_extend),
            "anim_motion/inair_disttocog/goingup_disttocog_speed" => {
                scalar!(self.air_com_up_speed)
            }
            "anim_motion/inair_disttocog/goingdown_disttocog_speed" => {
                scalar!(self.air_com_down_speed)
            }
            "anim_motion/inair_disttocog/min_inair_disttocog" => scalar!(self.air_com_min),
            "anim_motion/inair_disttocog/preland_disttocom_vel" => {
                scalar!(self.preland_com_speed)
            }
            "anim_motion/inair_disttocog/preland_final_disttocom" => scalar!(self.preland_com),
            "anim_motion/body_spin/spin_map" => curve!(self.spin_map),
            "anim_motion/body_spin/spin_influence_blendin" => scalar!(self.spin_blend_in),
            "anim_motion/body_spin/spin_influence_blendout" => scalar!(self.spin_blend_out),
            "anim_motion/body_spin/spin_clamp_influence_delta" => scalar!(self.spin_max_delta),
            "anim_motion/body_spin/spin_clamp_influence_deltadelta" => scalar!(self.spin_max_acc),
            "recognizer/right_stick/NumTicksPatternNotInRangeBeforeCulling" => match f {
                Some(f) if (0.0..=63.0).contains(&f) => {
                    self.flick_misses = f as u8;
                    true
                }
                _ => false,
            },
            _ => {
                self.grind.set(name, f, c())
                    || self.anim.set(name, f, c(), v)
                    || self.cam.set(name, f, c(), v)
                    || self.moves.set(name, f, c())
                    || self.wobble.set(name, f, c())
                    || self.biped.set(name, f, c())
                    || self.possession.set(name, f, c())
                    || self.plant.set(name, f, c(), v)
                    || self.handplant.set(name, f, c())
            }
        }
    }

    /// Steering strength for the current speed and stick (the deck tilt, 0..about 1).
    pub fn steer_tilt(&self, input: f32, speed: f32, hard: f32, push_scalar: f32) -> f32 {
        let speed_k = self
            .steer_vs_speed
            .eval((speed.abs() / self.steer_speed_range).clamp(0.0, 1.0));
        // Skate 3 multiplies by the signed turn value itself (not just its sign): half a stick tilts the
        // deck far less than half of a full one. Tighter trucks tilt less (SK8 steering.rs
        // `calculate_tilt`, its `tightness` factor).
        let tightness =
            self.tight_trucks_scalar * self.truck_tightness + 1.0 - self.truck_tightness;
        self.steer_scalar
            * tightness
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
            ("camera_tracker/distance/SpeedClamp", vec![3.25]),
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
        assert_eq!(t.cam.distance_speed, 3.25);
        assert_eq!(t.flick_misses, 4);
        assert_eq!(t.steer_vs_input, Tuning::BUILT_IN.steer_vs_input);
        assert!(parse(&data[..data.len() - 1]).is_none(), "truncated");
    }

    #[test]
    fn every_path_is_one_the_tuning_reads() {
        for p in all_paths() {
            let mut t = Tuning::BUILT_IN;
            // A two-point curve (or a four-value vector) also counts as a scalar (its first value).
            // The shake's Bezier points are a 4 x 4 matrix.
            assert!(
                t.set(p, &[0.0, 1.0, 1.0, 1.0]) || t.set(p, &[1.0]) || t.set(p, &[0.5; 16]),
                "{p}"
            );
        }
    }

    #[test]
    fn riding_animation_values_come_through_the_rules_entry() {
        let data = encode(&[
            ("anim_carving/default/quickAngleOffset", vec![0.25]),
            (
                "physics_animation/default/AnimationTurnLeanFilter",
                vec![0.5, 0.25, 0.0, 0.125],
            ),
            ("physics_animation/default/AnimationTurnGraphX", vec![12.0]),
            ("anim_motion/auto_pump/auto_pump_rise_speed", vec![0.5]),
            (
                "anim_motion/crouching/crouching_max_height",
                vec![0.0, 20.0, 0.9, 0.7],
            ),
        ]);
        let mut t = Tuning::BUILT_IN;
        assert_eq!(t.apply(&data), 5);
        assert_eq!(t.anim.quick_offset, 0.25);
        assert_eq!(t.anim.filter_lean, [0.5, 0.25, 0.0, 0.125]);
        assert_eq!(t.anim.cond[11], 12.0);
        assert_eq!(t.anim.auto_pump[2], 0.5);
        assert!((t.anim.crouch_max_height.eval(10.0) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn the_foot_plants_come_through_the_rules_entry() {
        let data = encode(&[
            (
                "Hash_CCB95A83C78B4FF9/default/Hash_6D781EFAF01E707D",
                vec![0.0, 1.0, 1.0, 2.5],
            ),
            (
                "physics_footplantmanager/default/DeckBB",
                vec![0.2, 0.05, 0.3, 0.0],
            ),
            (
                "physics_footplantmanager/default/LegLengthOnLanding",
                vec![0.9],
            ),
            (
                "physics_footplantmanager/default/Hash_68C05A7FCC6BB718",
                vec![0.3],
            ),
        ]);
        let mut t = Tuning::BUILT_IN;
        assert_eq!(t.plant.jump_up_vs_normal_y.eval(1.0), 3.0, "Skate 3's");
        assert_eq!(t.apply(&data), 4);
        assert_eq!(t.plant.jump_up_vs_normal_y.eval(1.0), 2.5);
        assert_eq!(t.plant.deck_bb, [0.2, 0.05, 0.3]);
        assert_eq!(t.plant.leg_length, 0.9);
        assert_eq!(t.plant.max_duration, 0.3);
        assert!(all_paths().any(|p| p == "physics_footplantmanager/default/DeckBB"));
    }

    #[test]
    fn the_hand_plants_come_through_the_rules_entry() {
        let data = encode(&[
            (
                "physics_handplantmanager/default/Hash_63303CA6C611DC79",
                vec![0.7],
            ),
            ("anim_handplant/default/Hash_673F3B1BD056C39E", vec![0.8]),
            (
                "Hash_55B5940075F78842/default/Hash_FF0DE8A42210E8C0",
                vec![0.0, 7.0, 3.0, 3.0],
            ),
            (
                "Hash_55B5940075F78842/default/Hash_A046D0516C0BD62B",
                vec![0.9],
            ),
        ]);
        let mut t = Tuning::BUILT_IN;
        assert_eq!(t.handplant.min_speed, 0.5, "Skate 3's");
        assert_eq!(t.handplant.anim, [0.1, 0.53, 0.75], "Skate 3's");
        assert_eq!(t.apply(&data), 4);
        assert_eq!(t.handplant.min_speed, 0.7);
        assert_eq!(t.handplant.anim[2], 0.8);
        assert_eq!(t.handplant.window[0].eval(2.0), 3.0);
        // The window's drop and the manager's apex radius share a field name in different classes.
        assert_eq!(t.handplant.window_drop, 0.9);
        assert_eq!(t.handplant.apex_radius, 0.8);
        assert!(all_paths().any(|p| p == "anim_handplant/default/Hash_B9012B86EB5258FE"));
    }

    #[test]
    fn speed_wobble_and_the_manual_mode_come_through_the_rules_entry() {
        let data = encode(&[
            ("physics_mode/normal/Hash_77AFCE78FE1206CA", vec![12.0]),
            ("physics_mode/normal/Hash_5B57F2CCCCEEF430", vec![1.0]),
            ("physics_speed_wobble/default/SpeedRange", vec![9.0]),
            (
                "physics_speed_wobble/default/AmplitudeVsSpeed",
                vec![0.0, 1.0, 0.0, 0.5],
            ),
            ("physics_mode/normal/Hash_F8CBC0F5FEF2240E", vec![0.0]),
        ]);
        let mut t = Tuning::BUILT_IN;
        assert!(
            t.manual_corrective,
            "Skate 3's Normal has the corrective force on"
        );
        assert_eq!(t.apply(&data), 5);
        assert_eq!(t.wobble.activation, 12.0);
        assert_eq!(t.wobble.amplitude_scale, 1.0);
        assert_eq!(t.wobble.speed_range, 9.0);
        assert_eq!(t.wobble.amplitude_vs_speed.eval(0.5), 0.25);
        assert!(!t.manual_corrective);
    }

    #[test]
    fn stronger_flicks_and_slower_riding_jump_higher() {
        let t = Tuning::BUILT_IN;
        assert!(t.jump_height(1.0, 3.0) > t.jump_height(0.0, 3.0));
        assert!(t.jump_height(1.0, 3.0) >= t.jump_height(1.0, 20.0));
        assert!(t.jump_height(0.0, 3.0) > t.jump_floor);
    }
}
