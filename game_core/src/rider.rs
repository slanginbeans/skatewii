//! Drives an imported [`Character`] from the skater's state: picks a clip for what the skater is doing,
//! crossfades between clips, poses and skins it once per step.
//!
//! Clip roles are the `skater/clip/<role>` entries `fg_import skater` writes: `idle`, `crouch`, `push`,
//! and `<trick>/pop` + `<trick>/air` for each flick trick. A missing clip falls back to `idle`.

use alloc::vec;
use alloc::vec::Vec;

use crate::character::{Character, Local};
use crate::flick::Flick;
use crate::gfx::SUN;
use crate::math::{Mat3, sqrtf, v3};
use crate::skater::{Mode, Skater};

/// Crossfade time between clips (s).
const FADE: f32 = 0.12;
/// Crouch amount above which the crouch clip plays.
const CROUCH_MIN: f32 = 0.05;
/// Playback speed limits when fitting a trick into the flight.
const MIN_SPEED: f32 = 0.6;
const MAX_SPEED: f32 = 2.5;

/// Which clip to show and how far into it (seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub role: &'static str,
    pub time: f32,
}

/// The clip roles of a trick, played back to back: pop, air, flip, catch (any may be missing).
pub fn trick_roles(f: Flick) -> [&'static str; 4] {
    match f {
        Flick::Ollie => ["ollie/pop", "ollie/air", "ollie/flip", "ollie/out"],
        Flick::Nollie => ["nollie/pop", "nollie/air", "nollie/flip", "nollie/out"],
        Flick::Kickflip => [
            "kickflip/pop",
            "kickflip/air",
            "kickflip/flip",
            "kickflip/out",
        ],
        Flick::Heelflip => [
            "heelflip/pop",
            "heelflip/air",
            "heelflip/flip",
            "heelflip/out",
        ],
        Flick::PopShoveIt => [
            "popshuvit/pop",
            "popshuvit/air",
            "popshuvit/flip",
            "popshuvit/out",
        ],
        Flick::FsShoveIt => [
            "fsshuvit/pop",
            "fsshuvit/air",
            "fsshuvit/flip",
            "fsshuvit/out",
        ],
    }
}

/// Seconds until a body at `height` above the ground, moving up at `vel_y`, lands under `gravity`.
pub fn time_to_land(height: f32, vel_y: f32, gravity: f32) -> f32 {
    (vel_y + sqrtf((vel_y * vel_y + 2.0 * gravity * height.max(0.0)).max(0.0))) / gravity
}

/// Picks the clip for the skater's current state. `duration` gives a clip's length in seconds, 0 when
/// the pack does not have it. `land_in` is the predicted time left in the air: the trick stages after
/// the pop are sped up or slowed down (within limits) so the catch ends as the board lands.
pub fn pick(s: &Skater, clock: f32, land_in: f32, duration: &dyn Fn(&str) -> f32) -> Pick {
    let idle = Pick {
        role: "idle",
        time: clock,
    };
    let has = |r: &str| duration(r) > 0.0;
    match s.mode {
        Mode::Bailed { .. } => idle,
        Mode::Air { time } => {
            let Some(t) = &s.trick else {
                return idle;
            };
            let mut stages = trick_roles(t.flick);
            if !stages.iter().any(|r| has(r)) {
                stages = trick_roles(Flick::Ollie);
            }
            let [pop, rest @ ..] = stages;
            let pop_d = duration(pop);
            if time < pop_d {
                return Pick { role: pop, time };
            }
            // Fit the rest of the trick into the flight, within sensible speeds.
            let rest_d: f32 = rest.iter().map(|r| duration(r)).sum();
            let window = time + land_in - pop_d;
            let speed = if window.is_finite() && window > 0.05 && rest_d > 0.0 {
                (rest_d / window).clamp(MIN_SPEED, MAX_SPEED)
            } else {
                1.0
            };
            let mut t = (time - pop_d) * speed;
            // Play the stages in order; the last one present holds its final frame until landing.
            let mut last = if has(pop) {
                Some(Pick {
                    role: pop,
                    time: pop_d,
                })
            } else {
                None
            };
            for role in rest.iter().copied().filter(|r| has(r)) {
                let d = duration(role);
                last = Some(Pick { role, time: t });
                if t < d {
                    break;
                }
                t -= d;
            }
            last.unwrap_or(idle)
        }
        Mode::Rolling => {
            if s.crouch > CROUCH_MIN && has("crouch") {
                Pick {
                    role: "crouch",
                    time: s.crouch.min(1.0) * duration("crouch"),
                }
            } else if s.since_push < duration("push") {
                Pick {
                    role: "push",
                    time: s.since_push,
                }
            } else {
                idle
            }
        }
    }
}

pub struct Rider {
    pub character: Character,
    cur: Pick,
    /// The clip faded out from, frozen where it was left.
    prev: Option<Pick>,
    fade: f32,
    a: Vec<Local>,
    b: Vec<Local>,
}

impl Rider {
    pub fn new(character: Character) -> Rider {
        let n = character.bones();
        let mut r = Rider {
            character,
            cur: Pick {
                role: "idle",
                time: 0.0,
            },
            prev: None,
            fade: FADE,
            a: vec![Local::IDENTITY; n],
            b: vec![Local::IDENTITY; n],
        };
        r.pose_and_skin(Mat3::IDENTITY);
        r
    }

    pub fn current(&self) -> Pick {
        self.cur
    }

