//! Pushing under the motion graph: SK8-ENGINE/skate-3-rust-engine's port of Skate 3's push behaviours
//! (GPL-3.0; crates/skate-core/src/riding/push_behaviors.rs and push_animation.rs, crates/skate-game/src/
//! graph_host/pushing.rs and pushing_settings.rs).
//!
//! The push trees blend four push sets (slow/fast start speed x strong/gentle push) by three parameters,
//! `HSTR_VEL_B`, `LSTR_VEL_B` (where the start speed falls between the slow and fast set) and `VEL_E` (how
//! far the push goes towards the strong set). `ComputeTargetCoefsFromSpeedAndStrength` picks the targets
//! from the board's speed and the push strength, `SetPushCoefs` eases the tree's values towards them, and
//! the physics chases the speed the same values give (the blended clips' end speed), so what the rider
//! does and what the board does come from one set of numbers.
//!
//! The push sets' lengths and speeds come from the player's motion library (the `_CYC1` clips' length and
//! their `HSTR_VEL_B` / `LSTR_VEL_B` and `VEL_E` attributes); the curves from the rules pack
//! (`anim_motion/pushing`). Without them flatground's own numbers stand in.

use crate::motion::MotionDb;
use crate::tuning::Tuning;

/// One push set's `_CYC1` clip: its length (s), the speed it starts at and ends at (m/s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushClip {
    pub length: f32,
    pub begin: f32,
    pub end: f32,
}

/// The tree parameters (`HSTR_VEL_B`, `LSTR_VEL_B`, `VEL_E`), 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefs {
    pub strong: f32,
    pub gentle: f32,
    pub strength: f32,
}

impl Coefs {
    /// "Not set yet" (SK8's negative sentinel): the first ease jumps to the target.
    pub const UNSET: Coefs = Coefs {
        strong: -1.0,
        gentle: -1.0,
        strength: -1.0,
    };
    pub const ZERO: Coefs = Coefs {
        strong: 0.0,
        gentle: 0.0,
        strength: 0.0,
    };
}

/// A foot's four push sets (slow strong, fast strong, slow gentle, fast gentle) and their ranges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushSets {
    pub clips: [PushClip; 4],
    min_begin: f32,
    max_begin: f32,
    min_delta: f32,
    max_delta: f32,
}

/// The push sets' clips in the motion library, regular foot then mongo.
pub const REGULAR_CLIPS: [&str; 4] = [
    "R_PUSHLSP_HSTR_N_0_CYC1",
    "R_PUSHHSP_HSTR_N_0_CYC1",
    "R_PUSHLSP_LSTR_N_0_CYC1",
    "R_PUSHHSP_LSTR_N_0_CYC1",
];
pub const MONGO_CLIPS: [&str; 4] = [
    "R_PUSHLSP_HSTR_MONGO_0_CYC1",
    "R_PUSHHSP_HSTR_MONGO_0_CYC1",
    "R_PUSHLSP_LSTR_MONGO_0_CYC1",
    "R_PUSHHSP_LSTR_MONGO_0_CYC1",
];

/// flatground's own push sets (contact length, start and end speed).
pub const BUILT_IN: [PushClip; 4] = [
    PushClip {
        length: 0.5,
        begin: 0.0,
        end: 4.0,
    },
    PushClip {
        length: 0.4,
        begin: 8.0,
        end: 10.0,
    },
    PushClip {
        length: 0.5,
        begin: 0.0,
        end: 1.5,
    },
    PushClip {
        length: 0.4,
        begin: 8.0,
        end: 8.5,
    },
];

impl PushSets {
    pub fn new(clips: [PushClip; 4]) -> PushSets {
        let begin = clips.map(|c| c.begin);
        let delta = clips.map(|c| c.end - c.begin);
        PushSets {
            clips,
            min_begin: min(min(begin[0], begin[1]), min(begin[2], begin[3])),
            max_begin: max(max(begin[0], begin[1]), max(begin[2], begin[3])),
            min_delta: min(min(delta[0], delta[1]), min(delta[2], delta[3])),
            max_delta: max(max(delta[0], delta[1]), max(delta[2], delta[3])),
        }
    }

    /// The sets named `names` in the motion library (length, `HSTR_VEL_B` or `LSTR_VEL_B`, `VEL_E`);
    /// `None` when any is missing.
    pub fn from_motion(db: &MotionDb, names: [&str; 4]) -> Option<PushSets> {
        let mut clips = BUILT_IN;
        for (c, name) in clips.iter_mut().zip(names) {
            let i = db.clip_index(name)?;
            let attr = |a: &str| db.attribute_at(i, a, 0.0).map(|v| v.value);
            *c = PushClip {
                length: db.clip(i).length(),
                begin: attr("HSTR_VEL_B").or_else(|| attr("LSTR_VEL_B"))?,
                end: attr("VEL_E")?,
            };
            if c.length.is_nan() || c.length <= 0.0 {
                return None;
            }
        }
        Some(PushSets::new(clips))
    }

