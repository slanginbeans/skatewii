//! Camera tuning: the numbers Skate 3's gameplay camera reads from its tuning database
//! (`camera_tracker`, `camera/dynamics`, `camera/rotation`, `camera/subject_tracker`, `camera_compass`,
//! `camera_positioner`, `camera_droppredictor`, `camera_lag_profile`, `camera/freecam`, `camera_shake`,
//! `slowmotion_controller`). The player's disc supplies Skate 3's through the rules pack
//! (`fg_import rules`); the free look, shake and slow motion built-ins are Skate 3's own numbers (read
//! off the disc). The shots themselves are a separate entry (`camera::shot`).

use super::Curve;

/// Skate 3's camera shake (`camera_shake/default`, SK8 camera/shake.rs `ShakeSettings`): a sway that
/// grows with speed, plus a one-shot impulse on landings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShakeTuning {
    /// Bezier control points (x, y in the first two lanes) mapping speed over the top speed to the
    /// amplitude's and the frequency's share between their minimum and maximum.
    pub amplitude_curve: [[f32; 4]; 4],
    pub frequency_curve: [[f32; 4]; 4],
    /// The landing impulse: its most, the landing speeds it grows between, its frequency, its decay a
    /// tick.
    pub impulse_magnitude: f32,
    pub impulse_min_speed: f32,
    pub impulse_max_speed: f32,
    pub impulse_frequency: f32,
    pub impulse_decay: f32,
    pub amplitude_min: f32,
    pub amplitude_max: f32,
    pub amplitude_top_speed: f32,
    pub frequency_min: f32,
    pub frequency_max: f32,
    pub frequency_top_speed: f32,
    /// Samples a second at frequency 1.
    pub samples_per_second: f32,
    pub translation: f32,
    pub rotation: f32,
    /// The roll's share of the rotation.
    pub dutch: f32,
}

