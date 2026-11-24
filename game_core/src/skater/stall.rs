//! Coping stalls (lip tricks): a grind on a ramp's coping held still and come back into the ramp.
//!
//! Skate 3 has no stall states of its own: its graphs play the grind the board is in on the coping (an
//! axle stall is its `FS_50_50`/`BS_50_50`, a blunt stall its `BS_BLUNT` ...), its physics holds the
//! board on the coping on the way up (`TimeOfGravityRelief...`: `skater::grind`'s coping hold) and lets
//! it fall back in when it stops, and its scoring catalogue has identifiers for some of them
//! (`fakie_rock`, `fs_rock_n_roll`, `bs_rock_n_roll`, SK8 `scoring/catalog.rs`) with no records on the
//! disc. flatground names the stall by the grind it is (the board's middle on the coping with its nose
//! over the deck is a rock, with its nose back over the ramp a disaster) and scores it with the
//! catalogue's own identifier where the rules pack has a record for it, else as the grind Skate 3 scores.
//!
//! A rock comes back in fakie; the main stick held to the side as it falls back turns the board round
//! (a rock and roll, flatground's control). The truck stalls turn back down the ramp.

use alloc::format;
use alloc::string::String;

use super::Family;
use crate::math::{Vec3, v3};
use crate::park::Terrain;

/// Slower than this along the coping all the way through, the grind is a stall (m/s; flatground's).
pub const STALL_SPEED: f32 = 1.0;
/// The main stick this far to the side as a rock falls back in turns the board round (flatground's).
pub const ROLL_STICK: f32 = 0.5;
/// A ledge is a coping when the ground this far out from it on its low side (m) is this far under it (m)
/// and sloping (a transition: its normal's y under this).
const COPING_OUT: f32 = 0.4;
const COPING_DROP: f32 = 0.3;
const COPING_SLOPE: f32 = 0.97;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stall {
    /// Both trucks on the coping (a 50-50).
    Axle,
    /// The back truck (a 5-0), the front truck (a nosegrind).
    FiveO,
    Nosegrind,
    /// A tip on the coping, the board out over the ramp: the nose or the tail.
    Nose,
    Tail,
    /// A tip on the coping with the board up over the deck (a blunt, a noseblunt).
    Blunt,
    Noseblunt,
    /// The board's middle on the coping, its nose over the deck: back in fakie, or turned round.
    RockToFakie,
    RockAndRoll,
    /// The board's middle on the coping with its nose back over the ramp (turned round in the air).
    Disaster,
}

impl Stall {
    /// The stall a coping grind is: its family, the truck or tip on the coping, whether the nose points
    /// over the deck (`high`: the deck's side), and whether the board is turned round on the way back in.
    pub fn of(family: Family, front: bool, fwd: Vec3, high: Vec3, turned: bool) -> Option<Stall> {
        let nose_over_deck = fwd.dot(high) > 0.0;
        Some(match family {
            Family::FiftyFifty => Stall::Axle,
            Family::FiveO if front => Stall::Nosegrind,
            Family::FiveO => Stall::FiveO,
            Family::Tipslide if front => Stall::Nose,
            Family::Tipslide => Stall::Tail,
            Family::Backslash if front => Stall::Noseblunt,
            Family::Backslash => Stall::Blunt,
            Family::Boardslide if !nose_over_deck => Stall::Disaster,
            Family::Boardslide if turned => Stall::RockAndRoll,
            Family::Boardslide => Stall::RockToFakie,
            Family::Darkslide => return None,
        })
    }

    /// As shown: `FS AXLE STALL`, `ROCK TO FAKIE`.
    pub fn display(self, frontside: bool) -> String {
        let side = if frontside { "FS" } else { "BS" };
        let base = match self {
            Stall::Axle => "AXLE STALL",
            Stall::FiveO => "5-0 STALL",
            Stall::Nosegrind => "NOSEGRIND STALL",
            Stall::Nose => "NOSE STALL",
            Stall::Tail => "TAIL STALL",
            Stall::Blunt => "BLUNT STALL",
            Stall::Noseblunt => "NOSEBLUNT STALL",
            Stall::RockToFakie => return String::from("ROCK TO FAKIE"),
            Stall::RockAndRoll => "ROCK N ROLL",
            Stall::Disaster => "DISASTER",
        };
        format!("{side} {base}")
    }

