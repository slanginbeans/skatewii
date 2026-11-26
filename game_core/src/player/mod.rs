//! Skate 3's physical player states and the selector that moves between them.
//!
//! Ported from SK8-ENGINE/skate-3-rust-engine (GPL-3.0), crates/skate-core/src/player/state.rs and
//! player/selector/ (TU3 `PhysicalPlayerStateChanger::CalcSuggestedState`, `0x82D8ADE8`). The state
//! ids, the decision tree and its counters are SK8's as they are; `skater::states` builds the
//! selector's input from flatground's physics each tick and runs each state's enter and exit.

pub mod selector;
pub mod state;
