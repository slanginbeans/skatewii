//! Drives an imported [`Character`] from the skater's state: picks a clip for what the skater is doing,
//! crossfades between clips, poses and skins it once per step.
//!
//! Clip roles are the `skater/clip/<role>` entries `fg_import skater` writes: `idle`, `crouch`, `push`,
//! `land/*`, `manual`, `nosemanual`, `grab/*`, `bail/fall` + `bail/getup`, `lean/*` (single poses blended
//! over the riding clips by steering),
//! and `<trick>/pop` + `<trick>/air` for each flick trick. A missing clip falls back to `idle`.

use alloc::vec;
use alloc::vec::Vec;

use crate::character::{Character, Local};
use crate::flick::Flick;
use crate::gfx::SUN;
use crate::math::{Mat3, sqrtf, v3};
use crate::skater::{GrabKind, GrindKind, LandKind, Mode, Skater};

/// Crossfade time between clips (s).
const FADE: f32 = 0.12;
/// Crouch amount above which the crouch clip plays.
const CROUCH_MIN: f32 = 0.05;
/// Playback speed limits when fitting a trick into the flight.
const MIN_SPEED: f32 = 0.6;
const MAX_SPEED: f32 = 2.5;
/// How far full steering blends towards the lean pose.
const LEAN_WEIGHT: f32 = 0.85;

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
        Mode::Bailed { time } => {
            // The fall, then getting up (holding its last frame until the respawn).
            let fall = duration("bail/fall");
            if time < fall {
                Pick {
                    role: "bail/fall",
                    time,
                }
            } else if has("bail/getup") {
                Pick {
                    role: "bail/getup",
                    time: time - fall,
                }
            } else if has("bail/fall") {
                Pick {
                    role: "bail/fall",
                    time: fall,
                }
            } else {
                idle
            }
        }
        Mode::Grind => {
            let Some(g) = &s.grind else {
                return idle;
            };
            let role = match g.kind {
                GrindKind::FiftyFifty => "grind/5050",
                GrindKind::Boardslide => "grind/bslide",
            };
            if has(role) {
                Pick { role, time: g.time }
            } else {
                idle
            }
        }
        Mode::Air { time } => {
            if let Some(g) = &s.grab {
                let role = grab_role(g.kind);
                if has(role) {
                    return Pick { role, time: g.time };
                }
            }
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
            if let Some(m) = &s.manual {
                let role = if m.nose { "nosemanual" } else { "manual" };
                if has(role) {
                    return Pick { role, time: m.time };
                }
            }
            let landing = s.landing.map(|l| (land_role(l.kind), l.time));
            if s.crouch > CROUCH_MIN && has("crouch") {
                Pick {
                    role: "crouch",
                    time: s.crouch.min(1.0) * duration("crouch"),
                }
            } else if let Some((role, time)) = landing.filter(|(r, t)| *t < duration(r)) {
                Pick { role, time }
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

pub fn grab_role(k: GrabKind) -> &'static str {
    match k {
        GrabKind::Indy => "grab/fs",
        GrabKind::Melon => "grab/bs",
        GrabKind::Mute => "grab/mute",
    }
}

pub fn land_role(k: LandKind) -> &'static str {
    match k {
        LandKind::Clean => "land/clean",
        LandKind::Heavy => "land/heavy",
        LandKind::Sketchy => "land/sketchy",
    }
}

/// The steering lean laid over the riding clips: which pose to blend towards, and how far (0..1).
/// `None` when the skater is doing anything but rolling along (manuals, tricks, bails, landings) or
/// the pack has no lean poses.
pub fn lean(s: &Skater, base: &str, has: &dyn Fn(&str) -> bool) -> Option<(&'static str, f32)> {
    if s.mode != Mode::Rolling || s.manual.is_some() || !matches!(base, "idle" | "push" | "crouch")
    {
        return None;
    }
    let crouched = base == "crouch";
    // Stick right (positive) turns the board right.
    let role = match (s.steer >= 0.0, crouched) {
        (true, false) => "lean/right",
        (false, false) => "lean/left",
        (true, true) => "lean/crouch_right",
        (false, true) => "lean/crouch_left",
    };
    let w = (s.steer.abs() * LEAN_WEIGHT).min(1.0);
    (w > 0.02 && has(role)).then_some((role, w))
}

