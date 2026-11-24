//! ToggleBoard's state machine: Skate 3's off-board drop, throw and retrieve animation sequencing (TU3
//! 82BA8FE8, retrieval 82BA9310, drop 82BA9990), as SK8-ENGINE/skate-3-rust-engine ports it (GPL-3.0;
//! skate-core player/offboard/toggle_board.rs). The host (`graph::host`) plays the clips it names on the
//! `RetrieveBoard` channel and publishes its flags; the physics (`skater::possession`) lets go of the
//! board at the throw clips' `OB_RELEASEBOARD` / `OB_THROWBOARD` and starts flying it back at each
//! `OB_RetrieveBoard`.

/// Where the sequence is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    RetrieveStart,
    RetrieveInto,
    RetrieveCycle,
    RetrieveOut,
    DropStart,
    DropPlaying,
}

/// What it reads each update: the possession's publication and the motion graph's requests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub holding_board: bool,
    /// The board is flying back to the hands (SK8 OffBoard313, not the retrieving flag).
    pub retrieval_active: bool,
    pub drop_requested: bool,
    pub throw_requested: bool,
    pub retrieve_requested: bool,
    /// The board's direction from the rider (rad, + to his right) and its pitch.
    pub yaw_radians: f32,
    pub pitch_radians: f32,
    pub mirrored: bool,
}

/// The `RetrieveBoard` channel as it is now.
#[derive(Clone, Copy, Debug, Default)]
pub struct Channel {
    pub exists: bool,
    pub remaining: f32,
    pub elapsed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    Throw,
    Drop,
    FrontInto,
    FrontCycle,
    FrontOut,
    BackInto,
    BackCycle,
    BackOut,
}

/// Every clip or tree it plays (the importer converts their poses: they are named here, not in the
/// graph).
pub const CLIPS: [&str; 8] = [
    "OFB_THROW_0",
    "OFB_THROW_90",
    "OFB_RETRIEVE_HIGH_FRONT_INTO",
    "OFB_RETRIEVE_HIGH_FRONT_CYC",
    "OFB_RETRIEVE_HIGH_FRONT_OUT",
    "OFB_RETRIEVE_HIGH_BACK_INTO",
    "OFB_RETRIEVE_HIGH_BACK_CYC",
    "OFB_RETRIEVE_HIGH_BACK_OUT",
];

