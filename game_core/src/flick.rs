//! The flick stick (the C-stick, Skate 3's right stick): gestures for flick tricks, plus the crouch you
//! wind up with before popping and the light hold that balances a manual.
//!
//! Each frame [`FlickInput::update`] turns the stick into gesture space (y towards the player, small
//! components zeroed as in Skate 3) and runs the [`Recognizer`]. What the gesture *does* (which trick,
//! mirrored for the stance, fitted to a manual) is up to the skater.

use crate::gesture::{Flick, Recognizer, Shape};
use crate::math::sqrtf;

/// Components smaller than this are zeroed before matching.
const COMPONENT_DEAD: f32 = 0.1;
/// Stick deflection (towards the player, or away for a nollie) that starts the crouch...
const CROUCH_FROM: f32 = 0.35;
/// ...and where it is a full crouch.
const CROUCH_FULL: f32 = 0.9;

/// What the flick stick did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlickOut {
    /// A completed flick gesture, as recognised (before stance mirroring).
    pub flick: Option<Flick>,
    /// 0..1: how fast it was (a faster flick pops higher).
    pub strength: f32,
    /// The stick is still on the end of the last kickflip or heelflip gesture (more flips).
    pub holding_flip: bool,
    /// 0..1: how far the rider is crouched, from the stick held back or forward.
    pub crouch: f32,
    /// The stick in gesture space (x right, y towards the player).
    pub stick: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct FlickInput {
    rec: Recognizer,
    /// The last recognised flick was a kickflip or heelflip (it can be held for more flips).
    hold_candidate: bool,
    /// Shapes come from the player's disc.
    pub from_disc: bool,
}

impl Default for FlickInput {
    fn default() -> Self {
        Self::new()
    }
}

impl FlickInput {
    /// With no shapes yet (the rules pack's come with [`FlickInput::with_shapes`]).
    pub fn new() -> FlickInput {
        FlickInput {
            rec: Recognizer::new(alloc::vec::Vec::new()),
            hold_candidate: false,
            from_disc: false,
        }
    }

    /// With shapes from a rules pack (Skate 3's own).
    pub fn with_shapes(shapes: alloc::vec::Vec<Shape>) -> FlickInput {
        FlickInput {
            rec: Recognizer::new(shapes),
            hold_candidate: false,
            from_disc: true,
        }
    }

    pub fn recognizer(&self) -> &Recognizer {
        &self.rec
    }

    /// Feeds one frame of the C-stick (`stick`: +y away from the player, as [`crate::input::Pad`]).
    pub fn update(&mut self, stick: [f32; 2], max_misses: u8) -> FlickOut {
        let g = [stick[0], -stick[1]].map(|v| if v.abs() < COMPONENT_DEAD { 0.0 } else { v });
        let holding = self.hold_candidate && self.rec.held(g).is_some();
        if !holding {
            self.hold_candidate = false;
        }
        let mut out = FlickOut {
            holding_flip: holding,
            stick: g,
            ..FlickOut::default()
        };
        if let Some(r) = self.rec.sample(g, max_misses) {
            let name = &self.rec.shapes()[r.shape].name;
            if let Some(f) = Flick::from_gesture(name) {
                self.hold_candidate = matches!(
                    name.as_str(),
                    "Kickflip" | "Heelflip" | "N_Kickflip" | "N_Heelflip"
                );
                out.flick = Some(f);
                out.strength = r.strength;
            }
        }
        // Crouch: the stick held mostly straight back (or forward, for a nollie).
        let mag = sqrtf(g[0] * g[0] + g[1] * g[1]);
        if g[1].abs() > g[0].abs() {
            out.crouch = ((mag - CROUCH_FROM) / (CROUCH_FULL - CROUCH_FROM)).clamp(0.0, 1.0);
        }
        out
    }

    /// Forgets a gesture in progress (after a bail or respawn).
    pub fn reset(&mut self) {
        self.rec.reset();
        self.hold_candidate = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gesture::Trick;

    /// Stick at `deg` clockwise from forward (+y up, as the pad reports it).
    fn at(deg: f32) -> [f32; 2] {
        let a = deg * crate::math::PI / 180.0;
        [crate::math::sinf(a), crate::math::cosf(a)]
    }

    #[test]
    fn pull_back_then_flick_forward_is_an_ollie_and_crouches_first() {
        let mut f = FlickInput::with_shapes(crate::gesture::tests::built_in());
        let mut crouched = false;
        let mut got = None;
        for s in [at(180.0); 8].into_iter().chain([[0.0, 0.0], at(0.0)]) {
            let o = f.update(s, 8);
            crouched |= o.crouch > 0.9;
            got = got.or(o.flick);
        }
        assert!(crouched);
        assert_eq!(got, Some(Flick::OLLIE));
    }

    #[test]
    fn a_held_kickflip_reports_holding() {
        let mut f = FlickInput::with_shapes(crate::gesture::tests::built_in());
        let mut got = None;
        let mut held = false;
        for (i, s) in [at(180.0); 6].into_iter().chain([at(50.0); 6]).enumerate() {
            let o = f.update(s, 8);
            got = got.or(o.flick);
            held |= i > 7 && o.holding_flip;
        }
        assert_eq!(got.map(|f| f.trick), Some(Trick::Kickflip));
        assert!(held);
        // Letting go ends it.
        assert!(!f.update([0.0, 0.0], 8).holding_flip);
    }
}