pub struct Rider {
    pub character: Character,
    cur: Pick,
    /// The clip faded out from, frozen where it was left.
    prev: Option<Pick>,
    fade: f32,
    lean: Option<(&'static str, f32)>,
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
            lean: None,
            a: vec![Local::IDENTITY; n],
            b: vec![Local::IDENTITY; n],
        };
        r.pose_and_skin(Mat3::IDENTITY);
        r
    }

    pub fn current(&self) -> Pick {
        self.cur
    }

    /// The lean pose blended over the current clip, and how far.
    pub fn lean(&self) -> Option<(&'static str, f32)> {
        self.lean
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
        self.lean = lean(s, next.role, &|r| ch.has_clip(r));
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
        // The lean poses are single frames: blend the whole body part way towards one.
        if let Some((role, w)) = self.lean
            && ch.sample(role, 0.0, &mut self.b)
        {
            for (a, b) in self.a.iter_mut().zip(&self.b) {
                *a = a.blend(b, w);
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

#[cfg(test)]
mod role_tests {
    use super::*;
    use crate::park::Park;
    use crate::skater::{Grab, Landing, Manual};

    fn dur(r: &str) -> f32 {
        match r {
            "idle" => 2.0,
            "push" => 0.7,
            "land/clean" => 0.6,
            "land/sketchy" => 1.0,
            "manual" => 3.0,
            "grab/fs" => 1.0,
            "bail/fall" => 0.6,
            "bail/getup" => 1.5,
            "lean/left" | "lean/right" => 3.0,
            _ => 0.0,
        }
    }

    #[test]
    fn landings_manuals_grabs_and_bails() {
        let park = Park::demo();
        let mut s = Skater::spawn(&park);
        s.landing = Some(Landing {
            kind: LandKind::Sketchy,
            time: 0.4,
        });
        assert_eq!(
            pick(&s, 0.0, 0.0, &dur),
            Pick {
                role: "land/sketchy",
                time: 0.4
            }
        );
        // Past the clip's end: back to riding. A missing clip (heavy) is skipped too.
        s.landing = Some(Landing {
            kind: LandKind::Clean,
            time: 0.7,
        });
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "idle");
        s.landing = Some(Landing {
            kind: LandKind::Heavy,
            time: 0.1,
        });
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "idle");

        s.manual = Some(Manual {
            nose: false,
            balance: 0.0,
            time: 1.2,
        });
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "manual");
        // No nose manual clip in this pack: plain riding.
        s.manual = Some(Manual {
            nose: true,
            balance: 0.0,
            time: 1.2,
        });
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "idle");
        s.manual = None;

        s.mode = Mode::Air { time: 0.5 };
        s.grab = Some(Grab {
            kind: GrabKind::Indy,
            time: 0.3,
        });
        assert_eq!(
            pick(&s, 0.0, 0.2, &dur),
            Pick {
                role: "grab/fs",
                time: 0.3
            }
        );
        s.grab = None;

        s.mode = Mode::Bailed { time: 0.2 };
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "bail/fall");
        s.mode = Mode::Bailed { time: 1.0 };
        let p = pick(&s, 0.0, 0.0, &dur);
        assert_eq!(p.role, "bail/getup");
        assert!((p.time - 0.4).abs() < 1e-5);
    }

    #[test]
    fn leans_into_turns_only_while_riding() {
        let park = Park::demo();
        let mut s = Skater::spawn(&park);
        let has = |r: &str| dur(r) > 0.0;
        assert_eq!(lean(&s, "idle", &has), None, "straight: no lean");
        s.steer = -1.0;
        let (role, w) = lean(&s, "idle", &has).unwrap();
        assert_eq!(role, "lean/left");
        assert!((w - LEAN_WEIGHT).abs() < 1e-6);
        s.steer = 0.5;
        assert_eq!(lean(&s, "push", &has).map(|l| l.0), Some("lean/right"));
        // No crouched lean poses in this pack.
        assert_eq!(lean(&s, "crouch", &has), None);
        assert_eq!(lean(&s, "land/clean", &has), None);
        s.mode = Mode::Air { time: 0.1 };
        assert_eq!(lean(&s, "idle", &has), None);
    }
}
