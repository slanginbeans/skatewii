//! After a bail: when the rider is put back, and where (SK8-ENGINE/skate-3-rust-engine, GPL-3.0:
//! `player/wipeout_state/recovery.rs`, `player/respawn.rs`, `physics/respawn.rs`).
//!
//! - [`Recovery`]: an automatic reset once the body has settled for a while (or after a long time), with
//!   a short fade first; the player may reset early (A or B, Skate 3's push buttons) once the body has
//!   settled or a moment has passed.
//! - [`Checkpoints`]: recent safe spots the board rode over (on the ground, facing up enough, clear
//!   above, away from rails), with the heading of travel and the stance. A reset puts the rider where
//!   the body came to rest when that spot is safe, else on the best recent checkpoint, else the park's
//!   spawn.

use alloc::vec::Vec;

use crate::math::{Vec3, atan2f, v3};
use crate::park::{STAIRS, Terrain, surface_category};
use crate::tuning::Tuning;

/// Most checkpoints kept.
const KEPT: usize = 32;
/// A checkpoint is recorded no sooner than this many ticks after the rider was put on the board, after
/// this many ticks of riding, at least `COOLDOWN` ticks after the last one and `SPACING` m from every
/// other.
const WARM_UP: u32 = 120;
const MIN_RIDING: u32 = 15;
const COOLDOWN: u32 = 20;
const SPACING: f32 = 1.5;
/// Selection: the newest is penalised (it is where the trouble started), and so are old ones.
const NEWEST_PENALTY: f32 = -100.0;
const OLD_PENALTY: f32 = -200.0;
const OLD_AFTER: usize = 5;
/// The player may reset after this long even if the body hasn't settled (s).
const MANUAL_AFTER: f32 = 1.6;
/// Ticks between asking for a reset and getting it.
const RESET_DELAY: u32 = 2;
/// Rails within this of a spot (m across, m up or down) make it unsafe.
const RAIL_CLEAR: f32 = 0.3;
const RAIL_CLEAR_Y: f32 = 0.6;
/// How far below a spot the ground is looked for (m).
const GROUND_PROBE: f32 = 10.0;

/// A safe spot: where, the heading of travel (yaw), and whether the rider was riding switch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Checkpoint {
    pub pos: Vec3,
    pub heading: f32,
    pub switch: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Checkpoints {
    entries: Vec<Checkpoint>,
    cooldown: u32,
    /// The heading of travel while last riding (where a reset on the spot faces).
    pub heading: Option<f32>,
}

impl Checkpoints {
    /// One tick of riding (`riding_ticks` in a row, `since_spawn` ticks since the rider was put on).
    pub fn observe(
        &mut self,
        t: &Tuning,
        park: &dyn Terrain,
        at: Checkpoint,
        riding_ticks: u32,
        since_spawn: u32,
    ) {
        self.heading = Some(at.heading);
        if since_spawn < WARM_UP {
            self.cooldown = 0;
            return;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return;
        }
        if riding_ticks <= MIN_RIDING
            || self
                .entries
                .iter()
                .any(|e| (e.pos - at.pos).length() < SPACING)
            || !safe(t, park, at.pos)
        {
            return;
        }
        if self.entries.len() == KEPT {
            self.entries.remove(0);
        }
        self.entries.push(at);
        self.cooldown = COOLDOWN;
    }