impl ShakeTuning {
    /// Skate 3's `camera_shake/default`.
    pub const BUILT_IN: ShakeTuning = ShakeTuning {
        amplitude_curve: [
            [0.991_666_7, 1.0, 0.0, 0.0],
            [0.658_333_4, 1.034_483, 0.0, 0.0],
            [1.141_667, -0.103_448_3, 0.0, 0.0],
            [-0.008_333_334, 0.0, 0.0, 0.0],
        ],
        frequency_curve: [
            [1.0, 1.0, 0.0, 0.0],
            [0.7, 0.913_793_1, 0.0, 0.0],
            [0.883_333_3, -0.068_965_55, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
        ],
        impulse_magnitude: 0.5,
        impulse_min_speed: 0.0,
        impulse_max_speed: 0.0,
        impulse_frequency: 0.5,
        impulse_decay: 0.02,
        amplitude_min: 0.3,
        amplitude_max: 6.0,
        amplitude_top_speed: 26.0,
        frequency_min: 0.02,
        frequency_max: 0.5,
        frequency_top_speed: 20.0,
        samples_per_second: 90.0,
        translation: 0.05,
        rotation: 4.0,
        dutch: 0.1,
    };

    fn set(&mut self, field: &str, f: Option<f32>, v: &[f32]) -> bool {
        let matrix = |m: &mut [[f32; 4]; 4]| {
            if v.len() != 16 || v.iter().any(|x| !x.is_finite()) {
                return false;
            }
            for (i, x) in v.iter().enumerate() {
                m[i / 4][i % 4] = *x;
            }
            true
        };
        let target = match field {
            "AmplitudeCurve" => return matrix(&mut self.amplitude_curve),
            "FrequencyCurve" => return matrix(&mut self.frequency_curve),
            "OneShotMagnitude" => &mut self.impulse_magnitude,
            "OneShotMinVelocity" => &mut self.impulse_min_speed,
            "OneShotMaxVelocity" => &mut self.impulse_max_speed,
            "OneShotFrequency" => &mut self.impulse_frequency,
            "OneShotDecay" => &mut self.impulse_decay,
            "AmplitudeMin" => &mut self.amplitude_min,
            "AmplitudeMax" => &mut self.amplitude_max,
            "AmplitudeTopSkaterSpeed" => &mut self.amplitude_top_speed,
            "FrequencyMin" => &mut self.frequency_min,
            "FrequencyMax" => &mut self.frequency_max,
            "FrequencyTopSkaterSpeed" => &mut self.frequency_top_speed,
            "DataFPS" => &mut self.samples_per_second,
            "TranslationMultiplier" => &mut self.translation,
            "RotationMultiplier" => &mut self.rotation,
            "DutchMultiplier" => &mut self.dutch,
            _ => return false,
        };
        match f {
            Some(f) => {
                *target = f;
                true
            }
            None => false,
        }
    }
}

/// An angle tracker (degrees): speed clamp, acceleration clamps far from and near the target, and the
/// error band between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AngleTrack {
    pub speed: f32,
    pub accel_far: f32,
    pub accel_near: f32,
    pub umbra: f32,
    pub penumbra: f32,
    /// The least smoothing (pan and tilt only).
    pub smoothing_min: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamTuning {
    pub heading: AngleTrack,
    pub elevation: AngleTrack,
    pub pan: AngleTrack,
    pub tilt: AngleTrack,
    /// The boom length's tracker (m/s, m/s^2) and smoothing.
    pub distance_speed: f32,
    pub distance_accel: f32,
    pub distance_smoothing: f32,
    /// Closest the camera comes (m), and during a wipeout.
    pub min_distance: f32,
    pub min_distance_wipeout: f32,
    /// Avoidance: look-ahead (s), heading and elevation smoothing, ease-out time (s), path padding (m).
    pub avoid_horizon: f32,
    pub avoid_heading_smoothing: f32,
    pub avoid_elevation_smoothing: f32,
    pub avoid_ease_out: f32,
    pub avoid_padding: f32,
    /// Seconds a collision is held.
    pub collision_hold: f32,
    /// Framing roll's speed (1/s).
    pub dutch_speed: f32,
    /// How much of the way the movement compass turns to the travel direction a tick.
    pub heading_response: f32,
    /// Highest the camera looks up (deg).
    pub max_pitch: f32,
    /// Pumping harder than this lags the camera.
    pub pump_min_accel: f32,
    /// Steering at least this hard counts as turning (not centred).
    pub centred_deadzone: f32,
    /// Pan: error band (deg), smoothing against nearness, least smoothing.
    pub pan_umbra: f32,
    pub pan_penumbra: f32,
    pub pan_curve: Curve,
    pub pan_min_smoothing: f32,
    /// The anchor's tracker.
    pub subject_speed: f32,
    pub subject_accel: f32,
    /// Movement compass: stopped this long (s), it lines up behind the board this much a tick.
    pub lineup_time: f32,
    pub lineup_speed: f32,
    pub deadzone_min_speed: f32,
    pub deadzone_max_speed: f32,
    pub deadzone_size: f32,
    pub deadzone_smoothing: f32,
    /// Drop predictor: shortest test (m), look-ahead time (s), longest test (m), deepest drop (m).
    pub drop_min_test: f32,
    pub drop_test_time: f32,
    pub drop_max_test: f32,
    pub drop_max_depth: f32,
    /// Push and pump lag: smoothing against time since (0..1 over half a second), and its most.
    pub push_curve: Curve,
    pub push_max: f32,
    pub pump_curve: Curve,
    pub pump_max: f32,
    /// Free look (`camera/freecam`, SK8 look_input.rs): the stick's full deflection turns the arm this
    /// far (deg), and the offset closes on that this much a second (times 1/60 a tick).
    pub look_heading_offset: f32,
    pub look_heading_speed: f32,
    pub look_elevation_offset: f32,
    pub look_elevation_speed: f32,
    pub shake: ShakeTuning,
    /// Slow motion (`slowmotion_controller/default`, SK8 slow_motion.rs): the share of the way from 60
    /// to `slow_fps` simulation ticks a second against the air's progress.
    pub slow_curve: Curve,
    pub slow_fps: f32,
}

impl CamTuning {
    pub const BUILT_IN: CamTuning = CamTuning {
        heading: AngleTrack {
            speed: 720.0,
            accel_far: 900.0,
            accel_near: 270.0,
            umbra: 0.0,
            penumbra: 32.0,
            smoothing_min: 0.0,
        },
        elevation: AngleTrack {
            speed: 1200.0,
            accel_far: 720.0,
            accel_near: 20.0,
            umbra: 0.0,
            penumbra: 28.0,
            smoothing_min: 0.0,
        },
        pan: AngleTrack {
            speed: 1800.0,
            accel_far: 900.0,
            accel_near: 900.0,
            umbra: 0.0,
            penumbra: 28.0,
            smoothing_min: 0.45,
        },
        tilt: AngleTrack {
            speed: 1800.0,
            accel_far: 1800.0,
            accel_near: 270.0,
            umbra: 0.0,
            penumbra: 32.0,
            smoothing_min: 0.35,
        },
        distance_speed: 6.5,
        distance_accel: 9.0,
        distance_smoothing: 0.82,
        min_distance: 0.55,
        min_distance_wipeout: 1.4,
        avoid_horizon: 0.9,
        avoid_heading_smoothing: 0.75,
        avoid_elevation_smoothing: 0.1,
        avoid_ease_out: 1.2,
        avoid_padding: 0.0,
        collision_hold: 1.1,
        dutch_speed: 5.5,
        heading_response: 0.85,
        max_pitch: 32.0,
        pump_min_accel: 0.025,
        centred_deadzone: 0.85,
        pan_umbra: 0.0,
        pan_penumbra: 24.0,
        pan_curve: Curve::new(&[
            (0.0, 0.0),
            (0.25, 0.1),
            (0.5, 0.45),
            (0.75, 0.85),
            (1.0, 1.0),
        ]),
        pan_min_smoothing: 0.65,
        subject_speed: 35.0,
        subject_accel: 9000.0,
        lineup_time: 0.08,
        lineup_speed: 0.45,
        deadzone_min_speed: 0.55,
        deadzone_max_speed: 0.75,
        deadzone_size: 0.0,
        deadzone_smoothing: 0.0,
        drop_min_test: 0.9,
        drop_test_time: 1.8,
        drop_max_test: 6.5,
        drop_max_depth: 5.5,
        push_curve: Curve::new(&[(0.0, 1.0), (0.3, 0.85), (0.6, 0.4), (1.0, 0.02)]),
        push_max: 0.75,
        pump_curve: Curve::new(&[(0.0, 1.0), (0.4, 0.8), (0.75, 0.45), (1.0, 0.02)]),
        pump_max: 0.85,
        // Skate 3's `camera/freecam`: no heading, up to 30 degrees of elevation.
        look_heading_offset: 0.0,
        look_heading_speed: 1.0,
        look_elevation_offset: 30.0,
        look_elevation_speed: 2.0,
        shake: ShakeTuning::BUILT_IN,
        // Skate 3's `slowmotion_controller/default`: down to 35 ticks a second a fifth of the way
        // through the air, back to 60 by half way.
        slow_curve: Curve::new(&[
            (0.001_007_049, 0.0),
            (0.033_232_63, 0.228_682_2),
            (0.061_430_01, 0.457_364_3),
            (0.097_683_79, 0.709_302_3),
            (0.143_001, 0.945_736_4),
            (0.199_395_8, 1.0),
            (0.253_776_4, 0.957_364_3),
            (0.311_178_2, 0.736_434_1),
            (0.369_587_1, 0.461_240_3),
            (0.445_115_8, 0.205_426_3),
            (0.547_834_9, 0.027_131_8),
            (0.710_976_8, 0.0),
            (0.845_921_5, 0.0),
            (0.907_351_4, 0.0),
            (0.950_654_6, 0.0),
            (1.0, 0.0),
        ]),
        slow_fps: 35.0,
    };

