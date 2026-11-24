//! Session markers: put one down where the rider is and go back to it (Skate 3's session marker, after
//! SK8-ENGINE/skate-3-rust-engine, GPL-3.0: `crates/skate-game/src/session_marker/`, `state.rs`'s
//! `Hold` and `duration`, the eligibility in `mod.rs` and the spot check in `validation.rs`).
//!
//! Skate 3 keeps the rider's transform (where and which way he faces), whether he was on the board or on
//! foot and which foot was forward (here: riding switch or not); not his speed, so going back puts him
//! there at rest, as a respawn does. A marker can go down riding on the ground with the board upright
//! (Skate 3: two wheels down, the deck's up over 0.71) or standing or walking on foot, on ground that
//! passes the same check as Skate 3's checkpoints (`skater::respawn`). Going back is a hold whose length
//! grows with the distance (0.2 s up to 100 m, 1 s from 1 km), as in Skate 3; within half a metre of the
//! marker nothing happens.
//!
//! The pad: Z + d-pad (see [`Chord`]): up puts the marker down, down (held) goes back to it, left opens
//! the replay.

use crate::input::btn;
use crate::math::{Vec3, v3};
use crate::park::Terrain;
use crate::skater::{Mode, Skater, respawn};
use crate::tuning::Tuning;

/// A marker: where the rider was, which way he faced (yaw), his stance and whether he was riding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker {
    pub pos: Vec3,
    pub heading: f32,
    pub switch: bool,
    pub on_board: bool,
}

/// Skate 3's minimum deck up (the deck's up's y) for a marker on the board.
const DECK_UP_MIN: f32 = 0.71;
/// Within this of the marker (m) going back does nothing (Skate 3's 828992C8).
const NEAR: f32 = 0.5;
/// One UI tick of the hold (Skate 3's `0x3c888889`, 1/60 s as a float).
const HOLD_STEP: f32 = 0.016_666_668;

/// How long the return is held at this distance (s): Skate 3's ramp, 0.2 s to 100 m, 1 s from 1 km.
pub fn duration(distance: f32) -> f32 {
    if distance <= 100.0 {
        0.2
    } else if distance >= 1000.0 {
        1.0
    } else {
        // Skate 3's 0x3a690453 and 0x3de38e39: 0.2 at 100 m, 1 at 1 km.
        distance * f32::from_bits(0x3a69_0453) + f32::from_bits(0x3de3_8e39)
    }
}

/// The return's hold (Skate 3's `PlayerUI::UpdateSessionMarker`), one tick at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hold {
    elapsed: f32,
    fired: bool,
    tail: u8,
}

/// One tick of the hold: the screen effect's amount (0..1) and whether to go back now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    pub progress: f32,
    pub relocate: bool,
}

impl Hold {
    pub fn cancel(&mut self) {
        *self = Hold::default();
    }

    /// `held`: the return is held; `usable`: there is a marker to go to; `distance` to it; `ready`: the
    /// rider can be moved now (not mid-bail).
    pub fn update(&mut self, held: bool, usable: bool, distance: f32, ready: bool) -> Step {
        let mut out = Step::default();
        if !held {
            self.elapsed = 0.0;
            self.fired = false;
        } else if !self.fired && usable {
            self.elapsed += HOLD_STEP;
            if distance > NEAR && distance.is_finite() {
                let d = duration(distance);
                if ready && self.elapsed > d {
                    self.fired = true;
                    self.tail = 3;
                    out.relocate = true;
                }
                out.progress = (self.elapsed / d).clamp(0.0, 1.0);
            }
        }
        // The effect stays full for the relocation tick and two more.
        if self.tail > 0 {
            out.progress = 1.0;
            self.tail -= 1;
        }
        out
    }
}

/// The game's marker state: the marker (one, as in Skate 3) and the return's hold.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Markers {
    pub marker: Option<Marker>,
    pub hold: Hold,
    /// The return's screen effect this tick (0..1).
    pub progress: f32,
}

impl Markers {
    /// Forgets the marker (a new location).
    pub fn clear(&mut self) {
        *self = Markers::default();
    }