    /// Where to put the rider back after coming to rest at `rest`: there if it is safe, else the best
    /// checkpoint (taken out, so repeated bails don't keep picking it), else `None` (the spawn).
    pub fn pick(
        &mut self,
        t: &Tuning,
        park: &dyn Terrain,
        rest: Vec3,
        switch: bool,
    ) -> Option<Checkpoint> {
        if let Some(heading) = self.heading
            && let Some(y) = ground_at(t, park, rest)
            && clear(t, park, v3(rest.x, y, rest.z))
        {
            return Some(Checkpoint {
                pos: v3(rest.x, y, rest.z),
                heading,
                switch,
            });
        }
        let mut best: Option<(usize, f32)> = None;
        for (age, i) in (0..self.entries.len()).rev().enumerate() {
            let mut score = 0.0;
            if age < 1 {
                score += NEWEST_PENALTY;
            }
            if age > OLD_AFTER {
                score += OLD_PENALTY;
            }
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((i, score));
            }
        }
        best.map(|(i, _)| self.entries.remove(i))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The ground under `p` when it is a safe surface to be put on: its height.
fn ground_at(t: &Tuning, park: &dyn Terrain, p: Vec3) -> Option<f32> {
    let start = p + Vec3::UP * 0.1;
    let h = park.raycast(start, Vec3::UP * -1.0, GROUND_PROBE)?;
    let category = surface_category(h.surface);
    let allowed = !matches!(category, 5 | 6 | 9 | 12 | 13 | STAIRS);
    (h.normal.y >= t.respawn_normal_y && start.y - h.point.y <= t.respawn_drop && allowed)
        .then_some(h.point.y)
}

/// Room for the rider over `p` and no rail at it.
fn clear(t: &Tuning, park: &dyn Terrain, p: Vec3) -> bool {
    let bottom = p + Vec3::UP * t.respawn_radius;
    let up = t.respawn_height + t.respawn_radius;
    if park.raycast(bottom, Vec3::UP, up).is_some() {
        return false;
    }
    !park.rails().iter().any(|r| {
        r.nearest(p)
            .is_some_and(|n| n.dist_h < RAIL_CLEAR && (n.point.y - p.y).abs() < RAIL_CLEAR_Y)
    })
}

fn safe(t: &Tuning, park: &dyn Terrain, p: Vec3) -> bool {
    ground_at(t, park, p).is_some_and(|y| clear(t, park, v3(p.x, y, p.z)))
}

/// The heading of travel for a checkpoint: along the velocity when moving, else the board's.
pub fn travel_heading(vel: Vec3, board_yaw: f32) -> f32 {
    let h = vel.horizontal();
    if vel.length() > 0.5 && h.length() > 0.3 {
        atan2f(h.x, h.z)
    } else {
        board_yaw
    }
}

/// The wipeout's recovery clock (SK8 `manage_recovery`, `post_physics`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Recovery {
    pub settled_time: f32,
    pub ever_settled: bool,
    /// Fading out before an automatic reset: seconds left.
    pub fade: Option<f32>,
    /// Ticks left before a reset the player asked for.
    pub countdown: Option<u32>,
}

impl Recovery {
    /// One tick, `time` s into the wipeout with the body moving at `speed` and a reset button just
    /// pressed or not. True when it is time to put the rider back.
    pub fn update(
        &mut self,
        t: &Tuning,
        time: f32,
        speed: f32,
        reset_pressed: bool,
        dt: f32,
    ) -> bool {
        let settled = speed < t.settled_speed && time > t.settled_min_time;
        self.ever_settled |= settled;
        self.settled_time = if settled { self.settled_time + dt } else { 0.0 };
        if let Some(n) = &mut self.countdown {
            if *n == 0 {
                return true;
            }
            *n -= 1;
            return false;
        }
        let eligible = self.ever_settled || time > MANUAL_AFTER;
        if eligible && reset_pressed {
            self.countdown = Some(RESET_DELAY);
            return false;
        }
        if let Some(f) = &mut self.fade {
            *f -= dt;
            return *f <= 0.0;
        }
        let automatic = time > t.reset_max_time
            || (time > t.reset_min_time && self.settled_time > t.reset_settled);
        if automatic {
            self.fade = Some(t.reset_fade);
        }
        false
    }

    /// How dark the screen is (0..1) while fading out for a reset.
    pub fn darkness(&self, t: &Tuning) -> f32 {
        match self.fade {
            Some(f) if t.reset_fade > 0.0 => (1.0 - f / t.reset_fade).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settled_body_resets_after_a_while_and_the_player_can_skip_it() {
        let t = Tuning::BUILT_IN;
        let dt = 1.0 / 60.0;
        let run = |press_at: Option<f32>| {
            let mut r = Recovery::default();
            let mut time = 0.0;
            for _ in 0..(60 * 30) {
                time += dt;
                let press = press_at.is_some_and(|p| (time - p).abs() < dt * 0.5);
                if r.update(&t, time, 0.1, press, dt) {
                    return time;
                }
            }
            f32::INFINITY
        };
        let auto = run(None);
        assert!(
            auto > t.reset_min_time && auto < t.reset_min_time + t.reset_settled + 1.0,
            "{auto}"
        );
        let early = run(Some(1.0));
        assert!(early < 1.2, "{early}");
        // Too early (not settled, under 1.6 s) a press does nothing.
        let mut r = Recovery::default();
        assert!(!r.update(&t, 0.1, 5.0, true, dt));
        assert!(r.countdown.is_none());
    }
}
