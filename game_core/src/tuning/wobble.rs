//! Speed wobble tuning: Skate 3's `physics_speed_wobble` group and the Normal physics mode's wobble
//! onset and strength (SK8 `skate-game/src/physics/ground_runtime/settings.rs`, `SpeedWobbleSettings`
//! and `wobble_activation` / `wobble_amplitude`). The built-in values are Skate 3's stock ones (read off
//! the disc's `skatercollections` with `fg_import values`); the rules pack carries the same paths.

use super::Curve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WobbleTuning {
    /// Frequency and amplitude shares against the wobble's time (/ `time_max`) and its speed over the
    /// onset (/ `speed_range`), all 0..1.
    pub frequency_vs_time: Curve,
    pub frequency_vs_speed: Curve,
    pub amplitude_vs_time: Curve,
    pub amplitude_vs_speed: Curve,
    /// Onset raised by this x the truck tightness (0..1) and by this x the crouch (0..1), m/s.
    pub tightness_speed: f32,
    pub crouch_speed: f32,
    /// Seconds and m/s over the onset that the curves span.
    pub time_max: f32,
    pub speed_range: f32,
    /// Highest frequency (Hz) and amplitude (deck tilt).
    pub max_frequency: f32,
    pub max_amplitude: f32,
    /// The centre of mass over the board (m) that counts as fully crouched and fully standing.
    pub crouch_min: f32,
    pub crouch_max: f32,
    /// The physics mode's onset (m/s, `Hash_77AFCE78FE1206CA`) and strength (`Hash_5B57F2CCCCEEF430`).
    pub activation: f32,
    pub amplitude_scale: f32,
}

impl WobbleTuning {
    /// Skate 3's stock values (`physics_speed_wobble/default`, `physics_mode/normal`).
    pub const BUILT_IN: WobbleTuning = WobbleTuning {
        frequency_vs_time: Curve::new(&[
            (0.0, 1.0),
            (0.213_470_3, 1.0),
            (0.296_803_7, 0.645_270_2),
            (0.375_570_8, 0.996_621_6),
            (0.505_707_7, 1.0),
            (0.756_849_3, 1.0),
            (0.811_643_8, 0.408_783_8),
            (0.859_589, 1.0),
        ]),
        frequency_vs_speed: Curve::new(&[
            (0.0, 1.0),
            (0.090_182_65, 1.0),
            (0.164_383_6, 0.591_216_2),
            (0.267_123_3, 0.587_837_8),
            (0.393_835_6, 0.888_513_5),
            (0.517_123_3, 0.885_135_1),
            (0.809_360_7, 0.689_189_2),
            (0.917_808_2, 0.888_513_5),
        ]),
        amplitude_vs_time: Curve::new(&[
            (0.0, 0.0),
            (0.042_237_44, 1.0),
            (0.381_278_5, 1.0),
            (0.408_675_8, 0.591_216_2),
            (0.602_739_8, 1.0),
            (0.891_552_5, 1.0),
            (0.906_392_7, 0.679_054),
            (0.973_744_3, 1.0),
        ]),
        amplitude_vs_speed: Curve::new(&[
            (0.001_141_552, 0.0),
            (0.182_410_4, 0.057_432_41),
            (0.327_361_6, 0.175_675_7),
            (0.486_970_7, 0.418_918_9),
            (0.630_293_1, 0.625),
            (0.756_849_3, 0.746_621_6),
            (0.876_221_5, 0.922_297_3),
            (1.0, 1.0),
        ]),
        tightness_speed: 5.0,
        crouch_speed: 3.0,
        time_max: 20.0,
        speed_range: 18.0,
        max_frequency: 6.0,
        max_amplitude: 0.3,
        crouch_min: 0.5,
        crouch_max: 0.9,
        activation: super::ModeTuning::NORMAL.wobble_onset,
        amplitude_scale: super::ModeTuning::NORMAL.wobble_amplitude,
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
        match name {
            "physics_speed_wobble/default/FrequencyVsTime" => curve!(self.frequency_vs_time),
            "physics_speed_wobble/default/FrequencyVsSpeed" => curve!(self.frequency_vs_speed),
            "physics_speed_wobble/default/AmplitudeVsTime" => curve!(self.amplitude_vs_time),
            "physics_speed_wobble/default/AmplitudeVsSpeed" => curve!(self.amplitude_vs_speed),
            "physics_speed_wobble/default/TruckTightnessSpeedMaxAdjust" => {
                scalar!(self.tightness_speed)
            }
            "physics_speed_wobble/default/CrouchStartSpeedMaxAdjust" => {
                scalar!(self.crouch_speed)
            }
            "physics_speed_wobble/default/TimeMax" => scalar!(self.time_max),
            "physics_speed_wobble/default/SpeedRange" => scalar!(self.speed_range),
            "physics_speed_wobble/default/MaxFrequency" => scalar!(self.max_frequency),
            "physics_speed_wobble/default/MaxAmplitude" => scalar!(self.max_amplitude),
            "physics_speed_wobble/default/CrouchMin" => scalar!(self.crouch_min),
            "physics_speed_wobble/default/CrouchMax" => scalar!(self.crouch_max),
            _ => false,
        }
    }
}

/// Where each value lives in Skate 3's tuning database (`class/key/field`).
pub const PATHS: &[&str] = &[
    "physics_speed_wobble/default/FrequencyVsTime",
    "physics_speed_wobble/default/FrequencyVsSpeed",
    "physics_speed_wobble/default/AmplitudeVsTime",
    "physics_speed_wobble/default/AmplitudeVsSpeed",
    "physics_speed_wobble/default/TruckTightnessSpeedMaxAdjust",
    "physics_speed_wobble/default/CrouchStartSpeedMaxAdjust",
    "physics_speed_wobble/default/TimeMax",
    "physics_speed_wobble/default/SpeedRange",
    "physics_speed_wobble/default/MaxFrequency",
    "physics_speed_wobble/default/MaxAmplitude",
    "physics_speed_wobble/default/CrouchMin",
    "physics_speed_wobble/default/CrouchMax",
];
