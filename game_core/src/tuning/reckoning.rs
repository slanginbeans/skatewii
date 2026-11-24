//! The ground reckoning's tuning: Skate 3's `physics_reckoning` up-vector and dynamic-normal fields
//! (SK8 skate-game/src/physics/riding_outputs.rs `GroundOrientationSettings`,
//! player_input/dynamic_normal.rs `DynamicNormalSettings`). The built-in values are Skate 3's stock ones
//! (read off the disc with `fg_import values`); the rules pack carries the same paths.

use super::Curve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReckoningTuning {
    /// The vector filters' controls (gain, error-blend gain, derivative gain, blend): the ground's
    /// normal, and the up's slow and fast followers.
    pub ground_normal_smoothing: [f32; 4],
    pub up_smoothing_slow: [f32; 4],
    pub up_smoothing_fast: [f32; 4],
    /// How much the rider's up predicts from the centre of mass against the ground normal's y, and the
    /// ground's share of the target against its slope (quarter turns).
    pub dynamic_up_vs_ground_y: Curve,
    pub ground_vector_blend: Curve,
    /// The deck's pitch taken into the up against speed (m/s).
    pub deck_angle_usage_vs_speed: Curve,
    /// The fast follower's share and the up's most change a tick (x 0.2) against speed / 10.
    pub up_smoothing_vs_speed: Curve,
    pub up_max_delta_vs_speed: Curve,
    pub ground_blend_max_delta: f32,
    /// The up's most acceleration (x 1/60 a tick), the damping when it overshoots, and the damping of
    /// its sideways part.
    pub up_max_acceleration: f32,
    pub anti_wobble_damping: f32,
    pub extra_side_damping: f32,
    /// The ground's share needs more wheels down than this.
    pub min_wheels_for_ground_blend: i32,
    /// The dynamic normal (Skate 3's Ground64, from the wheels' acceleration against gravity): its
    /// step's damping, the share of the error a tick, and the most a tick (x the curve against
    /// speed / `dynamic_speed_scale`).
    pub dynamic_speed_damping: f32,
    pub dynamic_up_damping: f32,
    pub dynamic_max_delta: f32,
    pub dynamic_speed_scale: f32,
    pub dynamic_max_delta_vs_speed: Curve,
}