    /// Where the rider stands now as a marker, if one may go down here.
    pub fn here(t: &Tuning, park: &dyn Terrain, s: &Skater) -> Option<Marker> {
        let (on_board, heading) = match s.mode {
            Mode::Rolling if s.board_frame().cols[1].y > DECK_UP_MIN => (true, s.heading),
            Mode::Walk if s.biped.air.is_none() => (false, s.biped.heading),
            _ => return None,
        };
        let y = respawn::marker_ground(t, park, s.pos)?;
        Some(Marker {
            pos: v3(s.pos.x, y, s.pos.z),
            heading,
            switch: s.switch,
            on_board,
        })
    }

    /// One tick of the return's hold. True when it is time to go back.
    pub fn tick(&mut self, held: bool, s: &Skater) -> bool {
        let usable = self.marker.is_some();
        let distance = self.marker.map_or(0.0, |m| (s.pos - m.pos).length());
        let ready = !matches!(s.mode, Mode::Bailed { .. } | Mode::Runout { .. })
            && !s.is_airborne()
            && s.biped.air.is_none();
        let step = self.hold.update(held, usable, distance, ready);
        self.progress = step.progress;
        step.relocate
    }
}

/// What a frame's Z + d-pad chord asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChordOut {
    /// Z + d-pad up went down: put a marker down.
    pub place: bool,
    /// Z + d-pad down is held: going back to the marker.
    pub back: bool,
    /// Z + d-pad left went down: open (or, in the replay, close) the replay.
    pub replay: bool,
}

const DPAD: u32 = btn::DPAD_UP | btn::DPAD_DOWN | btn::DPAD_LEFT | btn::DPAD_RIGHT;

/// The Z + d-pad chord. A d-pad press while Z is held, with the rider not in the air, belongs to the
/// chord: it never reaches the character gestures, and from then on Z reads as let go for the riding
/// until it really is (so the Z used for the chord can't dark-catch a flip). In the air Z stays the dark
/// catch and the d-pad its gestures. Without Z the d-pad is the gestures, as before.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Chord {
    prev: u32,
    /// Z went into a chord: hidden from the riding until let go.
    z_taken: bool,
    /// D-pad directions taken by the chord, hidden until let go.
    dpad_taken: u32,
}

impl Chord {
    /// One frame of buttons; `allowed`: chords are taken now (not in the air). Returns the buttons the
    /// riding sees and what the chord asks for.
    pub fn read(&mut self, buttons: u32, allowed: bool) -> (u32, ChordOut) {
        let pressed = buttons & !self.prev;
        self.prev = buttons;
        let z = buttons & btn::Z != 0;
        if !z {
            self.z_taken = false;
        }
        self.dpad_taken &= buttons;
        let mut new = 0;
        if z && allowed {
            new = pressed & DPAD;
            if new != 0 {
                self.z_taken = true;
                self.dpad_taken |= new;
            }
        }
        let out = ChordOut {
            place: new & btn::DPAD_UP != 0,
            back: z && self.dpad_taken & btn::DPAD_DOWN != 0,
            replay: new & btn::DPAD_LEFT != 0,
        };
        let mut seen = buttons & !self.dpad_taken;
        if self.z_taken {
            seen &= !btn::Z;
        }
        (seen, out)
    }

