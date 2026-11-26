//! Dropping in on foot, as Skate 3 does it: standing at the edge of a ramp with the board in hand, Y
//! (Skate 3's Y, `ToggleOffBoardState`) sets the board's tail on the coping and the rider stands on it
//! while Y is held; letting go drops in, pulling the stick back steps away again.
//!
//! Skate 3's motion graph plays it (`Mount/Ground/IntoDropIn`: entered while standing, the board in hand,
//! `DistToEdge` 1.5 m or nearer; its `Into` steps up to the edge with a big step over 1 m, a small one
//! over 0.4 m, or none; its `Cycle` stands there while `OB_MountRaw` is held, `TryToLeave` with the stick
//! at `OB_SteerMagnitude` -0.5 or under goes `BackToLoco`; let go, `OutOfDropIn` goes onto the board and,
//! 0.2 s in, into `OnBoard/Grinding/DroppingIn`, which needs the physics in a grind with
//! `IsDroppingIn`), and Skate 3's physics holds the board's tail on the coping as a tip slide with its
//! drop-in flag (SK8 grind output byte 324; `graph_host/action_conditions.rs`' `IsDroppingIn`) until it
//! drops in. SK8's off-board edge distance (`player/offboard/contact_segments.rs`: along the rider's
//! facing to the end of the surface he stands on) is [`find_edge`] here.
//!
//! The drop itself is flatground's rolling drop-in (`skater::grind`'s drop-in admission): the board let go
//! just behind the coping, rolling slowly over it into the transition.

use crate::math::{Vec3, v3};
use crate::park::Terrain;

/// Skate 3's motion graph (`Mount/Ground/IntoDropIn`): a drop-in starts this near the edge (m,
/// `DistToEdge` lessEqual 1.5), with a big step from further than 1 m and a small one from further than
/// 0.4 m.
pub const ENTER_DISTANCE: f32 = 1.5;
pub const BIG_STEP: f32 = 1.0;
pub const SMALL_STEP: f32 = 0.4;
/// Skate 3's motion graph: `OutOfDropIn` goes into `Grinding/DroppingIn` 0.2 s in (`WillExpire` 0.2).
pub const OUT_TIME: f32 = 0.2;
/// Skate 3's `TryToLeave`: the stick this far back (`OB_SteerMagnitude` lessEqual -0.5) steps away.
pub const BACK_OUT: f32 = -0.5;
/// How long the step up to the edge takes at most before the motion graph's `IntoDropIn/Cycle` (s, plus
/// a second; flatground's, the clips' lengths are the player's data): a big step, a small one, none.
pub const INTO_TIMES: [f32; 3] = [0.8, 0.6, 0.4];
/// How long the board stays on the coping in `Grinding/DroppingIn` before it goes (s; flatground's, the
/// `DroppingIn` clip's blend time), and it goes anyway this long after `OutOfDropIn` started (s).
pub const LOCK_TIME: f32 = 0.4;
pub const LOCK_LIMIT: f32 = 2.0;
/// The rider stands this far back from the edge (m), the board's middle this far back on the deck while
/// its tail is on the coping, and it is let go this far back, rolling at this speed (m/s: under SK8's
/// drop-in admission's 1.45).
pub const STAND_BACK: f32 = 0.15;
pub const BOARD_BACK: f32 = 0.3;
pub const RELEASE_BACK: f32 = 0.1;
pub const RELEASE_SPEED: f32 = 1.2;
/// The edge search (flatground's): along the facing this far (m) in steps of this (m), the ground falling
/// away by more than this (m), or falling away steeper than this (its normal's y under it), is the edge;
/// rising by more than this (m) is a wall.
const REACH: f32 = 2.0;
const STEP: f32 = 0.1;
const EDGE_DROP: f32 = 0.3;
const EDGE_STEEP: f32 = 0.85;
const WALL: f32 = 0.3;
/// A transition under the edge: ground within this far past it (m) sloping (its normal's y between
/// these).
const TRANSITION_REACH: f32 = 1.0;
const TRANSITION_UP: (f32, f32) = (0.02, 0.97);
/// A rail this near the edge (m) is its coping.
const COPING_NEAR: f32 = 0.35;

