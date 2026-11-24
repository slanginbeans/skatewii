//! Slow motion on long airs, after Skate 3's (SK8-ENGINE/skate-3-rust-engine, GPL-3.0,
//! crates/skate-core/src/camera/slow_motion.rs `SlowMotionController`, crates/skate-game/src/physics/
//! clock.rs).
//!
//! Skate 3's camera graph runs the behaviour in one state, the low camera's
//! `OnBoard/Air/MegaParkAirBugFix`: on the board in the air, not off a grind, with a predicted air time
//! over 2 s (the high camera never slows). While it runs, the game's simulation clock ticks at
//! `60 + curve(progress) x (fps_at_scale_one - 60)` ticks a second, rounded to whole ticks, where
//! progress is the time since the state began over (the time before the first air prediction + the
//! predicted air time, latched once) and is 0.1 until a prediction comes. Skate 3's curve dips to 35
//! ticks a second (0.58x) a fifth of the way through the air and is back to 60 by half way; leaving the
//! state puts the clock back to 60.
//!
//! The simulation itself never changes: each tick is still `game::TICK` (1/60 s); slow motion only runs
//! fewer of them per displayed frame (`Game::update`), and frames in between are drawn between the last
//! two ticks.

use crate::tuning::CamTuning;

/// The normal simulation rate (ticks a second).
pub const NORMAL: f32 = 60.0;

/// One run of the slow-motion behaviour (Skate 3's controller instance).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlowMotion {
    elapsed: f32,
    air_time: f32,
    before_prediction: f32,
    latched: bool,
}

impl SlowMotion {
    /// The behaviour began: its first rate (the curve's start: normal speed).
    pub fn begin(t: &CamTuning) -> (SlowMotion, f32) {
        (SlowMotion::default(), rate(t, 0.0))
    }

    /// One graph tick of `dt` s with the predicted air time (s; 0 or less: none yet). Returns the rate.
    pub fn update(&mut self, dt: f32, air_time: f32, t: &CamTuning) -> f32 {
        self.elapsed += dt;
        // Skate 3 takes the no-prediction branch for a NaN too.
        let progress = if air_time > 0.0 {
            if !self.latched {
                self.air_time = air_time;
                self.latched = true;
            }
            self.elapsed / (self.before_prediction + self.air_time)
        } else {
            if !self.latched {
                self.before_prediction += dt;
            }
            f32::from_bits(0x3dcc_cccd)
        };
        rate(t, progress)
    }
}

/// The simulation's ticks a second at `progress` through the air: Skate 3's timestep
/// `1 / (60 + curve x (fps_at_scale_one - 60))`, its timer rounding it to whole ticks a second.
pub fn rate(t: &CamTuning, progress: f32) -> f32 {
    let k = t.slow_curve.eval(progress);
    let hz = NORMAL + k * (t.slow_fps - NORMAL);
    libm::truncf(hz + 0.5).clamp(1.0, NORMAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skate3s_curve_dips_to_35_a_fifth_through_and_is_back_by_half_way() {
        let t = CamTuning::BUILT_IN;
        assert_eq!(rate(&t, 0.0), 60.0);
        assert_eq!(rate(&t, 0.2), 35.0);
        assert_eq!(rate(&t, 0.6), 60.0);
        assert_eq!(rate(&t, 1.0), 60.0);
        // In between it eases, in whole ticks a second.
        let r = rate(&t, 0.1);
        assert!(r > 35.0 && r < 60.0 && r == libm::truncf(r), "{r}");
    }

    #[test]
    fn progress_waits_for_a_prediction_then_latches_it() {
        let t = CamTuning::BUILT_IN;
        let (mut s, first) = SlowMotion::begin(&t);
        assert_eq!(first, 60.0);
        // No prediction yet: progress 0.1.
        assert_eq!(s.update(0.1, 0.0, &t), rate(&t, 0.1));
        // Then 2.4 s predicted: (0.2) / (0.1 + 2.4) of the way.
        assert_eq!(s.update(0.1, 2.4, &t), rate(&t, 0.2 / 2.5));
        // A later, different prediction doesn't move it.
        assert_eq!(s.update(0.1, 9.0, &t), rate(&t, 0.3 / 2.5));
    }
}