impl Clip {
    /// The drop plays `THROW_90` (SK8: as the original's constructor picks it).
    pub fn name(self) -> &'static str {
        CLIPS[match self {
            Self::Throw => 0,
            Self::Drop => 1,
            Self::FrontInto => 2,
            Self::FrontCycle => 3,
            Self::FrontOut => 4,
            Self::BackInto => 5,
            Self::BackCycle => 6,
            Self::BackOut => 7,
        }]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// A blended transition to the clip (a new channel if there is none).
    BlendTo(Clip),
    /// A sequence (no blend) to it on the playing channel.
    SequenceTo(Clip),
    Stop {
        blend_seconds: f32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Output {
    /// `OB_RetrievingBoard` and `OB_DroppingBoard` this update.
    pub retrieving: bool,
    pub dropping: bool,
    /// `OB_RetrieveBoard`, once per retrieve cycle: the physics flies the board back.
    pub retrieve: bool,
    /// The retrieve trees' `yaw` and `pitch` (degrees).
    pub yaw_pitch: Option<[f32; 2]>,
    pub channel: Option<Command>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ToggleBoard {
    pub phase: Phase,
    previous_yaw: f32,
    yaw: f32,
    pitch: f32,
    initialized_yaw: bool,
    back: bool,
    throwing: bool,
}

impl ToggleBoard {
    /// Begin: idle, orientation reset.
    pub fn begin(&mut self) {
        self.phase = Phase::Idle;
        self.reset_orientation();
    }

    fn reset_orientation(&mut self) {
        self.previous_yaw = 0.0;
        self.yaw = 0.0;
        self.pitch = 0.0;
        self.initialized_yaw = false;
        self.back = false;
    }

    /// The retrieve clips' aim: the board's direction in degrees, behind the rider folded onto his
    /// side, moving at most 5 degrees an update.
    fn orientation(&mut self, input: Input) {
        let mut yaw = input.yaw_radians * -57.295_776;
        if input.mirrored {
            yaw = -yaw;
        }
        if yaw < -90.0 {
            yaw = if self.initialized_yaw {
                if self.yaw >= 0.0 { 180.0 } else { -90.0 }
            } else if yaw >= -135.0 {
                -90.0
            } else {
                180.0
            };
        }
        if input.mirrored {
            yaw = -yaw;
        }
        if self.initialized_yaw {
            let step = (yaw - self.previous_yaw).clamp(-5.0, 5.0);
            yaw = self.previous_yaw + step;
        }
        self.previous_yaw = yaw;
        self.yaw = if input.mirrored { -yaw } else { yaw };
        self.pitch = input.pitch_radians * 57.295_776;
        self.initialized_yaw = true;
        self.back = self.yaw.abs() > 90.0;
    }

    pub fn update(&mut self, input: Input, channel: Channel) -> Output {
        if self.phase == Phase::Idle {
            if input.drop_requested && input.holding_board {
                self.phase = Phase::DropStart;
                self.throwing = false;
            } else if input.throw_requested && input.holding_board {
                self.phase = Phase::DropStart;
                self.throwing = true;
            } else if input.retrieve_requested && !input.holding_board {
                self.phase = Phase::RetrieveStart;
            }
        }
        let mut out = Output::default();
        match self.phase {
            Phase::Idle => self.reset_orientation(),
            Phase::DropStart => {
                out.dropping = true;
                self.phase = Phase::DropPlaying;
                out.channel = Some(Command::BlendTo(if self.throwing {
                    Clip::Throw
                } else {
                    Clip::Drop
                }));
            }
            Phase::DropPlaying => {
                out.dropping = true;
                if !channel.exists || channel.remaining <= 0.0 || channel.elapsed > 0.5 {
                    self.stop(&mut out);
                }
            }
            phase => {
                out.retrieving = true;
                match phase {
                    Phase::RetrieveStart => {
                        self.orientation(input);
                        self.phase = Phase::RetrieveInto;
                        out.channel = Some(Command::BlendTo(if self.back {
                            Clip::BackInto
                        } else {
                            Clip::FrontInto
                        }));
                    }
                    Phase::RetrieveInto => {
                        self.orientation(input);
                        if !channel.exists || channel.remaining <= 0.0 {
                            self.phase = Phase::Idle;
                        } else if channel.remaining <= 0.25 {
                            self.cycle(&mut out);
                        }
                    }
                    Phase::RetrieveCycle => {
                        self.orientation(input);
                        // (SK8 stops here on `!retrieval_active` alone, before the held test: with the
                        // possession published after its update, the board coming into the hands ends
                        // the flight and the hold together, which would never reach the `OUT` clip.
                        // flatground lets the hold through.)
                        if !input.retrieval_active && !input.holding_board {
                            self.stop(&mut out);
                        } else if !channel.exists || channel.remaining <= 0.0 {
                            self.phase = Phase::Idle;
                        } else if input.holding_board {
                            self.phase = Phase::RetrieveOut;
                            out.channel = Some(Command::BlendTo(if self.back {
                                Clip::BackOut
                            } else {
                                Clip::FrontOut
                            }));
                        } else if channel.remaining <= 0.25 {
                            self.cycle(&mut out);
                        }
                    }
                    _ => {
                        if !channel.exists
                            || channel.remaining <= 0.0
                            || channel.elapsed > 1.0 / 6.0
                        {
                            self.stop(&mut out);
                        }
                    }
                }
                out.yaw_pitch = Some([self.yaw, self.pitch]);
            }
        }
        out
    }

    fn cycle(&mut self, out: &mut Output) {
        self.phase = Phase::RetrieveCycle;
        out.channel = Some(Command::SequenceTo(if self.back {
            Clip::BackCycle
        } else {
            Clip::FrontCycle
        }));
        out.retrieve = true;
    }

    fn stop(&mut self, out: &mut Output) {
        self.phase = Phase::Idle;
        out.channel = Some(Command::Stop {
            blend_seconds: 1.0 / 6.0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(remaining: f32, elapsed: f32) -> Channel {
        Channel {
            exists: true,
            remaining,
            elapsed,
        }
    }

    #[test]
    fn a_drop_plays_throw_90_and_a_throw_throw_0_until_half_a_second() {
        for (throw, clip) in [(false, Clip::Drop), (true, Clip::Throw)] {
            let mut t = ToggleBoard::default();
            t.begin();
            let input = Input {
                holding_board: true,
                drop_requested: !throw,
                throw_requested: throw,
                ..Input::default()
            };
            let out = t.update(input, Channel::default());
            assert!(out.dropping);
            assert_eq!(out.channel, Some(Command::BlendTo(clip)));
            let held = Input {
                holding_board: false,
                ..Input::default()
            };
            let out = t.update(held, playing(0.5, 0.2));
            assert!(out.dropping && out.channel.is_none());
            let out = t.update(held, playing(0.3, 0.51));
            assert!(matches!(out.channel, Some(Command::Stop { .. })));
            assert_eq!(t.phase, Phase::Idle);
        }
        assert_eq!(Clip::Drop.name(), "OFB_THROW_90");
    }

    #[test]
    fn a_retrieve_aims_cycles_publishes_and_comes_out_once_held() {
        let mut t = ToggleBoard::default();
        let mut input = Input {
            retrieve_requested: true,
            // Behind the rider, to his left.
            yaw_radians: -2.8,
            ..Input::default()
        };
        let out = t.update(input, Channel::default());
        assert!(out.retrieving);
        assert_eq!(out.channel, Some(Command::BlendTo(Clip::BackInto)));
        input.retrieve_requested = false;
        let out = t.update(input, playing(0.2, 0.3));
        assert_eq!(out.channel, Some(Command::SequenceTo(Clip::BackCycle)));
        assert!(out.retrieve, "the physics starts flying it back");
        input.retrieval_active = true;
        let out = t.update(input, playing(0.8, 0.0));
        assert!(!out.retrieve && out.channel.is_none());
        input.holding_board = true;
        input.retrieval_active = false;
        let out = t.update(input, playing(0.6, 0.2));
        assert_eq!(out.channel, Some(Command::BlendTo(Clip::BackOut)));
        let out = t.update(input, playing(0.5, 0.2));
        assert!(matches!(out.channel, Some(Command::Stop { .. })));
    }

    #[test]
    fn the_aim_turns_at_most_five_degrees_an_update() {
        let mut t = ToggleBoard::default();
        let mut input = Input {
            retrieve_requested: true,
            yaw_radians: 0.0,
            ..Input::default()
        };
        t.update(input, Channel::default());
        input.yaw_radians = -1.0;
        let out = t.update(input, playing(1.0, 0.0));
        assert_eq!(out.yaw_pitch.map(|y| y[0]), Some(5.0));
    }
}
