//! Skate 3's off-board air body tweak: the OffboardBodyTweakBlend behaviour's sequence, as
//! SK8-ENGINE/skate-3-rust-engine reads it (GPL-3.0; crates/skate-game/src/graph_host/motion_offboard/
//! body_tweak.rs, TU3 Begin `0x82BBACC0`, Update `0x82BBACE0`, End `0x82BBB418`). The runner plays what
//! it asks for on the `AirBodyTweak` channel (`host::CHANNEL_BODY_TWEAK`).
//!
//! In the air off the board (a jump on foot, a hippie jump), the C-stick (`OB_AirBodyTweakX/Y`) pushed
//! past 0.1 plays the tweak's into clip and then its cycle, its `bodytweakx/y` following the stick
//! (filtered: 0.15 of the stick a tick); `OB_DoAirBodyTweak` (Z), or a flight over 4 s, plays it
//! centred. The stick has to be let go first (both axes under 0.1), or 80 ticks pass, before it counts, so
//! a stick already pushed at take-off doesn't tweak. Within 0.1 s of landing nothing new starts; letting
//! go fades the channel out over at most 0.3 s (the time left to land).
//!
//! The comparisons are written negated where SK8's are, so a NaN goes the same way as in Skate 3.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

/// Seconds to land under which the tweak doesn't play (SK8 `0x82BBB030`).
pub const NEAR_LANDING: f32 = 0.1;
/// The stick past this (its length) tweaks.
pub const STICK: f32 = 0.1;
/// Ticks after which the stick counts even if it was never let go.
pub const RELEASE_TICKS: u32 = 80;
/// A flight longer than this (s) tweaks centred by itself.
pub const LONG_FLIGHT: f32 = 4.0;
/// The axes' filter: this much of the old value stays each tick.
pub const KEEP: f32 = 0.85;
/// The channel's fade out when the tweak stops (at most this, else the time left to land), and when
/// the state ends.
pub const FADE_OUT: f32 = 0.3;
pub const END_FADE: f32 = 0.1;

/// One of the behaviour's clips (index into its `[NbCyc, NbInto, BrCyc, BrInto]`).
pub fn clip(board_held: bool, cycle: bool) -> usize {
    usize::from(board_held) * 2 + usize::from(!cycle)
}

