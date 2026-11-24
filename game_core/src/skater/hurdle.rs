//! Skate 3's hippie hurdle (`IsHippyHurdling`), as SK8-ENGINE/skate-3-rust-engine reads its landing on
//! deck (GPL-3.0; crates/skate-core/src/player/offboard/landing_deck.rs `fill`, landing_deck/query.rs
//! `prepare_query` and `sync`, landing_deck/update.rs `update`; TU3 `0x82D79948`, `0x82D79B40`,
//! `0x82D794A0`, `0x82D79420`).
//!
//! In a hippie jump the landing on deck sweeps the rider's flight down to where the board will be (its
//! deck at landing, 0.2 m up) for something in the way. A hit more than 0.1 m under that deck is the
//! ground and nothing is in the way. Anything higher is an obstacle: the first hit notes the touched
//! body's place, the next compares it. A body moving faster than 0.7 m/s is a hurdle, the rider
//! jumping a moving obstacle while the board rolls on (the motion graph's `OnBoard/HippyJump/HurdleCyc`
//! in place of `Cyc`, which holds the landing on the board off until it is over); a still one blocks the
//! landing. From the third query the hurdle is checked against where the obstacle will be when he lands:
//! coming down less than 0.7 m over it, or still going up with it ending within 1 m of where he lands,
//! blocks; ending over 2.5 m away clears. The hurdle is over 0.3 s past the apex, or past it with under
//! 0.2 s to land.
//!
//! The touched body's place is that of what was hit: the static world's frame never moves, so only a
//! moving object can be hurdled. Flatground's parks and districts are static (no cars or other moving
//! objects), so its hippie jumps never hurdle; the queries' hits there could only block, which the
//! landing doesn't read here (flatground's hippie jump lands on the board where the physics comes down).

use crate::math::Vec3;

/// The board's deck is taken this far above the board's place at landing (SK8 `0x3e4ccccd`).
pub const DECK_ABOVE: f32 = 0.2;
/// A hit this far under the deck is the ground under it, not an obstacle.
pub const UNDER_DECK: f32 = 0.1;
/// The touched body moving faster than this (squared speed, (m/s)^2; SK8 `0x3efae147`: 0.7 m/s) is a
/// hurdle.
pub const MOVING_SQ: f32 = 0.49;
/// The queries run once a tick (the touched body's speed is its move times 60).
pub const RATE: f32 = 60.0;
/// Coming down this little over the obstacle (m) blocks the landing.
pub const CLEARANCE: f32 = 0.7;
/// Going up, the obstacle ending within this of the landing (squared, m^2) blocks it, beyond the other
/// clears it.
pub const NEAR_SQ: f32 = 1.0;
pub const CLEAR_SQ: f32 = 6.25;
/// The hurdle is over this long past the apex (s), or past it with under `LATE_LAND` (s) to land.
pub const PAST_APEX: f32 = 0.3;
pub const LATE_LAND: f32 = 0.2;

/// A completed query's hit: where it touched, and the place of the touched body's frame.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub point: Vec3,
    pub body: Vec3,
}

/// The rider's flight as a query's result reads it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flight {
    /// Seconds left to land.
    pub time_to_land: f32,
    /// Where the flight lands.
    pub landing: Vec3,
    /// Coming down (the flight's vertical speed under 0).
    pub descending: bool,
    /// The rider's height.
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hurdle {
    body: Vec3,
    queries: u32,
    obstruction: f32,
    blocked: bool,
    tested: bool,
    hurdling: bool,
}

impl Hurdle {
    /// A new flight (SK8 `reset` and the air assist's `completed = 0, tested = false`).
    pub fn take_off(&mut self) {
        *self = Hurdle::default();
    }

    /// A query is due: none has settled the flight yet.
    pub fn wants_query(&self) -> bool {
        !self.tested
    }

    /// Before a query (SK8 `prepare_query`): the board's place at landing, whose deck an obstacle has
    /// to be above.
    pub fn prepare(&mut self, board_at_landing: Vec3) {
        self.blocked = false;
        self.obstruction = board_at_landing.y + DECK_ABOVE - UNDER_DECK;
    }

    /// A query's result (SK8 `sync`; `None`: nothing in the way).
    pub fn sync(&mut self, hit: Option<Hit>, f: &Flight) {
        if let Some(hit) = hit {
            if hit.point.y < self.obstruction {
                self.blocked = false;
                self.tested = true;
            } else if self.tested {
                self.blocked = true;
            } else if self.queries == 0 {
                self.body = hit.body;
            } else {
                let v = (hit.body - self.body) * RATE;
                if v.dot(v) > MOVING_SQ {
                    self.hurdling = true;
                    // On the second query SK8 also raises the flight to clear the obstacle when an
                    // actor flag (2488 bit 26) asks for it; flatground has no such flag.
                    if self.queries != 1 {
                        let ahead = hit.body + v * f.time_to_land;
                        let mut gap = f.landing - ahead;
                        gap.y = 0.0;
                        let sq = gap.dot(gap);
                        if f.descending {
                            if f.height - hit.point.y < CLEARANCE {
                                self.blocked = true;
                                self.tested = true;
                            }
                        } else if sq < NEAR_SQ {
                            self.blocked = true;
                            self.tested = true;
                        } else if sq > CLEAR_SQ {
                            self.blocked = false;
                            self.tested = true;
                        }
                    }
                } else {
                    self.hurdling = false;
                    self.blocked = true;
                    self.tested = true;
                }
                self.body = hit.body;
            }
        } else {
            self.blocked = false;
            self.tested = true;
        }
        self.queries = self.queries.wrapping_add(1);
    }