/// The edge of the surface the rider stands on, ahead along his facing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    /// On the edge at the feet's height, how far ahead it is (m), and the way over it (horizontal unit).
    pub point: Vec3,
    pub dist: f32,
    pub dir: Vec3,
    /// A ramp under it (else a plain drop: `IsDropInWithoutTransition`), and the rail along it.
    pub transition: bool,
    pub rail: Option<usize>,
}

/// The edge ahead of `pos` (the feet) along `facing`, if the ground falls away within reach.
pub fn find_edge(park: &dyn Terrain, pos: Vec3, facing: Vec3) -> Option<Edge> {
    let dir = v3(facing.x, 0.0, facing.z).normalize_or(Vec3::ZERO);
    if dir == Vec3::ZERO {
        return None;
    }
    let h0 = pos.y;
    let ground = |d: f32| {
        let p = pos + dir * d;
        park.ground(p.x, p.z, h0 + WALL)
    };
    let below = |g: crate::park::Surface| {
        g.height < h0 - EDGE_DROP || (g.height < h0 - 0.01 && g.normal.y < EDGE_STEEP)
    };
    let n = (REACH / STEP) as usize;
    let mut last = 0.0;
    let mut edge = None;
    for i in 1..=n {
        let d = i as f32 * STEP;
        let g = ground(d);
        if g.height > h0 + WALL {
            return None;
        }
        if below(g) {
            // Narrow it down.
            let (mut lo, mut hi) = (last, d);
            for _ in 0..5 {
                let mid = (lo + hi) * 0.5;
                if below(ground(mid)) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            edge = Some(lo);
            break;
        }
        last = d;
    }
    let dist = edge?;
    let point = pos + dir * dist;
    let transition = (1..=5).any(|k| {
        let g = ground(dist + TRANSITION_REACH * k as f32 / 5.0);
        g.height > h0 - 6.0
            && g.height < h0 - 0.05
            && g.normal.y > TRANSITION_UP.0
            && g.normal.y < TRANSITION_UP.1
    });
    let rail = park
        .rails()
        .iter()
        .enumerate()
        .filter(|(_, r)| r.near(point, COPING_NEAR))
        .filter_map(|(i, r)| {
            r.nearest_3d(point)
                .map(|(_, p, _)| (i, (p - point).length()))
        })
        .filter(|(_, d)| *d < COPING_NEAR)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i);
    Some(Edge {
        point,
        dist,
        dir,
        transition,
        rail,
    })
}

/// Where the drop-in is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Stepping up to the edge (Skate 3's `IntoDropIn/Into`).
    Into,
    /// Standing on the board at the coping while Y is held (`IntoDropIn/Cycle`).
    Stand,
    /// On the board, its tail held on the coping (`OutOfDropIn`, `Grinding/DroppingIn`).
    Out,
}

/// A drop-in under way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropIn {
    pub phase: Phase,
    /// Seconds in this phase, how long the step up to the edge takes, and how long the graphs have held
    /// the board on the coping.
    pub time: f32,
    pub into: f32,
    pub locked: f32,
    pub edge: Edge,
    /// Where the feet started, and where they stand at the edge.
    pub start: Vec3,
    pub stand: Vec3,
}

impl DropIn {
    pub fn start(pos: Vec3, edge: Edge) -> DropIn {
        let step = if edge.dist > BIG_STEP {
            0
        } else if edge.dist > SMALL_STEP {
            1
        } else {
            2
        };
        let stand = v3(edge.point.x, pos.y, edge.point.z) - edge.dir * STAND_BACK;
        DropIn {
            phase: Phase::Into,
            time: 0.0,
            into: INTO_TIMES[step],
            locked: 0.0,
            edge,
            start: pos,
            stand,
        }
    }

