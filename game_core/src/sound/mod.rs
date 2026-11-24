//! Sound: what the riding sounds like, as commands for a platform mixer.
//!
//! - [`director`] watches the simulation after each displayed frame (its events and state, never feeding
//!   back into it) and decides what plays: the wheels' rolling loop pitched and levelled by speed and
//!   picked by the surface under them, pops, landings by how hard and how clean they were, grind and
//!   slide loops by family and rail, pushes, the foot brake, powerslides, catches, bails, footsteps off
//!   the board and the board hitting the ground on its own. Its output is a list of [`Command`]s.
//! - The platform takes them through the [`Audio`] trait (as it draws through `gfx::Gfx`). The Wii and
//!   the host preview both mix them with [`mix::Mixer`] (32 kHz stereo); tests record them.
//! - [`bank::Bank`] holds the sounds: Skate 3's own, converted from the player's disc by `fg_import`
//!   into `sound.fgpak` (DSP-ADPCM, [`adpcm`]), else simple placeholders synthesised at start
//!   ([`synth`]).
//!
//! Levels, timings and the surface table follow SK8-ENGINE/skate-3-rust-engine's audio work
//! (`docs/hails-additions/11-audio.md`, GPL-3.0; numbers only).

pub mod adpcm;
pub mod bank;
pub mod director;
pub mod mix;
pub mod synth;

pub use bank::Bank;
pub use director::Director;
pub use mix::Mixer;

/// The mix's sample rate (Hz): the imported sounds are converted to it, so most voices play 1:1.
pub const RATE: u32 = 32_000;

/// A sound slot: what the director asks for. Each has one or more clips (variations, or a rolling
/// loop's speed bands) in the [`Bank`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Cue {
    /// The wheels rolling, by surface (Skate 3's grains); the clips are speed bands, slow to fast.
    RollAsphaltSmooth,
    RollAsphaltRough,
    RollConcreteSmooth,
    RollConcreteRough,
    RollConcreteAggregate,
    RollWood,
    RollMetal,
    /// The tail striking the ground for an ollie.
    Pop,
    /// Wheels coming down, and the heavy thud under a big landing.
    Land,
    LandHeavy,
    /// Feet catching the board after a flip.
    Catch,
    /// Stepping onto the board.
    BoardOn,
    /// The board hitting the ground on its own (dropped, thrown, after a bail).
    BoardDrop,
    /// Grind loops: metal (handrails, thin rails) and ledges (concrete, wood).
    GrindMetal,
    GrindLedge,
    /// The deck scraping on at the start of a slide.
    SlideScrape,
    /// Wheels skidding sideways (powerslides, sketchy landings): short pieces.
    Skid,
    /// The shoe dragging on the foot brake: short pieces.
    Brake,
    /// Footsteps (and the pushing foot), and running steps.
    Step,
    RunStep,
    /// The body hitting and sliding in a bail.
    Bail,
    /// The board flipping.
    Flip,
    /// The wheels spinning free in the air.
    WheelSpin,
}

impl Cue {
    pub const COUNT: usize = 23;
    pub const ALL: [Cue; Cue::COUNT] = [
        Cue::RollAsphaltSmooth,
        Cue::RollAsphaltRough,
        Cue::RollConcreteSmooth,
        Cue::RollConcreteRough,
        Cue::RollConcreteAggregate,
        Cue::RollWood,
        Cue::RollMetal,
        Cue::Pop,
        Cue::Land,
        Cue::LandHeavy,
        Cue::Catch,
        Cue::BoardOn,
        Cue::BoardDrop,
        Cue::GrindMetal,
        Cue::GrindLedge,
        Cue::SlideScrape,
        Cue::Skid,
        Cue::Brake,
        Cue::Step,
        Cue::RunStep,
        Cue::Bail,
        Cue::Flip,
        Cue::WheelSpin,
    ];