    /// Forgets what was held (a menu or the replay took the pad), keeping what is held now as held.
    pub fn sync(&mut self, buttons: u32) {
        self.prev = buttons;
        self.dpad_taken &= buttons;
        if buttons & btn::Z == 0 {
            self.z_taken = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::Park;

    #[test]
    fn release_cancels_and_a_completed_hold_does_not_repeat() {
        let mut hold = Hold::default();
        for _ in 0..6 {
            assert!(!hold.update(true, true, 20.0, true).relocate);
        }
        assert_eq!(hold.update(false, true, 20.0, true), Step::default());
        for _ in 0..11 {
            assert!(!hold.update(true, true, 20.0, true).relocate);
        }
        let mut count = 0;
        for _ in 0..120 {
            count += usize::from(hold.update(true, true, 20.0, true).relocate);
        }
        assert_eq!(count, 1);
        assert_eq!(hold.update(true, true, 20.0, true).progress, 0.0);
    }

    #[test]
    fn no_marker_nearby_and_blocked_returns() {
        let mut hold = Hold::default();
        for _ in 0..100 {
            assert_eq!(hold.update(true, false, 10.0, true), Step::default());
        }
        for _ in 0..100 {
            assert_eq!(hold.update(true, true, 0.5, true), Step::default());
        }
        assert!(!hold.update(true, true, 10.0, false).relocate);
        assert!(hold.update(true, true, 10.0, true).relocate);
        assert_eq!(hold.update(false, true, 10.0, true).progress, 1.0);
        assert_eq!(hold.update(false, true, 10.0, true).progress, 1.0);
        assert_eq!(hold.update(false, true, 10.0, true).progress, 0.0);
    }

    #[test]
    fn the_hold_grows_with_the_distance_as_in_skate_3() {
        assert_eq!(duration(1.0), 0.2);
        assert_eq!(duration(100.0), 0.2);
        assert!((duration(550.0) - 0.6).abs() < 1e-6);
        assert_eq!(duration(1000.0), 1.0);
        for (distance, expected) in [(100.0, 13), (550.0, 36), (1000.0, 61)] {
            let mut hold = Hold::default();
            let tick = (1..=120).find(|_| hold.update(true, true, distance, true).relocate);
            assert_eq!(tick, Some(expected), "distance {distance}");
        }
    }

    #[test]
    fn z_and_the_d_pad_make_a_chord_and_hide_both_from_the_riding() {
        let mut c = Chord::default();
        // Z alone is seen (the dark catch).
        let (seen, out) = c.read(btn::Z, true);
        assert_eq!((seen, out), (btn::Z, ChordOut::default()));
        // Z + up: a marker, and neither Z nor up reaches the riding.
        let (seen, out) = c.read(btn::Z | btn::DPAD_UP, true);
        assert!(out.place && !out.back && !out.replay);
        assert_eq!(seen, 0);
        // Still held: no second marker; let go of up, Z still hidden until let go.
        assert!(!c.read(btn::Z | btn::DPAD_UP, true).1.place);
        assert_eq!(c.read(btn::Z, true).0, 0);
        assert_eq!(c.read(0, true).0, 0);
        assert_eq!(c.read(btn::Z, true).0, btn::Z);
        // Z + down held: going back, every frame it is held.
        assert!(c.read(btn::Z | btn::DPAD_DOWN, true).1.back);
        assert!(c.read(btn::Z | btn::DPAD_DOWN, true).1.back);
        assert!(!c.read(btn::Z, true).1.back);
        // Z + left: the replay.
        assert!(c.read(btn::Z | btn::DPAD_LEFT, true).1.replay);
        c.read(0, true);
        // The d-pad without Z is the gestures.
        assert_eq!(
            c.read(btn::DPAD_UP, true),
            (btn::DPAD_UP, ChordOut::default())
        );
        c.read(0, true);
        // In the air no chord: Z (the dark catch) and the d-pad pass through.
        c.read(btn::Z, false);
        assert_eq!(
            c.read(btn::Z | btn::DPAD_UP, false),
            (btn::Z | btn::DPAD_UP, ChordOut::default())
        );
    }

    #[test]
    fn a_marker_goes_down_riding_upright_on_safe_ground_and_not_in_the_air() {
        let park = Park::demo();
        let t = Tuning::BUILT_IN;
        let mut s = Skater::spawn(&park);
        let m = Markers::here(&t, &park, &s).expect("on the ground at the spawn");
        assert!(m.on_board);
        assert_eq!(m.heading, s.heading);
        s.mode = Mode::Air { time: 0.1 };
        assert_eq!(Markers::here(&t, &park, &s), None);
        s.mode = Mode::Bailed { time: 0.1 };
        assert_eq!(Markers::here(&t, &park, &s), None);
    }

    #[test]
    fn going_back_waits_out_a_bail_and_the_air() {
        let park = Park::demo();
        let mut s = Skater::spawn(&park);
        let mut m = Markers {
            marker: Some(Marker {
                pos: s.pos + v3(10.0, 0.0, 0.0),
                heading: 0.0,
                switch: false,
                on_board: true,
            }),
            ..Markers::default()
        };
        s.mode = Mode::Air { time: 0.1 };
        assert!(!(0..60).any(|_| m.tick(true, &s)));
        s.mode = Mode::Rolling;
        assert!((0..60).any(|_| m.tick(true, &s)));
    }
}