    /// Skate 3's identifier for it, where its catalogue has one (`fakie_rock`, `fs_rock_n_roll` ...).
    pub fn id(self, frontside: bool) -> Option<u16> {
        use crate::scoring::catalog::id;
        match self {
            Stall::RockToFakie => id("fakie_rock"),
            Stall::RockAndRoll => id(if frontside {
                "fs_rock_n_roll"
            } else {
                "bs_rock_n_roll"
            }),
            _ => None,
        }
    }

    /// Back into the ramp: the truck stalls and a rock and roll turn round to ride down it, the rest
    /// come back as they are (a rock and the blunts fakie).
    pub fn turns_down(self) -> bool {
        matches!(
            self,
            Stall::Axle | Stall::FiveO | Stall::Nosegrind | Stall::RockAndRoll
        )
    }
}

/// A ledge rail at `point` (its high side `high`) is a ramp's coping: out from its low side the ground
/// falls away into a transition.
pub fn is_coping(park: &dyn Terrain, point: Vec3, high: Vec3) -> bool {
    let out = v3(-high.x, 0.0, -high.z).normalize_or(Vec3::ZERO);
    if out == Vec3::ZERO {
        return false;
    }
    let probe = point + out * COPING_OUT;
    let g = park.ground(probe.x, probe.z, point.y + 0.05);
    g.height > f32::MIN / 2.0 && point.y - g.height > COPING_DROP && g.normal.y < COPING_SLOPE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::{Axis, Park, Shape};
    use alloc::vec;

    #[test]
    fn stalls_are_named_by_the_grind_on_the_coping() {
        let high = v3(0.0, 0.0, 1.0);
        let up = v3(0.0, 0.0, 1.0);
        let along = v3(1.0, 0.0, 0.0);
        assert_eq!(
            Stall::of(Family::FiftyFifty, false, along, high, false),
            Some(Stall::Axle)
        );
        assert_eq!(
            Stall::of(Family::Boardslide, false, up, high, false),
            Some(Stall::RockToFakie)
        );
        assert_eq!(
            Stall::of(Family::Boardslide, false, up, high, true),
            Some(Stall::RockAndRoll)
        );
        assert_eq!(
            Stall::of(Family::Boardslide, false, -up, high, false),
            Some(Stall::Disaster)
        );
        assert_eq!(
            Stall::of(Family::Backslash, false, up, high, false),
            Some(Stall::Blunt)
        );
        assert_eq!(Stall::of(Family::Darkslide, false, up, high, false), None);
        assert_eq!(Stall::Axle.display(true), "FS AXLE STALL");
        assert_eq!(Stall::RockToFakie.display(false), "ROCK TO FAKIE");
        assert_eq!(
            Stall::RockToFakie.id(true),
            crate::scoring::catalog::id("fakie_rock")
        );
        assert!(Stall::RockAndRoll.id(false).is_some() && Stall::Axle.id(true).is_none());
    }

    #[test]
    fn a_coping_has_a_transition_under_it_and_a_ledge_does_not() {
        let block = Shape::Block {
            min: v3(-5.0, 0.0, 2.0),
            max: v3(5.0, 1.5, 6.0),
            rgba: 0,
        };
        let ramp = Shape::Ramp {
            min: v3(-5.0, 0.0, 0.0),
            max: v3(5.0, 0.0, 2.0),
            axis: Axis::Z,
            h0: 0.0,
            h1: 1.49,
            rgba: 0,
        };
        let park = |shapes| Park {
            shapes,
            half_size: 20.0,
            wall_height: 2.0,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
            rails: vec![],
        };
        let high = v3(0.0, 0.0, 1.0);
        assert!(is_coping(&park(vec![block, ramp]), v3(0.0, 1.5, 2.0), high));
        assert!(!is_coping(&park(vec![block]), v3(0.0, 1.5, 2.0), high));
    }
}