    /// ComputeTargetCoefsFromSpeedAndStrength: the tree parameters for a push of `strength` (m/s added)
    /// at `speed`. The blend weights account for the sets' different lengths (distance, not speed, is
    /// what blends linearly).
    pub fn target(&self, speed: f32, strength: f32) -> Coefs {
        let speed = bounded(speed.max(0.0), self.min_begin, self.max_begin);
        let strength = bounded(strength, self.min_delta, self.max_delta);
        let [ss, fs, sg, fg] = self.clips;
        let strong = velocity_blend(
            bounded(speed, ss.begin, fs.begin),
            ss.length,
            ss.begin,
            fs.length,
            fs.begin,
        );
        let gentle = velocity_blend(
            bounded(speed, sg.begin, fg.begin),
            sg.length,
            sg.begin,
            fg.length,
            fg.begin,
        );
        let strong_len = (1.0 - strong) * ss.length + fs.length * strong;
        let gentle_len = (1.0 - gentle) * sg.length + fg.length * gentle;
        let strong_end = blended_end(strong, ss, fs);
        let gentle_end = blended_end(gentle, sg, fg);
        let end = bounded(strength + speed, gentle_end, strong_end);
        Coefs {
            strong: unit(strong),
            gentle: unit(gentle),
            strength: unit(velocity_blend(
                end, gentle_len, gentle_end, strong_len, strong_end,
            )),
        }
    }

    /// The speed the push sets blended by `c` end at: what the board chases while the foot is down.
    pub fn end_speed(&self, c: Coefs) -> f32 {
        let [ss, fs, sg, fg] = self.clips;
        let (strong, gentle) = (unit(c.strong), unit(c.gentle));
        let strong_set = PushClip {
            length: (1.0 - strong) * ss.length + fs.length * strong,
            begin: 0.0,
            end: blended_end(strong, ss, fs),
        };
        let gentle_set = PushClip {
            length: (1.0 - gentle) * sg.length + fg.length * gentle,
            begin: 0.0,
            end: blended_end(gentle, sg, fg),
        };
        blended_end(c.strength, gentle_set, strong_set)
    }

    /// ComputeRepushDeadline (on leaving the push contact): how far through the next push tree the push
    /// may be broken out of (the "out factor"), from the push's strength and speed.
    pub fn out_factor(&self, speed: f32, strength: f32, speed_weight: f32, most: f32) -> f32 {
        let first = self.min_begin + self.min_delta;
        let speed = bounded(speed.max(0.0), first, self.max_begin);
        let strength = bounded(strength, self.min_delta, self.max_delta);
        let k_strength = unit(ratio(
            strength - self.min_delta,
            self.max_delta - self.min_delta,
        ));
        let k_speed = unit(ratio(speed - first, self.max_begin - first));
        let combined = k_strength * (1.0 - speed_weight) + k_speed * speed_weight;
        min(most, combined.max(0.0))
    }
}

/// The push curves and limits (`anim_motion/pushing`).
#[derive(Clone, Copy, Debug)]
pub struct PushCurves {
    pub tuning: Tuning,
}

/// Seconds of one of Skate 3's animation frames (its blend rates are in frames).
const FRAME: f32 = 1.0 / 60.0;

impl PushCurves {
    /// The push strength (speed added, m/s) for a button held `held` seconds at `speed`.
    pub fn strength(&self, held: f32, speed: f32) -> f32 {
        let t = &self.tuning;
        let full = t.push_hold_time.eval(speed);
        let k = if full > 0.0 { held / full } else { 1.0 };
        t.push_dv.eval(unit(k))
    }

    /// SetPushCoefs: eases the tree's values towards the target, the start-speed blends at a rate set
    /// by the strength blend, the strength at a rate set by the speed.
    pub fn ease(&self, current: Coefs, target: Coefs, speed: f32, dt: f32) -> Coefs {
        let t = &self.tuning;
        let current = if current.strength < 0.0 {
            target
        } else {
            current
        };
        let rate = |frames: f32| {
            if frames > 0.0 {
                1.0 / (frames * FRAME)
            } else {
                f32::INFINITY
            }
        };
        let vel = rate(t.push_blend_speed_frames.eval(current.strength));
        let acc = rate(t.push_blend_acc_frames.eval(speed.max(0.0)));
        Coefs {
            strong: approach(current.strong, target.strong, vel, dt),
            gentle: approach(current.gentle, target.gentle, vel, dt),
            strength: approach(current.strength, target.strength, acc, dt),
        }
    }
}