    /// The feet during the step up to the edge (eased in and out).
    pub fn feet(&self) -> Vec3 {
        let k = (self.time / self.into.max(1e-3)).clamp(0.0, 1.0);
        let k = k * k * (3.0 - 2.0 * k);
        self.start + (self.stand - self.start) * k
    }

    /// The board's tail is held on the coping now (`IsDroppingIn`).
    pub fn dropping(&self) -> bool {
        self.phase == Phase::Out
    }

    /// The board's middle while its tail is on the coping, and where it is let go.
    pub fn board(&self) -> Vec3 {
        self.edge.point - self.edge.dir * BOARD_BACK
    }
    pub fn release(&self) -> Vec3 {
        self.edge.point - self.edge.dir * RELEASE_BACK
    }
}

impl super::Skater {
    /// The edge a drop-in would start at now: standing still on foot on the ground with the board in
    /// hand (Skate 3's `LocoState` Stand and `IsHoldingSkateboard`), the edge within
    /// [`ENTER_DISTANCE`].
    pub(super) fn drop_in_edge(&self) -> Option<Edge> {
        if !self.walking()
            || self.biped.air.is_some()
            || !self.possession.held()
            || self.biped.loco != super::biped::Loco::Stand
        {
            return None;
        }
        self.edge.filter(|e| e.dist <= ENTER_DISTANCE)
    }

    /// One tick of the drop-in: up to the edge, standing on the board at it while Y is held (the stick
    /// pulled back steps away), then on the board with its tail on the coping, and let go into the
    /// transition. With the graphs their `IntoDropIn` clips time the step up to the edge.
    pub(super) fn step_drop_in(
        &mut self,
        park: &dyn Terrain,
        pad: &crate::input::Pad,
        dt: f32,
        events: &mut alloc::vec::Vec<super::Event>,
    ) {
        use super::Event;
        let Some(mut d) = self.drop_in else {
            return;
        };
        d.time += dt;
        let yaw = crate::math::atan2f(d.edge.dir.x, d.edge.dir.z);
        match d.phase {
            Phase::Into | Phase::Stand => {
                if d.phase == Phase::Into {
                    let stepped = self.graph.drop_in_stand || d.time > d.into + 1.0;
                    if stepped {
                        d.phase = Phase::Stand;
                        d.time = 0.0;
                        events.push(Event::DropInSet);
                    }
                } else {
                    // Skate 3's `TryToLeave`: the stick back steps away, Y let go drops in.
                    let world = super::biped::world_stick(pad.stick, self.cam_yaw);
                    let back = world[0] * d.edge.dir.x + world[1] * d.edge.dir.z;
                    if back <= BACK_OUT {
                        self.drop_in = None;
                        events.push(Event::DropInCancelled);
                        return;
                    }
                    if !pad.is_held(crate::input::btn::Y) {
                        d.phase = Phase::Out;
                        d.time = 0.0;
                        self.board_on_coping(&d);
                        self.drop_in = Some(d);
                        events.push(Event::SteppedOn);
                        return;
                    }
                }
                let feet = d.feet();
                let g = park.ground(feet.x, feet.z, feet.y + 0.3);
                self.pos = crate::math::v3(feet.x, g.height.max(feet.y - 0.3), feet.z);
                self.biped.heading = yaw;
                self.biped.vel = Vec3::ZERO;
                self.biped.speed = 0.0;
                self.sync_walk();
                self.drop_in = Some(d);
            }
            Phase::Out => {
                if self.graph.drop_in_grinding {
                    d.locked += dt;
                }
                let held = d.locked < LOCK_TIME && d.time < LOCK_LIMIT;
                if held {
                    self.board_on_coping(&d);
                    self.drop_in = Some(d);
                    return;
                }
                // Let go just behind the coping, rolling slowly over it.
                let at = d.release();
                self.pos = crate::math::v3(at.x, d.edge.point.y, at.z);
                self.fwd = d.edge.dir;
                self.normal = Vec3::UP;
                self.contact_n = Vec3::UP;
                self.speed = RELEASE_SPEED;
                self.slip = 0.0;
                self.vn = 0.0;
                self.vel = self.ground_velocity();
                self.states.facts.mount = Some(true);
                self.drop_in = None;
                self.sync_heading();
                events.push(Event::DroppedIn);
            }
        }
    }

