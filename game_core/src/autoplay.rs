//! Scripted input that reacts to game state, for tests, the host preview and (later) hands-off runs in
//! Dolphin. Produces raw pad states exactly like a real controller would.

use crate::game::Game;
use crate::input::{PadRaw, btn, flag};
use crate::math::{PI, atan2f, cosf, sinf, wrap_angle};
use crate::skater::Mode;

/// Push up to speed down the spawn lane, load the flick stick on the kicker and kickflip off the lip.
#[derive(Default)]
pub struct KickerRun {
    flicked: bool,
    /// Frames pushed so far (an imported park has no kicker at the spawn: flick on flat ground instead).
    pushed: u32,
}

/// Raw C-stick values: pulled back, and flicked forward and right (a goofy rider's kickflip).
const BACK: (i8, i8) = (0, -92);
const UP_RIGHT: (i8, i8) = (50, 62);
const UP: (i8, i8) = (0, 92);

/// The C-stick at `deg` degrees clockwise from straight forward, fully over.
fn rim(deg: f32) -> (i8, i8) {
    let a = deg * PI / 180.0;
    ((sinf(a) * 92.0) as i8, (cosf(a) * 92.0) as i8)
}

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
        // Hold A (keep pushing) until near top push speed.
        if s.mode == Mode::Rolling && s.speed < 6.0 && !self.flicked {
            pad.buttons |= btn::A;
        }
        if !self.flicked && s.mode == Mode::Rolling {
            self.pushed += 1;
            let (load, flick) = if game.imported_park().is_some() {
                (self.pushed > 150, self.pushed > 175)
            } else {
                (s.pos.z > -5.0, s.pos.z > -2.8)
            };
            if flick {
                (pad.cstick_x, pad.cstick_y) = UP_RIGHT;
                self.flicked = true;
            } else if load {
                (pad.cstick_x, pad.cstick_y) = BACK;
            }
        }
        pad
    }
}

/// The kicker run, then the rest of the move set: carve both ways (the rider leans), a U-turn, a
/// balanced manual, an ollie with an indy grab, a 360 flip, a 180 to fakie, pushing out of fakie (the
/// rider turns round to switch), a powerslide, a hippie jump, and finally a grab held into the landing
/// (a bail).
/// For the preview and an end-to-end test.
#[derive(Default)]
pub struct TrickTour {
    kicker: KickerRun,
    phase: Tour,
    /// Frames spent in the current phase.
    t: u32,
    /// The kicker jump has left the ground.
    jumped: bool,
    /// Ticks the current wind-up has been held, and when the flick started (0: not yet).
    wind: u32,
    flick_at: u32,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
enum Tour {
    #[default]
    Kicker,
    Carve,
    Manual,
    GrabOllie,
    GrabAir,
    TreFlip,
    TreAir,
    HalfCab,
    HalfCabAir,
    FakiePush,
    Powerslide,
    Hippie,
    HippieAir,
    BailOllie,
    BailAir,
    Done,
}

impl TrickTour {
    pub fn new() -> Self {
        Self::default()
    }