    /// One step: choose the clip, blend, pose and skin. `rot` is the character's world rotation (for
    /// lighting).
    pub fn update(&mut self, s: &Skater, clock: f32, land_in: f32, dt: f32, rot: Mat3) {
        let ch = &self.character;
        let mut next = pick(s, clock, land_in, &|r| ch.duration(r));
        if next.role == "push" && self.cur.role == "push" {
            // Another push while the last one plays: keep the cycle going instead of restarting it.
            next.time = (self.cur.time + dt) % ch.duration("push");
        }
        if next.role != self.cur.role {
            self.prev = Some(self.cur);
            self.fade = 0.0;
        }
        self.cur = next;
        self.fade = (self.fade + dt).min(FADE);
        self.pose_and_skin(rot);
    }

    fn pose_and_skin(&mut self, rot: Mat3) {
        let ch = &mut self.character;
        if !ch.sample(self.cur.role, self.cur.time, &mut self.a) {
            ch.sample("idle", self.cur.time, &mut self.a);
        }
        if let Some(p) = self.prev.filter(|_| self.fade < FADE)
            && ch.sample(p.role, p.time, &mut self.b)
        {
            let k = self.fade / FADE;
            for (a, b) in self.a.iter_mut().zip(&self.b) {
                *a = b.blend(a, k);
            }
        }
        ch.pose(&self.a);
        // The sun in the character's own space: R^T * sun.
        let c = &rot.cols;
        ch.skin(v3(c[0].dot(SUN), c[1].dot(SUN), c[2].dot(SUN)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::Park;
    use crate::skater::{ActiveTrick, trick_def};

    fn dur(r: &str) -> f32 {
        match r {
            "idle" => 2.0,
            "crouch" => 0.5,
            "push" => 0.7,
            "ollie/pop" => 0.2,
            "ollie/air" => 0.5,
            "kickflip/pop" => 0.2,
            "kickflip/air" => 0.2,
            "kickflip/out" => 0.3,
            _ => 0.0,
        }
    }

    #[test]
    fn picks_clips_from_the_skater_state() {
        let park = Park::demo();
        let mut s = Skater::spawn(&park);
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "idle");
        s.crouch = 0.5;
        assert_eq!(
            pick(&s, 3.0, f32::INFINITY, &dur),
            Pick {
                role: "crouch",
                time: 0.25
            }
        );
        s.crouch = 0.0;
        s.since_push = 0.3;
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "push");
        s.since_push = 0.8;
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "idle");

        s.mode = Mode::Air { time: 0.1 };
        s.trick = Some(ActiveTrick {
            flick: Flick::Kickflip,
            def: trick_def(Flick::Kickflip),
            t: 0.1,
        });
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "kickflip/pop");
        s.mode = Mode::Air { time: 0.3 };
        let p = pick(&s, 3.0, f32::INFINITY, &dur);
        assert_eq!(p.role, "kickflip/air");
        assert!((p.time - 0.1).abs() < 1e-6);
        s.mode = Mode::Air { time: 0.5 };
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "kickflip/out");
        // Past the end: the catch holds.
        s.mode = Mode::Air { time: 2.0 };
        let p = pick(&s, 3.0, f32::INFINITY, &dur);
        assert_eq!(p.role, "kickflip/out");
        assert!((p.time - 1.6).abs() < 1e-5);
        // No heelflip clips in this pack: the ollie stands in.
        s.trick = Some(ActiveTrick {
            flick: Flick::Heelflip,
            def: trick_def(Flick::Heelflip),
            t: 0.3,
        });
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "ollie/air");
        s.mode = Mode::Bailed { time: 0.2 };
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "idle");
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;
    use crate::park::Park;
    use crate::skater::{ActiveTrick, trick_def};

    #[test]
    fn landing_time_is_ballistic() {
        // Popped at 4 m/s from the ground under g = 10: up and down again in 0.8 s.
        assert!((time_to_land(0.0, 4.0, 10.0) - 0.8).abs() < 1e-5);
        // Falling from 5 m at rest: 1 s.
        assert!((time_to_land(5.0, 0.0, 10.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn trick_is_fitted_into_the_flight() {
        let dur = |r: &str| match r {
            "kickflip/pop" => 0.2,
            "kickflip/air" => 0.4,
            "kickflip/out" => 0.4,
            _ => 0.0,
        };
        let mut s = Skater::spawn(&Park::demo());
        s.trick = Some(ActiveTrick {
            flick: Flick::Kickflip,
            def: trick_def(Flick::Kickflip),
            t: 0.0,
        });
        // 0.6 s of flight: air + out (0.8 s) play at 2x in the 0.4 s after the 0.2 s pop.
        s.mode = Mode::Air { time: 0.3 };
        let p = pick(&s, 0.0, 0.3, &dur);
        assert_eq!(p.role, "kickflip/air");
        assert!((p.time - 0.2).abs() < 1e-5, "{p:?}");
        s.mode = Mode::Air { time: 0.5 };
        let p = pick(&s, 0.0, 0.1, &dur);
        assert_eq!(p.role, "kickflip/out");
        assert!((p.time - 0.2).abs() < 1e-5, "{p:?}");
        // A very long flight never slows the trick below MIN_SPEED.
        s.mode = Mode::Air { time: 1.2 };
        let p = pick(&s, 0.0, 9.0, &dur);
        assert_eq!(p.role, "kickflip/out");
        assert!((p.time - 0.2).abs() < 1e-4, "{p:?}");
    }
}