    /// On the board, still, its tail on the coping (the deck under its back wheels).
    fn board_on_coping(&mut self, d: &DropIn) {
        let at = d.board();
        self.pos = crate::math::v3(at.x, d.edge.point.y, at.z);
        self.fwd = d.edge.dir;
        self.normal = Vec3::UP;
        self.contact_n = Vec3::UP;
        self.speed = 0.0;
        self.slip = 0.0;
        self.vn = 0.0;
        self.vel = Vec3::ZERO;
        self.fakie = false;
        self.fakie_timer = 0.0;
        self.spin = 0.0;
        self.spin_rate = 0.0;
        self.states.facts.mount = Some(true);
        self.possession.under_feet();
        self.sync_heading();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::{Axis, Park, Shape};
    use alloc::vec;

    /// A 1.5 m deck (z 0..4) with a ramp falling away from its edge at z = 4.
    fn deck() -> Park {
        Park {
            shapes: vec![
                Shape::Block {
                    min: v3(-5.0, 0.0, 0.0),
                    max: v3(5.0, 1.5, 4.0),
                    rgba: 0,
                },
                Shape::Ramp {
                    min: v3(-5.0, 0.0, 4.0),
                    max: v3(5.0, 0.0, 6.0),
                    axis: Axis::Z,
                    h0: 1.49,
                    h1: 0.0,
                    rgba: 0,
                },
            ],
            half_size: 20.0,
            wall_height: 2.0,
            spawn: v3(0.0, 1.5, 1.0),
            spawn_yaw: 0.0,
            rails: vec![crate::rails::Rail::new(vec![
                v3(-5.0, 1.5, 4.0),
                v3(5.0, 1.5, 4.0),
            ])],
        }
    }

    #[test]
    fn the_edge_ahead_is_found_with_its_transition_and_coping() {
        let p = deck();
        let e = find_edge(&p, v3(0.0, 1.5, 3.0), v3(0.0, 0.0, 1.0)).expect("edge");
        assert!((e.dist - 1.0).abs() < 0.06, "{e:?}");
        assert!(e.transition && e.rail == Some(0), "{e:?}");
        // Facing away from it: nothing within reach.
        assert_eq!(find_edge(&p, v3(0.0, 1.5, 3.0), v3(0.0, 0.0, -1.0)), None);
        // A plain drop has no transition.
        let mut plain = deck();
        plain.shapes.truncate(1);
        plain.rails.clear();
        let e = find_edge(&plain, v3(0.0, 1.5, 3.5), v3(0.0, 0.0, 1.0)).unwrap();
        assert!(!e.transition && e.rail.is_none(), "{e:?}");
    }

    #[test]
    fn the_step_up_to_the_edge_is_sized_by_the_distance() {
        let p = deck();
        let at = |z: f32| {
            let e = find_edge(&p, v3(0.0, 1.5, z), v3(0.0, 0.0, 1.0)).unwrap();
            DropIn::start(v3(0.0, 1.5, z), e)
        };
        assert_eq!(at(2.7).into, INTO_TIMES[0]);
        assert_eq!(at(3.3).into, INTO_TIMES[1]);
        assert_eq!(at(3.8).into, INTO_TIMES[2]);
        let mut d = at(2.7);
        d.time = d.into;
        assert!(
            (d.feet().z - (4.0 - STAND_BACK)).abs() < 0.06,
            "{:?}",
            d.feet()
        );
    }
}
