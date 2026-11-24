//! Free look: the look stick's offset on the camera's arm, after Skate 3's (SK8-ENGINE/skate-3-rust-engine,
//! GPL-3.0, crates/skate-core/src/camera/look_input.rs, the gameplay camera's `FreeCamManager`).
//!
//! Each tick the stick (each axis -1..1) asks for an offset of up to `FreeCamHeadingOffset` /
//! `FreeCamElevationOffset` degrees (at most 85 degrees either way), and the offset closes on it by its
//! speed times 1/60 of the way. The camera only reads the stick in shots that allow it (Skate 3's
//! `OptionUseFreeCamStick`: on foot on the ground and the drop-in); elsewhere the stick reads as centred,
//! so the offset eases back to nothing and the camera is the shot's again. Skate 3 adds the elevation to
//! the arm's (`bind_shot`); its heading offset is 0 on the disc and the heading isn't applied.

use super::tracker::RAD;
use crate::tuning::CamTuning;

/// Skate 3's `0x3c888889`: 1/60 as a float (the speeds are per second, the update runs a tick).
const STEP: f32 = 0.016_666_668;
/// The most either offset goes: a right angle less 5 degrees.
const LIMIT: f32 = core::f32::consts::FRAC_PI_2 - 5.0 * RAD;

/// The arm's look offsets (radians).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LookInput {
    pub heading: f32,
    pub elevation: f32,
}

impl LookInput {
    /// One tick with the stick (x, y) as the shot lets the camera read it ([0, 0] when it doesn't).
    pub fn update(&mut self, stick: [f32; 2], t: &CamTuning) {
        let heading = (stick[0] * t.look_heading_offset * RAD).clamp(-LIMIT, LIMIT);
        self.heading += (heading - self.heading) * (t.look_heading_speed * STEP);
        let elevation = (stick[1] * t.look_elevation_offset * RAD).clamp(-LIMIT, LIMIT);
        self.elevation += (elevation - self.elevation) * (t.look_elevation_speed * STEP);
    }

    pub fn reset(&mut self) {
        *self = LookInput::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_closes_on_the_stick_and_eases_back() {
        let t = CamTuning::BUILT_IN;
        let mut l = LookInput::default();
        l.update([1.0, 1.0], &t);
        // Skate 3: 2/s times 1/60 of the way to 30 degrees on the first tick.
        assert!(
            (l.elevation - 30.0 * RAD * 2.0 / 60.0).abs() < 1e-5,
            "{}",
            l.elevation
        );
        for _ in 0..600 {
            l.update([1.0, 1.0], &t);
        }
        assert!((l.elevation - 30.0 * RAD).abs() < 1e-3, "{}", l.elevation);
        // FreeCamHeadingOffset is 0 on the disc: the stick's x turns nothing.
        assert_eq!(l.heading, 0.0);
        for _ in 0..600 {
            l.update([0.0, 0.0], &t);
        }
        assert!(l.elevation.abs() < 1e-3, "{}", l.elevation);
    }

    #[test]
    fn offsets_stop_short_of_straight_up() {
        let mut t = CamTuning::BUILT_IN;
        t.look_elevation_offset = 180.0;
        t.look_elevation_speed = 60.0;
        let mut l = LookInput::default();
        l.update([0.0, -1.0], &t);
        assert!((l.elevation + LIMIT).abs() < 1e-6);
    }
}
