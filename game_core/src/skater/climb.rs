//! Climbing ledges on foot: SK8-ENGINE/skate-3-rust-engine's own extension (GPL-3.0; skate-game
//! physics/climbing/: ledge.rs, approach.rs, mod.rs). Stock Skate 3 has no climbing; the player wants it.
//!
//! - Jumping (B, Skate 3's X) at a wall whose top is 1.25 to 2.75 m over the feet, with a flat top deep
//!   and wide enough to stand on and room over it, the rider reaches for it: the reach builds while he
//!   runs at it and through the jump, and once it is full with the edge within his arms' reach in the
//!   air he catches it (`Phase::Catch`, then `Hang`).
//! - Hanging, B again pulls him up (`Mantle`) and he stands on the top (`Settle`), then walks on.
//! - While he holds the ledge the climb owns his movement (no gravity, no collisions); the stock
//!   graphs stay where they were (SK8 pauses them; its authored hang and mantle clips are its own, not
//!   Skate 3's, so flatground draws the rider in the graphs' last pose moved along the climb).
//!
//! SK8 times the hang and mantle by its authored clips and places the root from their hands and feet;
//! flatground has neither, so the hanging body's hold below the edge, the arms' reach and the mantle's
//! length are its own ([`HANG_DROP`], [`HANG_OUT`], [`SHOULDER_HEIGHT`], [`ARM_REACH`], [`MANTLE`]).

use crate::math::{Vec3, atan2f, expf, v3, wrap_angle};
use crate::park::Terrain;

/// The ledge search on the ground and in the air (SK8 ledge.rs `find_range`): the wall within this far
/// ahead of the chest (1.15 m over the feet), its top this far at least over the feet (m).
const CHEST: f32 = 1.15;
const AIR_REACH: f32 = 2.6;
const AIR_MINIMUM: f32 = 1.25;
/// The highest top looked for over the feet (m).
const HIGHEST: f32 = 2.75;
/// The flying rider's catch: within these heights of the edge over the feet, this close across, this
/// much facing it (m, m, cosine).
const CATCH_LOW: f32 = 0.35;
const CATCH_FAR: f32 = 2.7;
const CATCH_FACING: f32 = 0.65;
/// The reach builds towards its gain over this (s), and the catch wants it this full.
const REACH_TIME: f32 = 0.10;
const REACH_FULL: f32 = 0.95;
/// After a climb (or a teleport) no new reach for this long (s).
const COOLDOWN: f32 = 0.3;
/// Flatground's hanging body: the feet this far under the edge and out from the wall (m).
pub const HANG_DROP: f32 = 2.0;
pub const HANG_OUT: f32 = 0.3;
/// Flatground's arms: the shoulders this high over the feet, this far each side, and the arm's reach
/// from them to the wrist (m).
pub const SHOULDER_HEIGHT: f32 = 1.45;
const SHOULDER_SIDE: f32 = 0.18;
pub const ARM_REACH: f32 = 0.62;
/// The mantle's length (s): SK8's authored clip's; flatground's own. The hands hold the edge through
/// its first 55%, pushing the body up to the edge with the feet this far under it (m), then the body
/// goes over onto the top.
pub const MANTLE: f32 = 1.2;
const MANTLE_HOLD: f32 = 0.55;
const MANTLE_PUSHED: f32 = 0.9;
/// Settling on the top (s).
pub const SETTLE: f32 = 0.25;

/// A ledge found to climb (SK8 ledge.rs `Ledge`): where the hands hold it, where the feet land on top,
/// and the way into the wall (horizontal).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ledge {
    pub anchor: Vec3,
    pub landing: Vec3,
    pub forward: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Catch,
    Hang,
    Mantle,
    Settle,
}

/// Holding a ledge: the phase and its time, the ledge, and where the climb started (the feet and the
/// facing).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attached {
    pub phase: Phase,
    pub time: f32,
    pub ledge: Ledge,
    start: Vec3,
    start_heading: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Approach {
    ledge: Ledge,
    weight: f32,
}