/// What the behaviour reads this update.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    /// `OB_AirBodyTweakX/Y` (absent when the graph didn't pass them on).
    pub x: Option<f32>,
    pub y: Option<f32>,
    /// `OB_DoAirBodyTweak`.
    pub request: bool,
    /// The flight's time left (s) and whole length (s).
    pub time_to_land: f32,
    pub air_duration: f32,
    /// The board is in the rider's hand (the `Br` clips).
    pub board_held: bool,
    /// The `AirBodyTweak` channel plays, and its tree's time left (s).
    pub channel: bool,
    pub remaining: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Play this clip (see [`clip`]) on the channel: the into clip (`cycle` false: 0.15 s in, cut) or
    /// the cycle (kept alive, 0.5 s in, 0.3 s out).
    Play { clip: usize, cycle: bool },
    /// End the channel, fading out over this.
    End(f32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Output {
    pub command: Option<Command>,
    /// The tree parameters `bodytweakx`, `bodytweaky` (set while it tweaks).
    pub axes: Option<[f32; 2]>,
    /// Tweaking: SK8 sets the skater animation's flag 0x8000 and seeds a bail's gesture from it.
    pub tweaking: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BodyTweak {
    ticks: u32,
    axes: [f32; 2],
    into_started: bool,
    cycle_started: bool,
    released: bool,
}

impl BodyTweak {
    /// The state begins (the axes keep their last values, as SK8's only zero on allocation).
    pub fn begin(&mut self) {
        self.ticks = 0;
        self.into_started = false;
        self.cycle_started = false;
        self.released = false;
    }

    pub fn update(&mut self, i: Input) -> Output {
        let (directed, centred) = self.gates(i);
        let active = directed || centred;
        let mut out = Output::default();
        if active && !(i.time_to_land < NEAR_LANDING) {
            if i.channel || self.into_started {
                if !(i.remaining > 0.01) && !self.cycle_started {
                    out.command = Some(Command::Play {
                        clip: clip(i.board_held, true),
                        cycle: true,
                    });
                    self.cycle_started = true;
                }
            } else {
                out.command = Some(Command::Play {
                    clip: clip(i.board_held, false),
                    cycle: false,
                });
                self.into_started = true;
            }
            let (x, y) = (i.x.unwrap_or(0.0), i.y.unwrap_or(0.0));
            self.axes = if centred {
                self.axes.map(|v| v * KEEP)
            } else {
                [
                    x * (1.0 - KEEP) + self.axes[0] * KEEP,
                    self.axes[1] * KEEP + y * (1.0 - KEEP),
                ]
            };
            out.axes = Some(self.axes);
            out.tweaking = true;
        } else if !active && i.channel {
            let fade = if i.time_to_land - FADE_OUT >= 0.0 {
                FADE_OUT
            } else {
                i.time_to_land
            };
            out.command = Some(Command::End(fade));
        }
        self.ticks = self.ticks.wrapping_add(1);
        out
    }

    /// (pushed: the stick tweaks, centred: the request or a long flight).
    fn gates(&mut self, i: Input) -> (bool, bool) {
        if self.ticks > RELEASE_TICKS {
            self.released = true;
        }
        // Signed comparisons, as SK8's (not the axes' sizes).
        if !self.released
            && i.x.is_some_and(|x| !(x >= STICK))
            && i.y.is_some_and(|y| !(y >= STICK))
        {
            self.released = true;
        }
        let (x, y) = (i.x.unwrap_or(0.0), i.y.unwrap_or(0.0));
        let len = libm::sqrtf(x * x + y * y);
        let directed = self.released && len >= STICK;
        (
            directed,
            i.request || (!(i.air_duration <= LONG_FLIGHT) && !directed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(x: f32, y: f32) -> Input {
        Input {
            x: Some(x),
            y: Some(y),
            time_to_land: 0.6,
            air_duration: 0.8,
            ..Input::default()
        }
    }

    #[test]
    fn a_stick_pushed_at_take_off_waits_to_be_let_go() {
        let mut t = BodyTweak::default();
        t.begin();
        for _ in 0..10 {
            assert_eq!(t.update(input(0.0, 0.9)).command, None);
        }
        // Let go, then pushed: the into clip, then (once it ends) the cycle.
        t.update(input(0.0, 0.0));
        let o = t.update(input(0.9, 0.0));
        assert_eq!(
            o.command,
            Some(Command::Play {
                clip: 1,
                cycle: false
            })
        );
        assert!(o.tweaking);
        let mut i = input(0.9, 0.0);
        i.channel = true;
        i.remaining = 0.3;
        assert_eq!(t.update(i).command, None);
        i.remaining = 0.0;
        assert_eq!(
            t.update(i).command,
            Some(Command::Play {
                clip: 0,
                cycle: true
            })
        );
        assert_eq!(t.update(i).command, None, "the cycle starts once");
    }

    #[test]
    fn eighty_ticks_count_the_stick_anyway() {
        let mut t = BodyTweak::default();
        t.begin();
        let mut started = None;
        for k in 0..100 {
            if t.update(input(0.0, 0.9)).command.is_some() {
                started = Some(k);
                break;
            }
        }
        assert_eq!(started, Some(RELEASE_TICKS as usize + 1));
    }

    #[test]
    fn the_axes_follow_the_stick_filtered() {
        let mut t = BodyTweak::default();
        t.begin();
        t.update(input(0.0, 0.0));
        let a = t.update(input(1.0, -0.5)).axes.unwrap();
        assert!((a[0] - 0.15).abs() < 1e-6 && (a[1] + 0.075).abs() < 1e-6);
        let mut last = a;
        for _ in 0..60 {
            last = t.update(input(1.0, -0.5)).axes.unwrap();
        }
        assert!((last[0] - 1.0).abs() < 0.01 && (last[1] + 0.5).abs() < 0.01);
    }

    #[test]
    fn z_tweaks_centred_and_the_board_in_hand_picks_its_clips() {
        let mut t = BodyTweak::default();
        t.begin();
        let mut i = input(0.0, 0.0);
        i.request = true;
        i.board_held = true;
        let o = t.update(i);
        assert_eq!(
            o.command,
            Some(Command::Play {
                clip: 3,
                cycle: false
            })
        );
        assert_eq!(o.axes, Some([0.0, 0.0]));
    }

    #[test]
    fn near_landing_nothing_starts_and_letting_go_fades_out() {
        let mut t = BodyTweak::default();
        t.begin();
        t.update(input(0.0, 0.0));
        let mut i = input(0.9, 0.0);
        i.time_to_land = 0.05;
        assert_eq!(t.update(i).command, None);
        assert!(!t.update(i).tweaking);
        let mut i = input(0.0, 0.0);
        i.channel = true;
        i.time_to_land = 0.2;
        assert_eq!(t.update(i).command, Some(Command::End(0.2)));
        i.time_to_land = 1.0;
        assert_eq!(t.update(i).command, Some(Command::End(FADE_OUT)));
    }

    #[test]
    fn a_long_flight_tweaks_by_itself() {
        let mut t = BodyTweak::default();
        t.begin();
        let mut i = input(0.0, 0.0);
        i.air_duration = 4.5;
        assert!(t.update(i).tweaking);
    }
}