    /// The name in a sound pack: its clips are `sound/<name>/<n>`.
    pub const fn name(self) -> &'static str {
        match self {
            Cue::RollAsphaltSmooth => "roll_asphalt_smooth",
            Cue::RollAsphaltRough => "roll_asphalt_rough",
            Cue::RollConcreteSmooth => "roll_concrete_smooth",
            Cue::RollConcreteRough => "roll_concrete_rough",
            Cue::RollConcreteAggregate => "roll_concrete_aggregate",
            Cue::RollWood => "roll_wood",
            Cue::RollMetal => "roll_metal",
            Cue::Pop => "pop",
            Cue::Land => "land",
            Cue::LandHeavy => "land_heavy",
            Cue::Catch => "catch",
            Cue::BoardOn => "board_on",
            Cue::BoardDrop => "board_drop",
            Cue::GrindMetal => "grind_metal",
            Cue::GrindLedge => "grind_ledge",
            Cue::SlideScrape => "slide_scrape",
            Cue::Skid => "skid",
            Cue::Brake => "brake",
            Cue::Step => "step",
            Cue::RunStep => "run_step",
            Cue::Bail => "bail",
            Cue::Flip => "flip",
            Cue::WheelSpin => "wheel_spin",
        }
    }

    pub fn from_name(name: &str) -> Option<Cue> {
        Cue::ALL.iter().copied().find(|c| c.name() == name)
    }

    /// A rolling loop.
    pub const fn is_roll(self) -> bool {
        (self as u8) <= Cue::RollMetal as u8
    }
}

/// The rolling loops' speed bands (clips per rolling cue): Skate 3's grains are slow-to-fast sweeps, cut
/// into this many loops.
pub const ROLL_BANDS: usize = 6;

/// What the platform's mixer is told, once a displayed frame. Voices are named by the director (never 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Start clip `clip` of the bank on `voice`: `gain` (linear, 1 = as recorded), `pan` (-1 left .. 1
    /// right), `pitch` (playback speed, 1 = as recorded), reaching the gain over `fade` seconds (0: at
    /// once, so an impact keeps its attack). A clip with a loop loops until stopped.
    Play {
        voice: u16,
        clip: u16,
        gain: f32,
        pan: f32,
        pitch: f32,
        fade: f32,
    },
    /// Change a playing voice (eased over a few milliseconds, so no clicks).
    Set {
        voice: u16,
        gain: f32,
        pan: f32,
        pitch: f32,
    },
    /// Fade a voice out over `fade` seconds and let it go.
    Stop { voice: u16, fade: f32 },
}

/// The platform's sound output: takes each frame's commands with the bank they name clips of, and the
/// master volume (0..1, the pause menu's). It must not affect the simulation in any way.
pub trait Audio {
    fn frame(&mut self, bank: &Bank, commands: &[Command], master: f32);
}

/// No sound (tests, tools).
pub struct NullAudio;

impl Audio for NullAudio {
    fn frame(&mut self, _: &Bank, _: &[Command], _: f32) {}
}

/// Keeps every command (tests): each frame's list, with the master volume.
#[derive(Default)]
pub struct Recorder {
    pub frames: alloc::vec::Vec<(alloc::vec::Vec<Command>, f32)>,
}

impl Audio for Recorder {
    fn frame(&mut self, _: &Bank, commands: &[Command], master: f32) {
        self.frames.push((commands.to_vec(), master));
    }
}

/// The master volume steps of the pause menu (0..=10), and its default.
pub const VOLUME_STEPS: u8 = 10;
/// 8 of 10: SK8 found unity gain (retail samples peak near full scale) too hot as a default and ships 75 %.
pub const DEFAULT_VOLUME: u8 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cue_names_round_trip() {
        for (i, c) in Cue::ALL.iter().enumerate() {
            assert_eq!(*c as usize, i);
            assert_eq!(Cue::from_name(c.name()), Some(*c));
        }
        assert!(Cue::RollMetal.is_roll() && !Cue::Pop.is_roll());
        assert_eq!(Cue::from_name("nope"), None);
    }
}
