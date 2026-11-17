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
