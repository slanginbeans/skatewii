//! Flick-stick trick input: the right stick (C-stick) is your feet.
//!
//! Pull the stick back to load up (crouch), then flick it forward fast to pop:
//! - back, then straight forward ...................... Ollie
//! - back, then forward-left .......................... Kickflip
//! - back, then forward-right ......................... Heelflip
//! - back, then roll around the left edge to forward .. Pop shove-it
//! - back, then roll around the right edge to forward . Frontside shove-it
//! - forward, then flick straight back ................ Nollie
//!
//! A flick only counts if it lands within [`FLICK_WINDOW`] seconds of leaving the loaded position;
//! a slow move is ignored. Sampled once per frame; works at 50 or 60 Hz.

use crate::math::{PI, atan2f, sqrtf, wrap_angle};

/// Seconds allowed from leaving the loaded position to reaching the flick target.
pub const FLICK_WINDOW: f32 = 0.25;
/// Stick deflection that counts as "pushed into a direction".
const PUSHED: f32 = 0.6;
/// Below this the stick is "through the centre".
const CENTRE: f32 = 0.45;
/// Half-width of the back/forward load sectors.
const SECTOR: f32 = 40.0 * PI / 180.0;
/// Rotation around the rim (degrees) that turns a flick into a shove-it.
const SHOVE_SWEEP: f32 = 100.0 * PI / 180.0;
/// Largest per-frame angle change still treated as rolling around the rim.
const MAX_RIM_STEP: f32 = 100.0 * PI / 180.0;
/// Sideways deflection at the end of a forward flick that turns it into a flip.
const FLIP_SIDE: f32 = 0.35;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flick {
    Ollie,
    Nollie,
    Kickflip,
    Heelflip,
    PopShoveIt,
    FsShoveIt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Load {
    /// Stick held back (towards the player).
    Back,
    /// Stick held forward.
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    Loaded {
        load: Load,
        angle: f32,
    },
    Flicking {
        load: Load,
        t: f32,
        swept: f32,
        last: Option<f32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlickReader {
    state: State,
}

impl Default for FlickReader {
    fn default() -> Self {
        Self::new()
    }
}

fn sector_of(angle: f32) -> Option<Load> {
    if wrap_angle(angle + PI / 2.0).abs() <= SECTOR {
        Some(Load::Back)
    } else if wrap_angle(angle - PI / 2.0).abs() <= SECTOR {
        Some(Load::Forward)
    } else {
        None
    }
}

impl FlickReader {
    pub const fn new() -> Self {
        FlickReader { state: State::Idle }
    }

    /// True while the stick is held back: the skater crouches, ready to pop.
    pub fn is_loaded(&self) -> bool {
        matches!(self.state, State::Loaded { .. })
    }

    /// Feeds one frame of stick input; returns a trick when a flick completes.
    pub fn update(&mut self, stick: [f32; 2], dt: f32) -> Option<Flick> {
        let [x, y] = stick;
        let mag = sqrtf(x * x + y * y);
        let angle = atan2f(y, x);
        // A fast flick can move several sectors in one frame, so a transition re-runs the new state
        // on the same sample (at most twice).
        for _ in 0..3 {
            match self.state {
                State::Idle => {
                    if mag >= PUSHED
                        && let Some(load) = sector_of(angle)
                    {
                        self.state = State::Loaded { load, angle };
                    }
                    return None;
                }
                State::Loaded { load, .. } => {
                    if mag >= PUSHED && sector_of(angle) == Some(load) {
                        self.state = State::Loaded { load, angle };
                        return None;
                    }
                    let State::Loaded { angle: last, .. } = self.state else {
                        unreachable!()
                    };
                    self.state = State::Flicking {
                        load,
                        t: 0.0,
                        swept: 0.0,
                        last: Some(last),
                    };
                    // Fall through: this sample may already complete the flick.
                }
                State::Flicking {
                    load,
                    t,
                    mut swept,
                    mut last,
                } => {
                    if mag >= CENTRE {
                        if let Some(prev) = last {
                            let step = wrap_angle(angle - prev);
                            // A jump this big in one frame went across the middle, not around the rim.
                            if step.abs() <= MAX_RIM_STEP {
                                swept += step;
                            }
                        }
                        last = Some(angle);
                    } else {
                        // Went through the centre: a straight flick, not a rim roll.
                        last = None;
                    }
                    let t = t + dt;
                    if let Some(trick) = Self::classify(load, x, y, mag, swept) {
                        self.state = State::Idle;
                        return Some(trick);
                    }
                    if mag >= PUSHED && sector_of(angle) == Some(load) {
                        // Wiggled back to where it started: still loaded.
                        self.state = State::Loaded { load, angle };
                    } else if t > FLICK_WINDOW {
                        self.state = State::Idle;
                    } else {
                        self.state = State::Flicking {
                            load,
                            t,
                            swept,
                            last,
                        };
                    }
                    return None;
                }
            }
        }
        None
    }

    fn classify(load: Load, x: f32, y: f32, mag: f32, swept: f32) -> Option<Flick> {
        if mag < PUSHED {
            return None;
        }
        match load {
            Load::Back if y >= PUSHED * 0.8 => Some(if swept >= SHOVE_SWEEP {
                // Counter-clockwise from back: back -> right -> forward.
                Flick::FsShoveIt
            } else if swept <= -SHOVE_SWEEP {
                Flick::PopShoveIt
            } else if x <= -FLIP_SIDE {
                Flick::Kickflip
            } else if x >= FLIP_SIDE {
                Flick::Heelflip
            } else {
                Flick::Ollie
            }),
            Load::Forward if y <= -PUSHED * 0.8 => Some(Flick::Nollie),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{cosf, sinf};
    use alloc::vec::Vec;

    const DT: f32 = 1.0 / 60.0;

    fn at(deg: f32, mag: f32) -> [f32; 2] {
        let a = deg * PI / 180.0;
        [cosf(a) * mag, sinf(a) * mag]
    }

    fn run(samples: &[[f32; 2]]) -> Vec<Flick> {
        let mut r = FlickReader::new();
        samples.iter().filter_map(|s| r.update(*s, DT)).collect()
    }

    fn hold(s: [f32; 2], frames: usize) -> Vec<[f32; 2]> {
        (0..frames).map(|_| s).collect()
    }

    #[test]
    fn straight_flicks() {
        for (end, want) in [
            (90.0, Flick::Ollie),
            (125.0, Flick::Kickflip),
            (55.0, Flick::Heelflip),
        ] {
            let mut s = hold(at(-90.0, 1.0), 10);
            s.push([0.0, 0.0]);
            s.push(at(end, 1.0));
            assert_eq!(run(&s), [want], "flick to {end} deg");
        }
    }

    #[test]
    fn one_frame_flick_still_counts() {
        let mut s = hold(at(-90.0, 1.0), 5);
        s.push(at(90.0, 1.0));
        assert_eq!(run(&s), [Flick::Ollie]);
    }

    #[test]
    fn rim_rolls_are_shove_its() {
        let mut ccw = hold(at(-90.0, 1.0), 5);
        ccw.extend([-45.0, 0.0, 45.0, 90.0].map(|d| at(d, 1.0)));
        assert_eq!(run(&ccw), [Flick::FsShoveIt]);

        let mut cw = hold(at(-90.0, 1.0), 5);
        cw.extend([-135.0, 180.0, 135.0, 90.0].map(|d| at(d, 1.0)));
        assert_eq!(run(&cw), [Flick::PopShoveIt]);
    }

    #[test]
    fn nollie_from_forward() {
        let mut s = hold(at(90.0, 1.0), 6);
        s.push([0.0, 0.0]);
        s.push(at(-90.0, 1.0));
        assert_eq!(run(&s), [Flick::Nollie]);
    }

    #[test]
    fn slow_move_is_not_a_flick() {
        let mut s = hold(at(-90.0, 1.0), 6);
        s.extend(hold([0.0, 0.0], 30)); // half a second in the centre
        s.push(at(90.0, 1.0));
        assert!(run(&s).is_empty());
    }

    #[test]
    fn forward_without_loading_does_nothing() {
        assert!(run(&hold(at(90.0, 1.0), 10)).is_empty());
    }

    #[test]
    fn loaded_while_held_back() {
        let mut r = FlickReader::new();
        r.update(at(-90.0, 1.0), DT);
        assert!(r.is_loaded());
        r.update([0.0, 0.0], DT);
        assert!(!r.is_loaded());
    }
}
