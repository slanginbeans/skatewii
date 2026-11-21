//! Camera tuning: the numbers Skate 3's gameplay camera reads from its tuning database
//! (`camera_tracker`, `camera/dynamics`, `camera/rotation`, `camera/subject_tracker`, `camera_compass`,
//! `camera_positioner`, `camera_droppredictor`, `camera_lag_profile`). The built-in values are
//! flatground's own; the player's disc supplies Skate 3's through the rules pack (`fg_import rules`).
//! The shots themselves are a separate entry (`camera::shot`).

use super::Curve;

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
    };

    /// Takes over a value from the rules pack (`class/key/field`). Returns whether it was one of these.
    pub(super) fn set(&mut self, name: &str, f: Option<f32>, c: Option<Curve>) -> bool {
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
];