impl Default for Ledge {
    fn default() -> Ledge {
        Ledge {
            anchor: Vec3::ZERO,
            landing: Vec3::ZERO,
            forward: v3(0.0, 0.0, 1.0),
        }
    }
}

/// The climb's state: the ledge held, the reach for one ahead, and the cooldown after a climb.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Climber {
    pub attached: Option<Attached>,
    approach: Option<Approach>,
    cooldown: f32,
}

/// What a climbing tick moved: the feet, the facing, and whether the climb is over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Held {
    pub pos: Vec3,
    pub heading: f32,
    pub done: bool,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t
}

/// The first surface on the segment `a`..`b`, and its normal.
fn hit(park: &dyn Terrain, a: Vec3, b: Vec3) -> Option<(Vec3, Vec3)> {
    let d = b - a;
    let len = d.length();
    if len < 1e-5 {
        return None;
    }
    park.raycast(a, d * (1.0 / len), len)
        .map(|h| (h.point, h.normal))
}

/// A segment swept with a radius (SK8's swept line): the line and four beside it `radius` out.
fn swept(park: &dyn Terrain, a: Vec3, b: Vec3, radius: f32) -> bool {
    if hit(park, a, b).is_some() {
        return true;
    }
    let d = (b - a).normalize_or(Vec3::UP);
    let side = if d.y.abs() > 0.9 {
        v3(1.0, 0.0, 0.0)
    } else {
        d.cross(Vec3::UP).normalize_or(v3(1.0, 0.0, 0.0))
    };
    let other = d.cross(side).normalize_or(Vec3::UP);
    [side, -side, other, -other].into_iter().any(|o| {
        let o = o * radius;
        hit(park, a + o, b + o).is_some()
    })
}

/// A ledge ahead of the feet along `facing` (SK8 ledge.rs `find_range`): a wall within `distance` of
/// the chest, facing back at it (within 0.2 of level, its normal at most -0.65 along the facing), its
/// top between `minimum` and 2.75 m over the feet, flat, under both hands (0.3 m each side), and clear to
/// stand on, with room outside the wall for the body as it rises.
pub fn find(
    park: &dyn Terrain,
    feet: Vec3,
    facing: Vec3,
    minimum: f32,
    distance: f32,
) -> Option<Ledge> {
    let facing = v3(facing.x, 0.0, facing.z);
    if facing.length() < 1e-4 {
        return None;
    }
    let facing = facing.normalize_or(v3(0.0, 0.0, 1.0));
    let chest = feet + Vec3::UP * CHEST;
    let (wall, normal) = hit(park, chest, chest + facing * distance)?;
    if normal.y.abs() > 0.2 || normal.dot(facing) > -CATCH_FACING {
        return None;
    }
    let forward = -v3(normal.x, 0.0, normal.z).normalize_or(facing);
    let right = Vec3::UP.cross(forward);
    let probe = wall + forward * 0.12;
    let (top, up) = hit(
        park,
        v3(probe.x, feet.y + HIGHEST, probe.z),
        v3(probe.x, feet.y + minimum, probe.z),
    )?;
    if up.y < 0.95 {
        return None;
    }
    let anchor = v3(wall.x, top.y + 0.025, wall.z) + forward * 0.055;
    // Both hands need the same ledge, not the tip of a thin pole or a corner.
    for side in [0.30, -0.30] {
        let p = wall + forward * 0.08 + right * side;
        let (p, n) = hit(park, v3(p.x, top.y + 0.15, p.z), v3(p.x, top.y - 0.15, p.z))?;
        if (p.y - top.y).abs() > 0.06 || n.y < 0.95 {
            return None;
        }
    }
    let ledge = Ledge {
        anchor,
        landing: v3(wall.x, top.y + 0.015, wall.z) + forward * 0.62,
        forward,
    };
    if !clear(park, ledge) {
        return None;
    }
    // Space for the torso outside the face, and for the head as it rises.
    let outside = anchor - forward * 0.32;
    if swept(
        park,
        outside - Vec3::UP * 1.25,
        outside + Vec3::UP * 0.55,
        0.20,
    ) {
        return None;
    }
    Some(ledge)
}

