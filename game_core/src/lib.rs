//! flatground game core: everything that is not platform code.
//!
//! `no_std` + `alloc` so the same code runs on the Wii (big-endian PowerPC 750CL) and on the host,
//! where it is unit-tested. It draws only through [`gfx::Gfx`] and reads input only through
//! [`input::PadRaw`]; the Wii side (`wii/game`) and the host preview (`tools/preview`) provide both.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod anim;
pub mod autoplay;
pub mod camera;
pub mod character;
pub mod collision;
pub mod flick;
pub mod game;
pub mod gfx;
pub mod hud;
pub mod input;
pub mod math;
pub mod model;
pub mod pack;
pub mod park;
pub mod rails;
pub mod rider;
pub mod skater;
pub mod skinned;
pub mod world;

pub use game::Game;
