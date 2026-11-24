//! Skate 3's point-backed countdowns (SK8 `skate-core/src/scoring/timer.rs`, TU3 82DA4C28): the line
//! timer and the combo timer behind the multiplier hold points and lose them at a drain rate.

/// A countdown held as points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointTimer {
    pub points: f32,
    /// Ran out this tick (an edge, not a state).
    pub expired: bool,
}

impl PointTimer {
    /// Drains `drain * scale` points a second. Holding (`hold_near_one`), it stops just above one point
    /// instead of running out; returns whether it held.
    pub fn advance(&mut self, dt: f32, drain: f32, scale: f32, hold_near_one: bool) -> bool {
        self.expired = false;
        if self.points > 0.0 {
            let previous = self.points;
            self.points -= drain * dt * scale;
            if hold_near_one && self.points < 1.001 && previous > 1.000_001 {
                self.points = previous;
                return true;
            }
            if self.points <= 0.0 {
                self.points = 0.0;
                self.expired = true;
            }
        }
        false
    }

    pub fn credit(&mut self, points: f32, capacity: f32) {
        self.points = (self.points + points).min(capacity);
    }

    /// Seconds left at the idle drain.
    pub fn seconds(&self, drain: f32) -> f32 {
        if drain > 0.0 {
            self.points / drain
        } else {
            0.0
        }
    }
}

/// The combo timer and the multiplier it sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComboTimer {
    pub timer: PointTimer,
    pub multiplier: f32,
}

impl Default for ComboTimer {
    fn default() -> Self {
        ComboTimer {
            timer: PointTimer::default(),
            multiplier: 1.0,
        }
    }
}

impl ComboTimer {
    /// Credits a reward: the multiplier goes up to the level the timer's points reach, or (a reward
    /// over `refresh`) to that level whatever it was. The timer draining alone never lowers it.
    pub fn credit(&mut self, reward: f32, capacity: f32, levels: [(f32, f32); 3], refresh: f32) {
        self.timer.credit(reward, capacity);
        let mut next = 1.0;
        for (threshold, multiplier) in levels {
            if self.timer.points >= threshold {
                next = multiplier;
            }
        }
        if next > self.multiplier || reward > refresh {
            self.multiplier = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVELS: [(f32, f32); 3] = [(50.0, 1.5), (450.0, 2.0), (800.0, 3.0)];

    #[test]
    fn a_held_timer_stops_above_one_point_and_running_out_is_an_edge() {
        let mut t = PointTimer {
            points: 2.0,
            expired: false,
        };
        assert!(t.advance(0.1, 50.0, 1.0, true));
        assert_eq!(t.points, 2.0);
        assert!(!t.advance(0.1, 50.0, 1.0, false));
        assert!(t.expired && t.points == 0.0);
        t.advance(0.1, 50.0, 1.0, false);
        assert!(!t.expired, "only the tick it ran out");
        let mut t = PointTimer::default();
        t.credit(500.0, 400.0);
        assert_eq!((t.points, t.seconds(50.0)), (400.0, 8.0), "capped");
    }

    #[test]
    fn the_multiplier_climbs_with_the_combo_points_and_outlives_their_drain() {
        let mut c = ComboTimer::default();
        c.credit(10.0, 801.0, LEVELS, 799.0);
        assert_eq!(c.multiplier, 1.0);
        c.credit(40.0, 801.0, LEVELS, 799.0);
        assert_eq!(c.multiplier, 1.5);
        c.credit(400.0, 801.0, LEVELS, 799.0);
        assert_eq!(c.multiplier, 2.0);
        c.credit(350.0, 801.0, LEVELS, 799.0);
        assert_eq!(c.multiplier, 3.0);
        c.timer.advance(8.0, 100.0, 1.0, false);
        c.credit(1.0, 801.0, LEVELS, 799.0);
        assert_eq!(c.multiplier, 3.0, "the drain does not lower it");
        c.timer.advance(8.0, 100.0, 1.0, false);
        c.credit(500.0, 801.0, LEVELS, 400.0);
        assert_eq!(c.multiplier, 2.0, "a big reward refreshes it to its level");
    }
}
