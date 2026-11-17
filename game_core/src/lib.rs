//! flatground game core: everything that is not platform code.
//!
//! `no_std` + `alloc` so the same code runs on the Wii (big-endian PowerPC 750CL) and on the host,
//! where it is unit-tested. It draws only through [`gfx::Gfx`] and reads input only through
//! [`input::PadRaw`]; the Wii side (`wii/game`) and the host preview (`tools/preview`) provide both.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod autoplay;
pub mod camera;
pub mod flick;
pub mod game;
pub mod gfx;
pub mod hud;
pub mod input;
pub mod math;
pub mod model;
pub mod park;
pub mod skater;

pub use game::Game;