impl ReckoningTuning {
    /// Skate 3's stock values (`physics_reckoning/default`).
    pub const BUILT_IN: ReckoningTuning = ReckoningTuning {
        ground_normal_smoothing: [0.2, 0.2, 0.05, 0.1],
        up_smoothing_slow: [0.04, 0.04, 0.2, 0.2],
        up_smoothing_fast: [0.2, 0.4, 0.2, 0.2],
        dynamic_up_vs_ground_y: Curve::new(&[
            (0.0, 0.0),
            (0.226_384_4, 0.0),
            (0.384_364_8, 0.141_891_9),
            (0.469_055_4, 0.307_432_4),
            (0.537_459_3, 0.516_891_9),
            (0.594_462_5, 0.868_243_2),
            (0.654_723_1, 1.0),
            (1.0, 1.0),
        ]),
        ground_vector_blend: Curve::new(&[
            (0.001_628_664, 0.003_378_391),
            (0.135_179_1, 0.0),
            (0.258_957_7, 0.388_513_5),
            (0.369_706_8, 0.662_162_2),
            (0.486_970_7, 0.861_486_5),
            (0.560_260_6, 0.918_918_9),
            (0.623_778_5, 0.945_945_9),
            (1.0, 1.0),
        ]),
        deck_angle_usage_vs_speed: Curve::new(&[
            (0.0, 0.0),
            (1.5, 0.0),
            (3.0, 0.0),
            (4.317_391, 0.046_428_57),
            (6.091_305, 0.175),
            (7.8, 0.296_428_6),
            (9.704_348, 0.417_857_1),
            (12.0, 0.55),
        ]),
        up_smoothing_vs_speed: Curve::new(&[
            (0.0, 0.0),
            (0.125, 0.0),
            (0.262_215, 0.040_540_52),
            (0.374_592_8, 0.158_783_8),
            (0.596_091_2, 0.418_918_9),
            (0.741_042_3, 0.743_243_2),
            (0.814_332_2, 1.0),
            (1.0, 1.0),
        ]),
        up_max_delta_vs_speed: Curve::new(&[
            (0.0, 0.564_189_2),
            (0.110_749_2, 0.618_243_2),
            (0.229_641_7, 0.682_432_4),
            (0.368_078_2, 0.777_027),
            (0.542_345_3, 0.875),
            (0.705_211_7, 0.949_324_3),
            (0.902_280_2, 1.0),
            (1.0, 1.0),
        ]),
        ground_blend_max_delta: 0.1,
        up_max_acceleration: 2.0,
        anti_wobble_damping: 0.5,
        extra_side_damping: 0.6,
        min_wheels_for_ground_blend: 0,
        dynamic_speed_damping: 0.5,
        dynamic_up_damping: 0.2,
        dynamic_max_delta: 0.15,
        dynamic_speed_scale: 24.0,
        dynamic_max_delta_vs_speed: Curve::new(&[
            (0.0, 0.070_945_92),
            (0.174_267_1, 0.236_486_5),
            (0.325_732_9, 0.489_864_9),
            (0.452_768_7, 0.915_540_5),
            (0.514_658, 1.0),
            (0.615_635_2, 0.817_567_6),
            (0.755_700_3, 0.239_864_9),
            (1.0, 0.060_810_8),
        ]),
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
        macro_rules! control {
            ($field:expr) => {
                match v {
                    [a, b, c, d, ..] => {
                        $field = [*a, *b, *c, *d];
                        true
                    }
                    _ => false,
                }
            };
        }
        match name {
            "physics_reckoning/default/GroundNormalSmoothing" => {
                control!(self.ground_normal_smoothing)
            }
            "physics_reckoning/default/UpVectorSmoothingSlow" => control!(self.up_smoothing_slow),
            "physics_reckoning/default/UpVectorSmoothingFast" => control!(self.up_smoothing_fast),
            "physics_reckoning/default/DynamicUpVsGroundY" => curve!(self.dynamic_up_vs_ground_y),
            "physics_reckoning/default/GroundVectorBlendGraph" => curve!(self.ground_vector_blend),
            "physics_reckoning/default/DeckAngleUsageVsSpeed" => {
                curve!(self.deck_angle_usage_vs_speed)
            }
            "physics_reckoning/default/UpVectorSmoothingVsSpeed" => {
                curve!(self.up_smoothing_vs_speed)
            }
            "physics_reckoning/default/UpVectorMaxDeltaVsSpeed" => {
                curve!(self.up_max_delta_vs_speed)
            }
            "physics_reckoning/default/GroundVMaxBlendDelta" => {
                scalar!(self.ground_blend_max_delta)
            }
            "physics_reckoning/default/UpVectorMaxAcceleration" => {
                scalar!(self.up_max_acceleration)
            }
            "physics_reckoning/default/UpVectAntiWobbleDamping" => {
                scalar!(self.anti_wobble_damping)
            }
            "physics_reckoning/default/ExtraSideDamping" => scalar!(self.extra_side_damping),
            // An integer in the database: a 4-byte one arrives as its bits read as a float.
            "physics_reckoning/default/MinWheelsToUseGroundVector" => match f {
                Some(f) if f.to_bits() < 0x0080_0000 => {
                    self.min_wheels_for_ground_blend = f.to_bits() as i32;
                    true
                }
                Some(f) if (0.0..=8.0).contains(&f) => {
                    self.min_wheels_for_ground_blend = f as i32;
                    true
                }
                _ => false,
            },
            "physics_reckoning/default/DynamicSpeedDamping" => {
                scalar!(self.dynamic_speed_damping)
            }
            "physics_reckoning/default/DynamicUpVectorDamping" => scalar!(self.dynamic_up_damping),
            "physics_reckoning/default/DynamicSpeedMaxDelta" => scalar!(self.dynamic_max_delta),
            "physics_reckoning/default/DynamicSpeedMaxDeltaXScale" => {
                scalar!(self.dynamic_speed_scale)
            }
            "physics_reckoning/default/DynamicSpeedMaxDeltaGraphZ" => {
                curve!(self.dynamic_max_delta_vs_speed)
            }
            _ => false,
        }
    }
}

/// Where each value lives in Skate 3's tuning database (`class/key/field`). `GroundNormalSmoothing` is
/// in the main list (the drawn board's smoothing reads its first value too).
pub const PATHS: &[&str] = &[
    "physics_reckoning/default/UpVectorSmoothingSlow",
    "physics_reckoning/default/UpVectorSmoothingFast",
    "physics_reckoning/default/DynamicUpVsGroundY",
    "physics_reckoning/default/GroundVectorBlendGraph",
    "physics_reckoning/default/DeckAngleUsageVsSpeed",
    "physics_reckoning/default/UpVectorSmoothingVsSpeed",
    "physics_reckoning/default/UpVectorMaxDeltaVsSpeed",
    "physics_reckoning/default/GroundVMaxBlendDelta",
    "physics_reckoning/default/UpVectorMaxAcceleration",
    "physics_reckoning/default/UpVectAntiWobbleDamping",
    "physics_reckoning/default/ExtraSideDamping",
    "physics_reckoning/default/MinWheelsToUseGroundVector",
    "physics_reckoning/default/DynamicSpeedDamping",
    "physics_reckoning/default/DynamicUpVectorDamping",
    "physics_reckoning/default/DynamicSpeedMaxDelta",
    "physics_reckoning/default/DynamicSpeedMaxDeltaXScale",
    "physics_reckoning/default/DynamicSpeedMaxDeltaGraphZ",
];