/// Room to stand on the top (SK8 ledge.rs `clear`): flat ground within 0.06 m of the landing at it and
/// 0.25 m round it, 1.2 m of headroom over each, and the way over the edge free.
pub fn clear(park: &dyn Terrain, ledge: Ledge) -> bool {
    let right = Vec3::UP.cross(ledge.forward);
    for offset in [
        Vec3::ZERO,
        right * 0.25,
        -right * 0.25,
        ledge.forward * 0.25,
        -ledge.forward * 0.25,
    ] {
        let p = ledge.landing + offset;
        let Some((ground, n)) = hit(park, p + Vec3::UP * 0.15, p - Vec3::UP * 0.15) else {
            return false;
        };
        if (ground.y - ledge.landing.y).abs() > 0.06 || n.y < 0.95 {
            return false;
        }
        if swept(park, p + Vec3::UP * 0.35, p + Vec3::UP * 1.55, 0.28) {
            return false;
        }
    }
    !swept(
        park,
        ledge.anchor + Vec3::UP * 0.65 - ledge.forward * 0.32,
        ledge.landing + Vec3::UP * 0.65,
        0.22,
    )
}

/// SK8 approach.rs `reach_gain`: how far the reach goes at this distance from the edge (none from 2.6
/// m, all within 0.8 m), under half of it on the ground.
pub fn reach_gain(distance: f32, airborne: bool) -> f32 {
    smooth((2.6 - distance) / 1.8) * if airborne { 1.0 } else { 0.45 }
}

/// Where the body hangs from a ledge (the feet: [`HANG_DROP`] under the edge, [`HANG_OUT`] out from
/// the wall, never under the ground there).
fn hang_root(park: &dyn Terrain, l: &Ledge) -> Vec3 {
    let p = l.anchor - Vec3::UP * HANG_DROP - l.forward * HANG_OUT;
    let g = park.ground(p.x, p.z, p.y + 0.5);
    if g.height > f32::MIN / 2.0 && g.height > p.y {
        v3(p.x, g.height, p.z)
    } else {
        p
    }
}

/// Both hands can reach the edge from the shoulders (SK8 approach.rs `reachable`, with flatground's
/// arms): the wrists at the palms 0.3 m each side of the anchor.
fn reachable(feet: Vec3, heading: f32, l: &Ledge) -> bool {
    let right = Vec3::UP.cross(l.forward);
    let side = v3(-libm::cosf(heading), 0.0, libm::sinf(heading));
    [1.0f32, -1.0].into_iter().all(|s| {
        let shoulder = feet + Vec3::UP * SHOULDER_HEIGHT + side * (SHOULDER_SIDE * s);
        let palm = l.anchor + right * (0.30 * s);
        (palm - shoulder).length() <= ARM_REACH - 0.015
    })
}

fn yaw_of(v: Vec3) -> f32 {
    atan2f(v.x, v.z)
}

impl Climber {
    pub fn holding(&self) -> bool {
        self.attached.is_some()
    }

    /// How full the reach for a ledge is (0..1; 0 without one).
    pub fn reach(&self) -> f32 {
        self.approach.map_or(0.0, |a| a.weight)
    }

    /// Drops the climb (a teleport): no reach for a moment.
    pub fn cancel(&mut self) {
        self.attached = None;
        self.approach = None;
        self.cooldown = COOLDOWN;
    }

