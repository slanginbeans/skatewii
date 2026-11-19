//! Scripted input that reacts to game state, for tests, the host preview and (later) hands-off runs in
//! Dolphin. Produces raw pad states exactly like a real controller would.

use crate::game::Game;
use crate::input::{PadRaw, btn, flag};
use crate::skater::Mode;

/// Push up to speed down the spawn lane, load the flick stick on the kicker and kickflip off the lip.
#[derive(Default)]
pub struct KickerRun {
    frame: u32,
    flicked: bool,
}

/// Raw stick values for the C-stick.
const BACK: (i8, i8) = (0, -80);
const UP_LEFT: (i8, i8) = (-50, 62);

impl KickerRun {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next(&mut self, game: &Game) -> PadRaw {
        let s = &game.skater;
        let mut pad = PadRaw {
            flags: flag::CONNECTED,
            ..Default::default()
        };
        // Tap A every half second until near top push speed.
        if s.mode == Mode::Rolling
            && s.speed < 6.5
            && self.frame.is_multiple_of(30)
            && !self.flicked
        {
            pad.buttons |= btn::A;
        }
        if !self.flicked && s.mode == Mode::Rolling {
            if s.pos.z > -2.8 {
                (pad.cstick_x, pad.cstick_y) = UP_LEFT;
                self.flicked = true;
            } else if s.pos.z > -5.0 {
                (pad.cstick_x, pad.cstick_y) = BACK;
            }
        }
        self.frame += 1;
        pad
    }
}

/// The kicker run, then the rest of the move set: carve both ways (the rider leans), a U-turn, a
/// balanced manual, an ollie with an indy grab, and finally an over-spun ollie that bails. For the
/// preview and an end-to-end test.
#[derive(Default)]
pub struct TrickTour {
    kicker: KickerRun,
    phase: Tour,
    /// Frames spent in the current phase.
    t: u32,
    /// The kicker jump has left the ground.
    jumped: bool,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
enum Tour {
    #[default]
    Kicker,
    Carve,
    Manual,
    GrabOllie,
    GrabAir,
    BailOllie,
    BailAir,
    Done,
}

const UP: (i8, i8) = (0, 80);

impl TrickTour {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next(&mut self, game: &Game) -> PadRaw {
        let s = &game.skater;
        let rolling = s.mode == Mode::Rolling;
        let mut pad = PadRaw {
            flags: flag::CONNECTED,
            ..Default::default()
        };
        let next = |p: &mut Self, phase| {
            p.phase = phase;
            p.t = 0;
        };
        self.t += 1;
        // Keep the speed down so everything fits in the park.
        if rolling && s.speed > 4.0 && self.phase != Tour::Kicker {
            pad.buttons |= btn::B;
        }
        match self.phase {
            Tour::Kicker => {
                pad = self.kicker.next(game);
                // Landed off the kicker and rolled a moment.
                if !rolling {
                    self.jumped = true;
                    self.t = 0;
                } else if self.jumped && self.t > 30 {
                    next(self, Tour::Carve);
                }
            }
            Tour::Carve => {
                // A little right, then a long left: a U-turn back down the park beside the kicker.
                pad.stick_x = if self.t < 25 { -60 } else { 72 };
                if self.t > 25 + 90 {
                    next(self, Tour::Manual);
                }
            }
            Tour::Manual => {
                // Pull the stick down and keep the balance marker centred.
                let b = s.manual.map_or(0.0, |m| m.balance);
                let push = (0.75 - 1.2 * b).clamp(0.35, 1.0);
                pad.stick_y = -(push * 72.0) as i8;
                if s.speed < 3.0 {
                    pad.buttons |= btn::A;
                }
                if self.t > 100 {
                    next(self, Tour::GrabOllie);
                }
            }
            Tour::GrabOllie => {
                // Load, then flick straight up.
                (pad.cstick_x, pad.cstick_y) = if self.t < 10 { BACK } else { UP };
                if !rolling {
                    next(self, Tour::GrabAir);
                }
            }
            Tour::GrabAir => {
                // Grab from just after the pop until falling, then let go to land.
                if self.t > 4 && s.vel.y > -2.0 {
                    pad.trig_r = 255;
                }
                if rolling {
                    next(self, Tour::BailOllie);
                }
            }
            Tour::BailOllie => {
                if self.t > 50 {
                    (pad.cstick_x, pad.cstick_y) = if self.t < 60 { BACK } else { UP };
                }
                if !rolling {
                    next(self, Tour::BailAir);
                }
            }
            Tour::BailAir => {
                // Spin about 90 degrees and hold it: lands sideways.
                pad.stick_x = 26;
                if matches!(s.mode, Mode::Bailed { .. }) {
                    next(self, Tour::Done);
                }
            }
            Tour::Done => {}
        }
        pad
    }

    /// True once the whole tour has been played (after the bail).
    pub fn done(&self) -> bool {
        self.phase == Tour::Done
    }
}