    /// A wind-up then a flick: holds `hold` until the rider is wound up (with Skate 3's state graphs,
    /// until its motion graph is in the anticipation cycle; else for 10 ticks), then plays `flick` one
    /// position per tick and holds the last.
    fn wind_and_flick(&mut self, game: &Game, hold: (i8, i8), flick: &[(i8, i8)]) -> (i8, i8) {
        if self.flick_at == 0 {
            self.wind += 1;
            let ready = match &game.graph {
                Some(g) => {
                    let st = g.runner.motion_state();
                    st.contains("AnticCyc") || (self.wind > 120 && st.contains("Anticipation"))
                }
                None => self.wind >= 10,
            };
            if !ready {
                return hold;
            }
            self.flick_at = self.t;
        }
        let k = (self.t - self.flick_at) as usize;
        flick[k.min(flick.len() - 1)]
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
            p.wind = 0;
            p.flick_at = 0;
        };
        self.t += 1;
        // Somewhere the script doesn't fit (an imported park): give up on a phase after 20 s.
        if self.t > 20 * 60 && self.phase != Tour::Done {
            next(self, Tour::Done);
            return pad;
        }
        // Keep the speed down so everything fits in the park (X: foot brake), and steer back towards the
        // middle when near a wall.
        let busy_stick = matches!(
            self.phase,
            Tour::Kicker | Tour::Carve | Tour::HalfCabAir | Tour::Powerslide | Tour::BailAir
        );
        if rolling && s.speed > 4.5 && !matches!(self.phase, Tour::Kicker | Tour::Powerslide) {
            pad.buttons |= btn::X;
        }
        if rolling && !busy_stick && (s.pos.x.abs() > 6.0 || s.pos.z.abs() > 9.0) {
            let want = atan2f(-s.pos.x, -s.pos.z);
            let err = wrap_angle(want - s.travel_yaw());
            pad.stick_x = (-err * 120.0).clamp(-100.0, 100.0) as i8;
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
                // A quick lean right, straight on past the funbox, then a U-turn to the left back down
                // the park between the kicker and the funbox.
                if s.speed > 3.5 {
                    pad.buttons |= btn::X;
                }
                let turned = wrap_angle(s.travel_yaw() - PI).abs() < 0.25;
                pad.stick_x = if self.t < 12 {
                    85
                } else if s.pos.z < 9.0 && self.t < 200 {
                    -10
                } else if !turned && self.t < 300 {
                    -100
                } else {
                    0
                };
                if turned && self.t > 40 {
                    next(self, Tour::Manual);
                }
            }
            Tour::Manual => {
                // Hold the C-stick lightly back and keep the balance marker centred.
                let b = s.manual.map_or(0.0, |m| m.balance);
                let push = (0.35 - 1.2 * b).clamp(0.02, 0.98);
                let gesture_y = 0.1 + push * 0.8;
                // Undo the stick conditioning: 0.25 dead zone, full at 0.95 of the C-stick's range.
                pad.cstick_y = -((gesture_y * 0.7 + 0.25) * crate::input::C_RANGE) as i8;
                if s.speed < 3.0 && s.manual.is_none() {
                    pad.buttons |= btn::A;
                }
                if self.t > 110 {
                    next(self, Tour::GrabOllie);
                }
            }
            Tour::GrabOllie => {
                // Load, then flick straight up.
                if self.t > 8 {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, &[UP]);
                }
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
                    next(self, Tour::TreFlip);
                }
            }
            Tour::TreFlip => {
                if s.speed < 3.5 && self.t < 40 {
                    pad.buttons |= btn::A;
                }
                // Roll the stick round the back from the left, then flick across to forward-right.
                if self.t > 50 {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(
                        game,
                        rim(260.0),
                        &[rim(215.0), rim(165.0), (39, -19), rim(65.0)],
                    );
                }
                if !rolling {
                    next(self, Tour::TreAir);
                }
            }
            Tour::TreAir => {
                if rolling {
                    next(self, Tour::HalfCab);
                }
            }
            Tour::HalfCab => {
                if s.speed < 3.5 && self.t < 40 {
                    pad.buttons |= btn::A;
                }
                if self.t > 50 {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, &[UP]);
                }
                if !rolling {
                    next(self, Tour::HalfCabAir);
                }
            }
            Tour::HalfCabAir => {
                // Spin left to about half a turn, then let the stick go.
                if s.spin < PI - 0.35 {
                    pad.stick_x = -100;
                }
                if rolling {
                    next(self, Tour::FakiePush);
                }
            }
            Tour::FakiePush => {
                // Rolling tail first: a push turns the rider round to switch.
                if self.t == 20 || (self.t > 40 && s.speed < 3.5) {
                    pad.buttons |= btn::A;
                }
                if self.t > 100 {
                    next(self, Tour::Powerslide);
                }
            }
            Tour::Powerslide => {
                if self.t < 60 && s.speed < 5.5 {
                    pad.buttons |= btn::A;
                } else if self.t >= 60 && self.t < 100 {
                    // Main stick fully back and to the right.
                    (pad.stick_x, pad.stick_y) = (50, -87);
                }
                if self.t > 120 {
                    next(self, Tour::Hippie);
                }
            }
            Tour::Hippie => {
                if self.t < 40 && s.speed < 3.0 {
                    pad.buttons |= btn::A;
                }
                if (50..70).contains(&self.t) {
                    pad.buttons |= btn::A | btn::B;
                }
                if !rolling {
                    next(self, Tour::HippieAir);
                }
            }
            Tour::HippieAir => {
                if rolling {
                    next(self, Tour::BailOllie);
                }
            }
            Tour::BailOllie => {
                if s.speed < 3.0 && self.t < 30 {
                    pad.buttons |= btn::A;
                }
                if self.t > 50 {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, &[UP]);
                }
                if !rolling {
                    next(self, Tour::BailAir);
                }
            }
            Tour::BailAir => {
                // Grab and never let go: still holding the board when the wheels touch down.
                if self.t > 4 {
                    pad.trig_l = 255;
                }
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

/// Hands-off play for scripted runs on the Wii and in Dolphin (`autoplay.txt` in the app folder, see
/// docs/logging.md): the [`TrickTour`] over and over, after the rider is back on the board each time.
#[derive(Default)]
pub struct Autoplay {
    tour: TrickTour,
    /// Tours finished so far.
    pub rounds: u32,
    /// Frames to wait (rolling, no input) before the next tour starts.
    rest: u32,
}

impl Autoplay {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next pad state. Returns the line to log when a tour finishes.
    pub fn next(&mut self, game: &Game) -> (PadRaw, Option<alloc::string::String>) {
        let idle = PadRaw {
            flags: flag::CONNECTED,
            ..Default::default()
        };
        if self.rest > 0 {
            self.rest -= 1;
            // Wait until the rider is up again before starting over.
            if self.rest == 0 && !matches!(game.skater.mode, Mode::Rolling) {
                self.rest = 30;
            }
            return (idle, None);
        }
        let pad = self.tour.next(game);
        if self.tour.done() {
            self.rounds += 1;
            self.tour = TrickTour::new();
            self.rest = 180;
            return (
                idle,
                Some(alloc::format!("autoplay: tour {} done", self.rounds)),
            );
        }
        (pad, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoplay_repeats_the_tour() {
        let mut game = Game::new();
        let mut auto = Autoplay::new();
        let mut lines = 0;
        for _ in 0..(60 * 120) {
            let (pad, line) = auto.next(&game);
            lines += line.is_some() as u32;
            game.update(&pad, 1.0 / 60.0);
            if auto.rounds >= 2 {
                break;
            }
        }
        assert!(auto.rounds >= 2, "rounds {}", auto.rounds);
        assert_eq!(lines, auto.rounds);
    }
}