/// The push state the push behaviours share (SK8 PushState).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushState {
    pub out_factor: f32,
    /// The push strength so far (m/s to add).
    pub dv: f32,
    pub current: Coefs,
    pub target: Coefs,
    /// A held or re-pressed button keeps the push cycling.
    pub continue_push: bool,
    /// The last `ComputeTargetCoefs` used the regular foot's sets (else mongo).
    pub regular: bool,
}

impl Default for PushState {
    fn default() -> Self {
        PushState {
            out_factor: 0.0,
            dv: 0.0,
            current: Coefs::ZERO,
            target: Coefs::ZERO,
            continue_push: false,
            regular: true,
        }
    }
}

impl PushState {
    /// InitPush (keeps the out factor).
    pub fn init(&mut self) {
        self.continue_push = false;
        self.dv = 0.0;
        self.current = Coefs::UNSET;
        self.target = Coefs::UNSET;
    }
}

/// ComputeFirstPushStrength's instance: the first push's strength grows while its button is held (or,
/// just after the rider was put on the board, as if it were held).
#[derive(Clone, Copy, Debug, Default)]
pub struct FirstPush {
    pub simulated: f32,
    pub from_teleport: bool,
    pub first: bool,
}

impl FirstPush {
    pub fn begin(time_since_teleport: f32, window: f32) -> FirstPush {
        FirstPush {
            simulated: 0.0,
            from_teleport: time_since_teleport < window,
            first: true,
        }
    }

    /// `pushing`: the motion intent `Pushing` (seconds the button has been held), if present.
    pub fn update(
        &mut self,
        s: &mut PushState,
        c: &PushCurves,
        pushing: Option<f32>,
        speed: f32,
        dt: f32,
    ) {
        if pushing.is_none() {
            self.first = false;
        }
        if self.first || self.from_teleport {
            let held = if self.from_teleport {
                let h = self.simulated;
                self.simulated += dt;
                h
            } else {
                pushing.unwrap_or(0.0)
            };
            let k = c.strength(held, speed.max(0.0));
            if k > s.dv {
                s.dv = k;
            }
        }
    }
}

/// PushCycle's phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    HoldingFirst,
    WaitingNext,
    HoldingNext,
    NextReleased,
}

/// PushCycle's instance: a held button keeps adding strength (up to `max_holding_acc`); a new press
/// during the cycle queues the next push at the strength its hold gives.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cycle {
    pub phase: Phase,
    pub next_dv: f32,
}

/// The motion intents PushCycle reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct Intents {
    pub pushing: Option<f32>,
    pub new_push: bool,
    /// The cycle's own foot (`RightPush` / `LeftPush`) is held.
    pub foot: bool,
}

impl Cycle {
    pub fn begin(s: &mut PushState, c: &PushCurves, i: Intents) -> Cycle {
        let holding = i.pushing.is_some() && i.foot;
        s.continue_push = holding;
        Cycle {
            phase: if holding {
                Phase::HoldingFirst
            } else {
                Phase::WaitingNext
            },
            next_dv: c.tuning.push_dv.eval(0.0),
        }
    }

    pub fn update(&mut self, s: &mut PushState, c: &PushCurves, i: Intents, speed: f32) {
        let speed = speed.max(0.0);
        let most = c.tuning.push_hold_cap;
        if self.phase == Phase::HoldingFirst {
            if let Some(held) = i.pushing {
                self.next_dv = c.strength(held, speed);
                if self.next_dv > most {
                    self.next_dv = most;
                    s.dv = most;
                }
                if s.dv < self.next_dv {
                    s.dv = self.next_dv;
                }
            } else {
                self.phase = Phase::WaitingNext;
                self.next_dv = c.tuning.push_dv.eval(0.0);
                s.continue_push = false;
            }
        }
        if self.phase == Phase::WaitingNext && i.new_push && i.foot {
            self.phase = Phase::HoldingNext;
        }
        if self.phase == Phase::HoldingNext && !i.foot {
            self.phase = Phase::NextReleased;
            s.dv = self.next_dv;
        }
        if self.phase == Phase::HoldingNext {
            self.next_dv = c.strength(i.pushing.unwrap_or(0.0), speed);
            if s.dv < self.next_dv {
                s.dv = self.next_dv;
            }
            s.continue_push = true;
        }
    }
}

fn velocity_blend(speed: f32, low_len: f32, low_vel: f32, high_len: f32, high_vel: f32) -> f32 {
    let low_dist = low_len * low_vel;
    let speed_dist = speed * low_len;
    let sum = high_len * high_vel + speed_dist;
    let num = speed_dist - low_dist;
    let den = -(speed * high_len - (sum - low_dist));
    unit(ratio(num, den))
}