    /// On foot, after the walker's tick (SK8 approach.rs `update`): looks for a ledge ahead while moving
    /// (the stick's way, else the facing), builds the reach towards it, and in the air with the reach
    /// full and the edge within the arms catches it. `stick` is the stick in the world (x, z).
    #[allow(clippy::too_many_arguments)]
    pub fn approach(
        &mut self,
        park: &dyn Terrain,
        feet: Vec3,
        heading: f32,
        vel: Vec3,
        airborne: bool,
        stick: [f32; 2],
        dt: f32,
    ) {
        self.cooldown = (self.cooldown - dt).max(0.0);
        if self.attached.is_some() {
            return;
        }
        if self.cooldown > 0.0 {
            self.approach = None;
            return;
        }
        let ground_speed = v3(vel.x, 0.0, vel.z).length();
        let pushed = stick[0] * stick[0] + stick[1] * stick[1] > 0.04;
        let direction = if pushed {
            v3(stick[0], 0.0, stick[1])
        } else {
            v3(libm::sinf(heading), 0.0, libm::cosf(heading))
        };
        let moving = airborne || ground_speed > 0.15 || pushed;
        let found = if moving {
            find(park, feet, direction, AIR_MINIMUM, AIR_REACH)
        } else {
            None
        };
        // Follow the edge while running at it; once in the air, keep the one committed to.
        if !airborne
            && let (Some(a), Some(l)) = (&mut self.approach, found)
            && (a.ledge.anchor - l.anchor).length() < 0.5
        {
            a.ledge = l;
        }
        if self.approach.is_none()
            && let Some(l) = found
            && vel.dot(l.forward) >= -0.1
        {
            self.approach = Some(Approach {
                ledge: l,
                weight: 0.0,
            });
        }
        let Some(a) = &mut self.approach else {
            return;
        };
        let separation = a.ledge.anchor - feet;
        let distance = v3(separation.x, 0.0, separation.z).length();
        let valid = moving
            && separation.y > CATCH_LOW
            && separation.y < HIGHEST
            && distance < CATCH_FAR
            && separation.dot(a.ledge.forward) > 0.0
            && direction.normalize_or(Vec3::ZERO).dot(a.ledge.forward) > CATCH_FACING
            && vel.dot(a.ledge.forward) >= -0.3
            && clear(park, a.ledge);
        let desired = if valid {
            reach_gain(distance, airborne)
        } else {
            0.0
        };
        a.weight += (desired - a.weight) * (1.0 - expf(-dt / REACH_TIME));
        if a.weight < 0.001 && !valid {
            self.approach = None;
            return;
        }
        if airborne && valid && a.weight >= REACH_FULL && reachable(feet, heading, &a.ledge) {
            self.attached = Some(Attached {
                phase: Phase::Catch,
                time: 0.0,
                ledge: a.ledge,
                start: feet,
                start_heading: heading,
            });
            self.approach = None;
        }
    }