    /// Takes over a value from the rules pack (`class/key/field`; `v`: all its numbers). Returns whether
    /// it was one of these.
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
        if let Some(field) = name.strip_prefix("camera_shake/default/") {
            return self.shake.set(field, f, v);
        }
        if let Some(rest) = name.strip_prefix("camera_tracker/") {
            let (key, field) = rest.split_once('/').unwrap_or((rest, ""));
            let t = match key {
                "heading" => &mut self.heading,
                "elevation" => &mut self.elevation,
                "pan" => &mut self.pan,
                "tilt" => &mut self.tilt,
                "distance" => {
                    return match field {
                        "SpeedClamp" => scalar!(self.distance_speed),
                        "AccelerationClampMax" => scalar!(self.distance_accel),
                        "SmoothingConstant" => scalar!(self.distance_smoothing),
                        _ => false,
                    };
                }
                _ => return false,
            };
            return match field {
                "SpeedClamp" => scalar!(t.speed),
                "AccelerationClampMin" => scalar!(t.accel_far),
                "AccelerationClampMax" => scalar!(t.accel_near),
                "DeltaUmbra" => scalar!(t.umbra),
                "DeltaPenumbra" => scalar!(t.penumbra),
                "SmoothingMin" => scalar!(t.smoothing_min),
                _ => false,
            };
        }
        match name {
            "camera_positioner/default/MinDistance" => scalar!(self.min_distance),
            "camera_positioner/default/MinDistanceDuringWipeout" => {
                scalar!(self.min_distance_wipeout)
            }
            "camera_positioner/default/AvoidanceCollisionOffset" => scalar!(self.avoid_padding),
            "camera/dynamics/AvoidanceLookahead" => scalar!(self.avoid_horizon),
            "camera/dynamics/AvoidanceHeadingSmoothing" => scalar!(self.avoid_heading_smoothing),
            "camera/dynamics/AvoidanceElevationSmoothing" => {
                scalar!(self.avoid_elevation_smoothing)
            }
            "camera/dynamics/AvoidanceEaseOut" => scalar!(self.avoid_ease_out),
            "camera/dynamics/CollisionInterpolationDuration" => scalar!(self.collision_hold),
            "camera/dynamics/DutchInterpolationSpeed" => scalar!(self.dutch_speed),
            "camera/dynamics/HeadingChangeResponseSpeed" => scalar!(self.heading_response),
            "camera/dynamics/MaxPitch" => scalar!(self.max_pitch),
            "camera/dynamics/PumpingMinAcceleration" => scalar!(self.pump_min_accel),
            "camera/dynamics/TurningCentredDeadzone" => scalar!(self.centred_deadzone),
            "camera/rotation/PanUmbra" => scalar!(self.pan_umbra),
            "camera/rotation/PanPenumbra" => scalar!(self.pan_penumbra),
            "camera/rotation/PanSmoothingCurve" => curve!(self.pan_curve),
            "camera/rotation/PanMinSmoothing" => scalar!(self.pan_min_smoothing),
            "camera/subject_tracker/SubjectTrackerMaxSpeed" => scalar!(self.subject_speed),
            "camera/subject_tracker/SubjectTrackerMaxAcceleration" => scalar!(self.subject_accel),
            "camera_compass/default/TimeBeforeAutoLineup" => scalar!(self.lineup_time),
            "camera_compass/default/AutoLineupSpeed" => scalar!(self.lineup_speed),
            "camera_compass/default/MinDeadzoneSpeed" => scalar!(self.deadzone_min_speed),
            "camera_compass/default/MaxDeadzoneSpeed" => scalar!(self.deadzone_max_speed),
            "camera_compass/default/MaxDeadzoneSize" => scalar!(self.deadzone_size),
            "camera_compass/default/DeadzoneSizeSmoothing" => scalar!(self.deadzone_smoothing),
            "camera_droppredictor/default/MinTestDistance" => scalar!(self.drop_min_test),
            "camera_droppredictor/default/TotalTestTime" => scalar!(self.drop_test_time),
            "camera_droppredictor/default/Hash_FB5A1024D47298A1" => scalar!(self.drop_max_test),
            "camera_droppredictor/default/MaxDropDistance" => scalar!(self.drop_max_depth),
            "camera_lag_profile/push/Curve" => curve!(self.push_curve),
            "camera_lag_profile/push/MaxSmoothing" => scalar!(self.push_max),
            "camera_lag_profile/pump/Curve" => curve!(self.pump_curve),
            "camera_lag_profile/pump/MaxSmoothing" => scalar!(self.pump_max),
            "camera/freecam/FreeCamHeadingOffset" => scalar!(self.look_heading_offset),
            "camera/freecam/FreeCamHeadingSpeed" => scalar!(self.look_heading_speed),
            "camera/freecam/FreeCamElevationOffset" => scalar!(self.look_elevation_offset),
            "camera/freecam/FreeCamElevationSpeed" => scalar!(self.look_elevation_speed),
            "slowmotion_controller/default/timescale" => curve!(self.slow_curve),
            "slowmotion_controller/default/fps_at_scale_one" => match f {
                Some(f) if (1.0..=60.0).contains(&f) => {
                    self.slow_fps = f;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }
}

/// Where each value lives in Skate 3's tuning database (`class/key/field`).
pub const PATHS: &[&str] = &[
    "camera_tracker/heading/SpeedClamp",
    "camera_tracker/heading/AccelerationClampMin",
    "camera_tracker/heading/AccelerationClampMax",
    "camera_tracker/heading/DeltaUmbra",
    "camera_tracker/heading/DeltaPenumbra",
    "camera_tracker/elevation/SpeedClamp",
    "camera_tracker/elevation/AccelerationClampMin",
    "camera_tracker/elevation/AccelerationClampMax",
    "camera_tracker/elevation/DeltaUmbra",
    "camera_tracker/elevation/DeltaPenumbra",
    "camera_tracker/pan/AccelerationClampMin",
    "camera_tracker/pan/AccelerationClampMax",
    "camera_tracker/pan/SmoothingMin",
    "camera_tracker/pan/DeltaUmbra",
    "camera_tracker/pan/DeltaPenumbra",
    "camera_tracker/tilt/AccelerationClampMin",
    "camera_tracker/tilt/AccelerationClampMax",
    "camera_tracker/tilt/SmoothingMin",
    "camera_tracker/tilt/DeltaUmbra",
    "camera_tracker/tilt/DeltaPenumbra",
    "camera_tracker/distance/SpeedClamp",
    "camera_tracker/distance/AccelerationClampMax",
    "camera_tracker/distance/SmoothingConstant",
    "camera_positioner/default/MinDistance",
    "camera_positioner/default/MinDistanceDuringWipeout",
    "camera_positioner/default/AvoidanceCollisionOffset",
    "camera/dynamics/AvoidanceLookahead",
    "camera/dynamics/AvoidanceHeadingSmoothing",
    "camera/dynamics/AvoidanceElevationSmoothing",
    "camera/dynamics/AvoidanceEaseOut",
    "camera/dynamics/CollisionInterpolationDuration",
    "camera/dynamics/DutchInterpolationSpeed",
    "camera/dynamics/HeadingChangeResponseSpeed",
    "camera/dynamics/MaxPitch",
    "camera/dynamics/PumpingMinAcceleration",
    "camera/dynamics/TurningCentredDeadzone",
    "camera/rotation/PanUmbra",
    "camera/rotation/PanPenumbra",
    "camera/rotation/PanSmoothingCurve",
    "camera/rotation/PanMinSmoothing",
    "camera/subject_tracker/SubjectTrackerMaxSpeed",
    "camera/subject_tracker/SubjectTrackerMaxAcceleration",
    "camera_compass/default/TimeBeforeAutoLineup",
    "camera_compass/default/AutoLineupSpeed",
    "camera_compass/default/MinDeadzoneSpeed",
    "camera_compass/default/MaxDeadzoneSpeed",
    "camera_compass/default/MaxDeadzoneSize",
    "camera_compass/default/DeadzoneSizeSmoothing",
    "camera_droppredictor/default/MinTestDistance",
    "camera_droppredictor/default/TotalTestTime",
    "camera_droppredictor/default/Hash_FB5A1024D47298A1",
    "camera_droppredictor/default/MaxDropDistance",
    "camera_lag_profile/push/Curve",
    "camera_lag_profile/push/MaxSmoothing",
    "camera_lag_profile/pump/Curve",
    "camera_lag_profile/pump/MaxSmoothing",
    "camera/freecam/FreeCamHeadingOffset",
    "camera/freecam/FreeCamHeadingSpeed",
    "camera/freecam/FreeCamElevationOffset",
    "camera/freecam/FreeCamElevationSpeed",
    "camera_shake/default/AmplitudeCurve",
    "camera_shake/default/FrequencyCurve",
    "camera_shake/default/OneShotMagnitude",
    "camera_shake/default/OneShotMinVelocity",
    "camera_shake/default/OneShotMaxVelocity",
    "camera_shake/default/OneShotFrequency",
    "camera_shake/default/OneShotDecay",
    "camera_shake/default/AmplitudeMin",
    "camera_shake/default/AmplitudeMax",
    "camera_shake/default/AmplitudeTopSkaterSpeed",
    "camera_shake/default/FrequencyMin",
    "camera_shake/default/FrequencyMax",
    "camera_shake/default/FrequencyTopSkaterSpeed",
    "camera_shake/default/DataFPS",
    "camera_shake/default/TranslationMultiplier",
    "camera_shake/default/RotationMultiplier",
    "camera_shake/default/DutchMultiplier",
    "slowmotion_controller/default/timescale",
    "slowmotion_controller/default/fps_at_scale_one",
];
