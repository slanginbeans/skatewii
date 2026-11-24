//! Skate 3's speed wobble (SK8 `skate-core/src/riding/speed_wobble.rs`, TU3
//! `SpeedWobble::CalculateSpeedWobble`): riding faster than an onset that tight trucks and a crouch
//! raise, the trucks' tilt picks up a sine wobble whose frequency and amplitude follow the wobble's time
//! and the speed over the onset. Ground Update applies the returned tilt before the trucks' targets
//! (flatground: [`super::Skater`]'s ride, before the deck tilt eases in), and the amplitude (0..1) is what
//! the riding animation's turn conditioner reads (`SpeedWobbleTurnBlend`, `AnimationTurnWobbleScalar`).

use crate::tuning::wobble::WobbleTuning;

/// One tick of the game (s): SK8 advances the wobble's clock by this constant, not by `dt`.
const TICK: f32 = 1.0 / 60.0;
/// The phase step per Hz in a tick (2 pi / 60).
const PHASE_PER_HZ: f32 = 2.0 * crate::math::PI / 60.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpeedWobble {
    pub active: bool,
    /// Seconds since the wobble started, its phase (rad), amplitude share (0..1) and last displacement.
    pub time: f32,
    pub phase: f32,
    pub amplitude: f32,
    pub displacement: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct WobbleInput {
    /// The steering's deck tilt this tick.
    pub tilt: f32,
    /// Speed along the board (m/s), signed: Skate 3 does not take its magnitude, so rolling tail first
    /// never wobbles.
    pub speed: f32,
    /// The rider's centre of mass over the board (m).
    pub com_height: f32,
    /// The trucks' tightness (0 loose .. 1 tight).
    pub truck_tightness: f32,
}

impl SpeedWobble {
    /// Back to rest (SK8 `SpeedWobbleState::reset`, also run on entering the ground).
    pub fn reset(&mut self) {
        *self = SpeedWobble::default();
    }

    /// The speed the wobble starts above (m/s).
    pub fn onset(t: &WobbleTuning, com_height: f32, truck_tightness: f32) -> f32 {
        let height = (com_height - t.crouch_min) / (t.crouch_max - t.crouch_min);
        let crouch = unit(1.0 - height);
        crouch * t.crouch_speed + t.tightness_speed * truck_tightness + t.activation
    }

    /// One tick: the tilt to steer with.
    pub fn update(&mut self, t: &WobbleTuning, i: WobbleInput) -> f32 {
        let threshold = Self::onset(t, i.com_height, i.truck_tightness);
        if !self.active && i.speed > threshold {
            self.reset();
            self.active = true;
        }
        if !self.active {
            return i.tilt;
        }
        let time_fraction = unit(self.time / t.time_max);
        let speed_fraction = unit((i.speed - threshold) / t.speed_range);
        let frequency = t.frequency_vs_time.eval(time_fraction)
            * t.frequency_vs_speed.eval(speed_fraction)
            * t.max_frequency;
        let amplitude =
            t.amplitude_vs_speed.eval(speed_fraction) * t.amplitude_vs_time.eval(time_fraction);
        let phase = frequency * PHASE_PER_HZ + self.phase;
        let displacement = libm::sinf(phase) * t.max_amplitude * amplitude * t.amplitude_scale;
        self.time += TICK;
        self.phase = phase;
        self.amplitude = amplitude;
        self.displacement = displacement;
        // While it wobbles, the steering's tilt and the wobble are averaged (as TU3 does).
        let tilt = (displacement + i.tilt) * 0.5;
        // This tick still steers with the wobble when the speed has dropped below the onset.
        if i.speed < threshold {
            self.reset();
        }
        tilt
    }
}

/// Clamped to 0..1 (NaN to 0, as SK8's `unit`).
fn unit(v: f32) -> f32 {
    let lower = if -v >= 0.0 { 0.0 } else { v };
    if 1.0 - lower >= 0.0 { lower } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: WobbleTuning = WobbleTuning::BUILT_IN;

    fn input(speed: f32) -> WobbleInput {
        WobbleInput {
            tilt: 0.1,
            speed,
            com_height: 0.9,
            truck_tightness: 0.7,
        }
    }

    #[test]
    fn the_onset_follows_skate_3s_rule() {
        // Standing tall: 15 + 5 x 0.7.
        assert!((SpeedWobble::onset(&T, 0.9, 0.7) - 18.5).abs() < 1e-5);
        // Fully crouched adds 3 m/s; looser trucks start sooner.
        assert!((SpeedWobble::onset(&T, 0.5, 0.7) - 21.5).abs() < 1e-5);
        assert!((SpeedWobble::onset(&T, 0.7, 0.0) - 16.5).abs() < 1e-5);
    }

    #[test]
    fn below_the_onset_the_tilt_passes_through() {
        let mut w = SpeedWobble::default();
        for _ in 0..120 {
            assert_eq!(w.update(&T, input(18.0)), 0.1);
        }
        assert_eq!(w, SpeedWobble::default());
        // Riding backwards fast never wobbles (the speed is signed).
        assert_eq!(w.update(&T, input(-30.0)), 0.1);
        assert!(!w.active);
    }

    #[test]
    fn above_the_onset_the_tilt_wobbles_and_stops_below_it() {
        let mut w = SpeedWobble::default();
        let speed = 18.5 + 0.5 * T.speed_range;
        // The first tick: time 0 (no amplitude yet), the steering halved.
        let first = w.update(&T, input(speed));
        assert!(w.active);
        assert!((first - 0.05).abs() < 1e-6, "{first}");
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..120 {
            let tilt = w.update(&T, input(speed));
            lo = lo.min(tilt);
            hi = hi.max(tilt);
            assert!((0.0..=1.0).contains(&w.amplitude));
            assert!(
                (tilt - (w.displacement + 0.1) * 0.5).abs() < 1e-6,
                "the tilt is the average of the steering and the wobble"
            );
        }
        // Amplitude at half the speed range, well into the time curve's plateau.
        let amp = T.amplitude_vs_speed.eval(0.5) * T.amplitude_vs_time.eval(2.0 / T.time_max);
        let swing = T.max_amplitude * T.amplitude_scale * amp * 0.5;
        assert!(swing > 0.01);
        assert!(
            hi > 0.05 + 0.9 * swing && lo < 0.05 - 0.9 * swing,
            "{lo}..{hi}"
        );
        // Dropping below the onset: that tick still wobbles, then it is over.
        let last = w.update(&T, input(10.0));
        // (No amplitude at the onset itself: only the halving is left.)
        assert!((last - 0.05).abs() < 1e-6, "{last}");
        assert!(!w.active && w.amplitude == 0.0);
        assert_eq!(w.update(&T, input(10.0)), 0.1);
    }

    #[test]
    fn a_crouch_holds_the_wobble_off() {
        let mut w = SpeedWobble::default();
        let crouched = WobbleInput {
            com_height: 0.5,
            ..input(20.0)
        };
        assert_eq!(w.update(&T, crouched), 0.1);
        assert!(!w.active);
        w.update(&T, input(20.0));
        assert!(w.active);
    }
}