    /// One tick holding the ledge (SK8 climbing/mod.rs `advance`): the catch eases the body under the
    /// hands, the hang holds it until `pull` (B, with the top still clear) starts the mantle, which
    /// pushes the body up to the edge and over onto the top, where it settles.
    pub fn hold(&mut self, park: &dyn Terrain, pull: bool, dt: f32) -> Option<Held> {
        let a = self.attached.as_mut()?;
        if a.phase == Phase::Hang && pull && clear(park, a.ledge) {
            a.phase = Phase::Mantle;
            a.time = 0.0;
        }
        a.time += dt;
        let l = a.ledge;
        let yaw = yaw_of(l.forward);
        let hang = hang_root(park, &l);
        let (pos, heading, done) = match a.phase {
            Phase::Catch => {
                // A longer catch from further away lowers the body more gently.
                let duration = (0.22 + (a.start - hang).length() * 0.18).min(0.5);
                let t = (a.time / duration).min(1.0);
                let w = smooth(t);
                let heading = a.start_heading + wrap_angle(yaw - a.start_heading) * w;
                if t >= 1.0 {
                    a.phase = Phase::Hang;
                    a.time = 0.0;
                }
                (lerp(a.start, hang, w), heading, false)
            }
            Phase::Hang => (hang, yaw, false),
            Phase::Mantle => {
                let pushed = l.anchor - Vec3::UP * MANTLE_PUSHED + l.forward * 0.05;
                let f = a.time / MANTLE;
                let pos = if f < MANTLE_HOLD {
                    lerp(hang, pushed, smooth(f / MANTLE_HOLD))
                } else {
                    lerp(
                        pushed,
                        l.landing,
                        smooth((f - MANTLE_HOLD) / (1.0 - MANTLE_HOLD)),
                    )
                };
                if a.time >= MANTLE {
                    a.phase = Phase::Settle;
                    a.time = 0.0;
                }
                (pos, yaw, false)
            }
            Phase::Settle => (l.landing, yaw, a.time >= SETTLE),
        };
        if done {
            self.attached = None;
            self.cooldown = COOLDOWN;
        }
        Some(Held {
            pos,
            heading: wrap_angle(heading),
            done,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::{Park, Shape};

    /// The built-in park with a block across +z from z = 0.6 (x -2..2), `height` high and `depth` deep.
    fn wall(height: f32, width: f32, depth: f32) -> Park {
        let mut p = Park::demo();
        p.shapes.clear();
        p.rails.clear();
        p.shapes.push(Shape::Block {
            min: v3(-width, 0.0, 0.6),
            max: v3(width, height, 0.6 + depth),
            rgba: 0,
        });
        p
    }

    #[test]
    fn a_ledge_needs_its_height_a_top_to_stand_on_and_both_hands() {
        let z = v3(0.0, 0.0, 1.0);
        let good = find(&wall(2.2, 2.0, 3.0), Vec3::ZERO, z, 1.90, 0.95).expect("reachable ledge");
        assert!((good.anchor.y - 2.225).abs() < 0.01, "{good:?}");
        assert!((good.forward - z).length() < 1e-3);
        assert!((good.landing.z - (0.6 + 0.62)).abs() < 0.05, "{good:?}");
        assert!(
            find(&wall(4.0, 2.0, 3.0), Vec3::ZERO, z, 1.90, 0.95).is_none(),
            "too high"
        );
        assert!(
            find(&wall(2.2, 0.2, 3.0), Vec3::ZERO, z, 1.90, 0.95).is_none(),
            "too narrow for the hands"
        );
        assert!(
            find(&wall(2.2, 2.0, 0.4), Vec3::ZERO, z, 1.90, 0.95).is_none(),
            "nowhere to stand"
        );
        assert!(
            find(&wall(2.2, 2.0, 3.0), Vec3::ZERO, -z, 1.90, 0.95).is_none(),
            "behind"
        );
    }

    #[test]
    fn the_reach_builds_with_the_distance_and_in_the_air() {
        assert_eq!(reach_gain(2.6, false), 0.0);
        assert!(reach_gain(2.0, false) > 0.0);
        assert!(reach_gain(1.5, false) > reach_gain(2.0, false));
        assert!(reach_gain(0.8, false) <= 0.45);
        assert_eq!(reach_gain(0.8, true), 1.0);
    }

    #[test]
    fn a_held_ledge_catches_hangs_and_mantles_onto_the_top() {
        let park = wall(2.2, 2.0, 3.0);
        let ledge = find(&park, Vec3::ZERO, v3(0.0, 0.0, 1.0), AIR_MINIMUM, AIR_REACH).unwrap();
        let mut c = Climber {
            attached: Some(Attached {
                phase: Phase::Catch,
                time: 0.0,
                ledge,
                start: v3(0.0, 0.6, 0.1),
                start_heading: 0.4,
            }),
            ..Climber::default()
        };
        let dt = 1.0 / 60.0;
        let mut last = None;
        for _ in 0..60 {
            last = c.hold(&park, false, dt);
        }
        assert_eq!(
            c.attached.unwrap().phase,
            Phase::Hang,
            "hangs, waiting for B"
        );
        let h = last.unwrap();
        assert!(h.heading.abs() < 1e-4, "faces the wall");
        assert!(h.pos.y >= 0.0 && h.pos.y < ledge.anchor.y - 1.0, "{h:?}");
        c.hold(&park, true, dt);
        assert_eq!(c.attached.unwrap().phase, Phase::Mantle);
        let mut done = None;
        for i in 0..200 {
            let h = c.hold(&park, false, dt).unwrap();
            if h.done {
                done = Some((i, h));
                break;
            }
        }
        let (ticks, h) = done.expect("over the top");
        assert!(
            (ticks as f32 * dt - (MANTLE + SETTLE)).abs() < 0.05,
            "{ticks}"
        );
        assert!((h.pos - ledge.landing).length() < 1e-4, "{h:?}");
        assert!(c.attached.is_none() && !c.holding());
    }
}
