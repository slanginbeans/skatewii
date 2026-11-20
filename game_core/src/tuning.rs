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
    /// Length of one push cycle (s): the next push can start after this.
    pub push_cycle: f32,

    // Rolling resistance and braking.
    /// Deceleration (m/s^2) against speed, always.
    pub friction: Curve,
    /// Extra deceleration against speed once nothing has been pressed for `no_input_time`.
    pub friction_no_input: Curve,
    pub no_input_time: f32,
    /// Extra deceleration against speed on two wheels.
    pub friction_manual: Curve,
    /// Foot brake and tail brake strength (deceleration = force * `brake_scale`).
    pub foot_brake: f32,
    pub tail_brake: f32,
    pub brake_scale: f32,

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
    "physics_surfaces/smooth/FrictionVsSpeedNew",
    "physics_friction/default/FrictionVsSpeed_NoInput",
    "physics_friction/default/NoInputTime",
    "physics_friction/default/FrictionVsSpeed_Manual",
    "physics_brakes/default/FootBrakeForce",
    "physics_brakes/default/TailBrakeForce",
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
    "physics_mode/normal/JumpMinHeight",
    "physics_mode/normal/JumpMaxHeight",
    "physics_jump/default/MinHeightVsSpeed",
    "physics_jump/default/MaxHeightVsSpeed",
    "physics_jump/default/SpeedResponseMaxSpeed",
    "physics_jump/default/HippyJumpMinHeight",
    "physics_jump/default/HippyJumpMaxHeight",
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
        push_cycle: 0.6,
        friction: Curve::new(&[(0.0, 0.0), (8.0, 0.05), (30.0, 1.0)]),
        friction_no_input: Curve::new(&[(0.0, 0.1), (8.0, 0.45)]),
        no_input_time: 2.5,
        friction_manual: Curve::new(&[(0.0, 0.1), (8.0, 0.45)]),
        foot_brake: 45.0,
        tail_brake: 32.0,
        brake_scale: 0.06,
        steer_follow: 0.75,
        steer_scalar: 0.6,
        steer_vs_input: Curve::new(&[(0.0, 0.0), (0.2, 0.02), (1.0, 0.8)]),
        steer_vs_speed: Curve::new(&[(0.0, 1.0), (0.1, 0.9), (0.5, 0.35), (1.0, 0.3)]),
        steer_speed_range: 45.0,
        hard_turn_boost: 1.0,
        push_steer_drop: 0.02,
        push_steer_rise: 0.05,
        push_steer_min: 0.5,
        jump_floor: 0.9,
        jump_low: 1.3,
        jump_high: 1.7,
        jump_low_vs_speed: Curve::constant(1.0),
        jump_high_vs_speed: Curve::new(&[(0.0, 1.0), (0.5, 0.8), (1.0, 0.5)]),
        jump_speed_range: 25.0,
        hippy_low: 1.3,
        hippy_high: 1.6,
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
            "anim_motion/pushing/last_push_out_time" => scalar!(self.push_cycle),
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
        let tilt = self.steer_scalar
            * self.steer_vs_input.eval(input.abs())
            * speed_k
            * push_scalar
            * (1.0 + hard.abs() * self.hard_turn_boost);
        if input < 0.0 { -tilt } else { tilt }
    }

    /// The speed a push adds when the button was held for `held` seconds at `speed`.
    pub fn push_gain(&self, held: f32, speed: f32) -> f32 {
        let full = self.push_hold_time.eval(speed).max(0.01);
        self.push_dv.eval((held / full).clamp(0.0, 1.0))
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
