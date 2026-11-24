//! Hall of Meat: what a bail did to the rider. Skate 3 scores its Hall of Meat from the injuries of a
//! wipeout, with broken bones (its camera even slows down for one, `IsBrokenBoneSlowMo`), but neither
//! the disc's tuning database nor SK8-ENGINE/skate-3-rust-engine (GPL-3.0) has its injury model: SK8
//! keeps only the audio and camera hooks, and its `broken-bones` example mod breaks a limb on an impact
//! of at least 80 N s at a closing speed of at least 3 m/s. flatground's model is its own, on those
//! two numbers: every impact of a ragdoll point (`skater::ragdoll`) faster than [`MIN_SPEED`] hurts the
//! body part the point belongs to by its momentum over that speed, and breaks the part's bone when the
//! point's impulse reaches [`BREAK_IMPULSE`] (or the part's damage adds up to [`BREAK_DAMAGE`]). The
//! score is the damage in dollars ([`DOLLARS`] a newton-second) and [`BONE_BONUS`] a broken bone.

use super::ragdoll::{BODY_MASS, Contacts, MASS, PARTS, POINT_PART, POINTS};

/// The bones each body part breaks (the rider's own left and right).
pub const NAMES: [&str; PARTS] = [
    "PELVIS",
    "RIBS",
    "SKULL",
    "L HUMERUS",
    "L FOREARM",
    "R HUMERUS",
    "R FOREARM",
    "L FEMUR",
    "L TIBIA",
    "R FEMUR",
    "R TIBIA",
];

/// SK8's broken-bones mod: no injury under this closing speed (m/s), a break from this impulse (N s).
pub const MIN_SPEED: f32 = 3.0;
pub const BREAK_IMPULSE: f32 = 80.0;
/// Damage (N s over the least speed, added up) that breaks a part's bone however it came (ours).
pub const BREAK_DAMAGE: f32 = 120.0;
/// The score: dollars a newton-second of damage and a broken bone (ours).
pub const DOLLARS: f32 = 10.0;
pub const BONE_BONUS: u32 = 1000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Meat {
    /// Each part's damage (N s over the least speed), and whether its bone is broken.
    pub damage: [f32; PARTS],
    pub broken: [bool; PARTS],
    /// Impacts that hurt, and the fastest (m/s).
    pub hits: u32,
    pub worst: f32,
}

impl Meat {
    /// A tick's impacts. Returns how many bones broke in it.
    pub fn take(&mut self, c: &Contacts) -> u32 {
        let mut new = 0;
        for i in 0..POINTS {
            let v = c.impact[i];
            if !c.touching[i] || v <= MIN_SPEED {
                continue;
            }
            let part = POINT_PART[i];
            let m = BODY_MASS * MASS[i];
            self.damage[part] += m * (v - MIN_SPEED);
            self.hits += 1;
            self.worst = self.worst.max(v);
            if !self.broken[part] && (m * v >= BREAK_IMPULSE || self.damage[part] >= BREAK_DAMAGE) {
                self.broken[part] = true;
                new += 1;
            }
        }
        new
    }

    /// The bail's Hall of Meat score (dollars).
    pub fn score(&self) -> u32 {
        let damage: f32 = self.damage.iter().sum();
        libm::roundf(damage * DOLLARS) as u32 + self.bones() * BONE_BONUS
    }

    pub fn bones(&self) -> u32 {
        self.broken.iter().filter(|b| **b).count() as u32
    }

    /// The broken bones' names, worst damage first.
    pub fn broken_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        let mut order = [0usize; PARTS];
        for (i, o) in order.iter_mut().enumerate() {
            *o = i;
        }
        // Insertion sort by damage, highest first (no allocation).
        for i in 1..PARTS {
            let mut j = i;
            while j > 0 && self.damage[order[j]] > self.damage[order[j - 1]] {
                order.swap(j, j - 1);
                j -= 1;
            }
        }
        order
            .into_iter()
            .filter(|&p| self.broken[p])
            .map(|p| NAMES[p])
    }
}

#[cfg(test)]
mod tests {
    use super::super::ragdoll::{HEAD, L_FOOT, PELVIS};
    use super::*;

    fn hit(points: &[(usize, f32)]) -> Contacts {
        let mut c = Contacts::default();
        for &(i, v) in points {
            c.touching[i] = true;
            c.impact[i] = v;
        }
        c
    }

    #[test]
    fn slow_impacts_dont_hurt_and_hard_ones_break_bones() {
        let mut m = Meat::default();
        assert_eq!(m.take(&hit(&[(PELVIS, 2.9), (HEAD, 1.0)])), 0);
        assert_eq!((m.score(), m.hits), (0, 0));
        // The head (6 kg) at 8 m/s: 48 N s, hurt, not broken.
        assert_eq!(m.take(&hit(&[(HEAD, 8.0)])), 0);
        assert!(m.damage[2] > 29.0 && !m.broken[2]);
        let before = m.score();
        assert!(before > 0);
        // The pelvis (10.5 kg) at 8 m/s: 84 N s, broken.
        assert_eq!(m.take(&hit(&[(PELVIS, 8.0), (L_FOOT, 4.0)])), 1);
        assert!(m.broken[0]);
        assert!(m.score() >= before + BONE_BONUS);
        assert_eq!(m.broken_names().collect::<alloc::vec::Vec<_>>(), ["PELVIS"]);
        // Repeated hits add up to a break.
        for _ in 0..10 {
            m.take(&hit(&[(HEAD, 8.0)]));
        }
        assert!(m.broken[2]);
        assert_eq!(m.bones(), 2);
    }
}