fn blended_end(k: f32, low: PushClip, high: PushClip) -> f32 {
    let k = unit(k);
    let hw = k * high.length;
    let lw = (1.0 - k) * low.length;
    let w = hw + lw;
    if w > 0.0 {
        (high.end * hw + low.end * lw) / w
    } else {
        low.end
    }
}

/// A division that is 0 instead of NaN or infinite (sets with equal speeds).
fn ratio(a: f32, b: f32) -> f32 {
    let r = a / b;
    if r.is_finite() { r } else { 0.0 }
}

fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let d = target - current;
    let r = if d.abs() > 0.001 {
        let step = dt * rate;
        if d > 0.0 {
            (current + step).min(target)
        } else {
            (current - step).max(target)
        }
    } else {
        target
    };
    unit(r)
}

fn min(a: f32, b: f32) -> f32 {
    if a - b >= 0.0 { b } else { a }
}
fn max(a: f32, b: f32) -> f32 {
    if a - b >= 0.0 { a } else { b }
}
fn bounded(v: f32, lo: f32, hi: f32) -> f32 {
    min(hi, max(lo, v))
}
fn unit(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sets() -> PushSets {
        PushSets::new(BUILT_IN)
    }

    #[test]
    fn targets_follow_speed_and_strength() {
        let s = sets();
        // Standing still, a weak push: the slow gentle set.
        let c = s.target(0.0, 0.0);
        assert_eq!((c.strong, c.gentle), (0.0, 0.0));
        assert!(c.strength < 0.05, "{c:?}");
        // A strong push from standing: all strong.
        let c = s.target(0.0, 10.0);
        assert!(c.strength > 0.99, "{c:?}");
        // Fast: the fast sets.
        let c = s.target(8.0, 1.0);
        assert_eq!((c.strong, c.gentle), (1.0, 1.0));
        // Somewhere between.
        let c = s.target(4.0, 2.0);
        assert!(c.strong > 0.0 && c.strong < 1.0, "{c:?}");
    }

    #[test]
    fn the_board_chases_the_speed_the_tree_blends_to() {
        let s = sets();
        for (speed, strength) in [(0.0, 1.0), (2.0, 2.0), (5.0, 0.5), (3.0, 3.0)] {
            let c = s.target(speed, strength);
            let end = s.end_speed(c);
            // Inside what the sets can do, the blended end is the speed plus the strength.
            let [ss, fs, sg, fg] = s.clips;
            let strong_end = blended_end(c.strong, ss, fs);
            let gentle_end = blended_end(c.gentle, sg, fg);
            let want = (speed + strength).clamp(gentle_end, strong_end);
            assert!(
                (end - want).abs() < 0.01,
                "{speed} {strength}: {end} {want}"
            );
        }
    }

    #[test]
    fn out_factor_grows_with_strength_and_speed() {
        let s = sets();
        let weak = s.out_factor(0.0, 0.0, 0.5, 0.9);
        let strong = s.out_factor(0.0, 4.0, 0.5, 0.9);
        let fast = s.out_factor(8.0, 4.0, 0.5, 0.9);
        assert!(weak < strong && strong < fast, "{weak} {strong} {fast}");
        assert!(fast <= 0.9);
    }

    #[test]
    fn a_held_cycle_adds_strength_up_to_the_cap_and_a_release_ends_it() {
        let c = PushCurves {
            tuning: Tuning::BUILT_IN,
        };
        let mut s = PushState::default();
        s.init();
        let held = |t: f32| Intents {
            pushing: Some(t),
            new_push: false,
            foot: true,
        };
        let mut cy = Cycle::begin(&mut s, &c, held(0.1));
        assert!(s.continue_push);
        for k in 1..60 {
            cy.update(&mut s, &c, held(k as f32 / 60.0), 3.0);
        }
        assert!(s.dv > 0.0 && s.dv <= c.tuning.push_hold_cap);
        cy.update(&mut s, &c, Intents::default(), 3.0);
        assert!(!s.continue_push);
        assert_eq!(cy.phase, Phase::WaitingNext);
        // A new press queues the next push.
        cy.update(
            &mut s,
            &c,
            Intents {
                pushing: Some(0.0),
                new_push: true,
                foot: true,
            },
            3.0,
        );
        assert!(s.continue_push);
    }

    #[test]
    fn easing_starts_at_the_target_then_moves_at_its_rate() {
        let c = PushCurves {
            tuning: Tuning::BUILT_IN,
        };
        let t = Coefs {
            strong: 1.0,
            gentle: 1.0,
            strength: 1.0,
        };
        assert_eq!(c.ease(Coefs::UNSET, t, 0.0, 1.0 / 60.0), t);
        let e = c.ease(Coefs::ZERO, t, 0.0, 1.0 / 60.0);
        assert!(e.strength > 0.0 && e.strength < 1.0, "{e:?}");
    }
}