    /// Each tick of the flight (SK8 `update`): `past_apex` seconds past the apex (negative before it).
    pub fn update(&mut self, past_apex: f32, time_to_land: f32) {
        if self.hurdling && (past_apex > PAST_APEX || (past_apex > 0.0 && time_to_land < LATE_LAND))
        {
            self.hurdling = false;
        }
    }

    /// `IsHippyHurdling` (SK8 `fill`: OffBoard317).
    pub fn hurdling(&self) -> bool {
        self.hurdling && !self.blocked
    }

    /// Something is in the way of the landing on the board.
    pub fn blocked(&self) -> bool {
        self.blocked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;

    fn flight() -> Flight {
        Flight {
            time_to_land: 0.5,
            landing: v3(0.0, 0.0, 4.0),
            descending: false,
            height: 1.2,
        }
    }

    fn hit(body: Vec3) -> Option<Hit> {
        Some(Hit {
            point: v3(0.0, 0.8, 2.0),
            body,
        })
    }

    #[test]
    fn nothing_in_the_way_settles_the_flight() {
        let mut h = Hurdle::default();
        h.take_off();
        h.prepare(Vec3::ZERO);
        assert!(h.wants_query());
        h.sync(None, &flight());
        assert!(!h.wants_query() && !h.hurdling() && !h.blocked());
    }

    #[test]
    fn a_hit_on_the_ground_under_the_deck_is_not_an_obstacle() {
        let mut h = Hurdle::default();
        h.prepare(Vec3::ZERO);
        h.sync(
            Some(Hit {
                point: v3(0.0, 0.05, 3.0),
                body: Vec3::ZERO,
            }),
            &flight(),
        );
        assert!(!h.wants_query() && !h.blocked());
    }

    #[test]
    fn a_still_obstacle_blocks_and_never_hurdles() {
        let mut h = Hurdle::default();
        for _ in 0..2 {
            h.prepare(Vec3::ZERO);
            h.sync(hit(Vec3::ZERO), &flight());
        }
        assert!(h.blocked() && !h.hurdling() && !h.wants_query());
    }

    #[test]
    fn a_moving_obstacle_is_a_hurdle_until_past_the_apex() {
        let mut h = Hurdle::default();
        // Crossing the path at 3 m/s (0.05 m a tick), passing well clear of the landing.
        let mut body = v3(-1.0, 0.0, 2.0);
        for _ in 0..2 {
            h.prepare(Vec3::ZERO);
            h.sync(hit(body), &flight());
            body.x += 0.05;
        }
        assert!(h.hurdling(), "moving: a hurdle");
        assert!(h.wants_query(), "the second query doesn't settle it");
        // Third: where it will be at landing (x 2.5 m on, the landing at x 0): over 2.5 m away clears.
        let mut f = flight();
        f.time_to_land = 1.0;
        h.prepare(Vec3::ZERO);
        h.sync(hit(body), &f);
        assert!(h.hurdling() && !h.wants_query());
        h.update(-0.1, 0.8);
        assert!(h.hurdling(), "before the apex");
        h.update(0.1, 0.5);
        assert!(h.hurdling());
        h.update(0.1, 0.15);
        assert!(!h.hurdling(), "past the apex and about to land");
    }

    #[test]
    fn a_moving_obstacle_ending_under_the_landing_blocks() {
        let mut h = Hurdle::default();
        let mut body = v3(0.0, 0.0, 3.0);
        for _ in 0..3 {
            h.prepare(Vec3::ZERO);
            // Coming towards the landing along z at 3 m/s: there when he lands.
            h.sync(hit(body), &flight());
            body.z += 0.05;
        }
        assert!(h.blocked() && !h.hurdling());
        // Coming down less than 0.7 m over it blocks too.
        let mut h = Hurdle::default();
        let mut body = v3(-1.0, 0.0, 2.0);
        let mut f = flight();
        f.descending = true;
        f.height = 1.3;
        for _ in 0..3 {
            h.prepare(Vec3::ZERO);
            h.sync(hit(body), &f);
            body.x += 0.05;
        }
        assert!(h.blocked() && !h.hurdling());
    }

    #[test]
    fn the_hurdle_ends_0_3_s_past_the_apex() {
        let mut h = Hurdle::default();
        let mut body = v3(-1.0, 0.0, 2.0);
        for _ in 0..2 {
            h.prepare(Vec3::ZERO);
            h.sync(hit(body), &flight());
            body.x += 0.05;
        }
        h.update(0.29, 1.0);
        assert!(h.hurdling());
        h.update(0.31, 1.0);
        assert!(!h.hurdling());
    }
}
