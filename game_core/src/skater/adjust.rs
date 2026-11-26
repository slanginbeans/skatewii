//! Board adjust in the air: the C-stick turns and tips the deck against the body to line it up for a
//! grind or a landing (Skate 3's BoardAdjust; SK8-ENGINE/skate-3-rust-engine, GPL-3.0:
//! `skate-game/src/graph_host/action_board_adjust.rs`, `skate-core/src/player/input_phase/
//! pose_output.rs`).
//!
//! Skate 3 poses the deck with its `BLEND_ADJUST_*` clips and its physical board follows the animated
//! one, so the grind investigation's boxes and the landing see the adjusted deck. flatground keeps the
//! deck's yaw and pitch against the body ([`DeckAdjust`]): with the state graphs and the imported rider
//! they are the posed board's (while the action graph is adjusting), else the C-stick's at Skate 3's
//! measured angles and pace (`tuning::adjust`). [`Skater::frame`](super::Skater::frame) turns the deck
//! by them, and touching down on a rail or the ground bakes them into the board.

use crate::math::{Vec3, atan2f, cosf, sinf};
use crate::tuning::adjust::AdjustTuning;
use libm::asinf;

use super::{Skater, rotate};
use crate::input::Pad;

const DEG: f32 = crate::math::PI / 180.0;
/// How far past the stick's built-in pose a posed deck still counts ([`DeckAdjust::towards`]).
const SLACK: f32 = 1.25;

/// The deck against the body (rad): yaw about the body's up (+ swings the nose towards the board's
/// `up x nose` side), pitch about its right (+ nose up).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeckAdjust {
    pub yaw: f32,
    pub pitch: f32,
}

impl DeckAdjust {
    pub const NONE: DeckAdjust = DeckAdjust {
        yaw: 0.0,
        pitch: 0.0,
    };

    pub fn is_none(&self) -> bool {
        self.yaw == 0.0 && self.pitch == 0.0
    }

    /// The pose the C-stick asks for (`stick`: x right, y away from the player, within the unit disc):
    /// right swings the nose towards `-(up x nose)` (the clips' goofy rider's heel side), left the other
    /// way, up tips the nose down, down lifts it. A mirrored rider plays the other side's clip
    /// (BoardAdjust negates its angle) mirrored, so the same stick swings the nose the same way round by
    /// that clip's angle (measured through Skate 3's graphs: about 89 degrees for a regular rider's
    /// right).
    pub fn target(t: &AdjustTuning, stick: [f32; 2], mirrored: bool) -> DeckAdjust {
        let [x, y] = stick;
        let (right, left) = if mirrored {
            (t.yaw_left, t.yaw_right)
        } else {
            (t.yaw_right, t.yaw_left)
        };
        let yaw = -x * if x > 0.0 { right } else { left };
        let pitch = -y * if y > 0.0 { t.pitch_up } else { t.pitch_down };
        DeckAdjust {
            yaw: yaw * DEG,
            pitch: pitch * DEG,
        }
    }

    /// The posed board's adjust from its nose in the rider's frame (x `up x nose`, y up, z ahead).
    pub fn from_pose(nose: Vec3) -> DeckAdjust {
        DeckAdjust {
            yaw: atan2f(nose.x, nose.z),
            pitch: asinf(nose.y.clamp(-1.0, 1.0)),
        }
    }

    /// The part of this (posed) adjust that goes the way `stick` asks, up to a quarter past it: as the
    /// board-adjust clips blend in, the take-off clip's own pitch fades out under them, and flatground's
    /// board never had it.
    pub fn towards(self, stick: DeckAdjust) -> DeckAdjust {
        let within = |v: f32, s: f32| {
            let s = s * SLACK;
            v.clamp(s.min(0.0), s.max(0.0))
        };
        DeckAdjust {
            yaw: within(self.yaw, stick.yaw),
            pitch: within(self.pitch, stick.pitch),
        }
    }

    /// One tick towards `target`: `blend` of the way, at most `max_step` degrees.
    pub fn step(&mut self, t: &AdjustTuning, target: DeckAdjust) {
        let max = t.max_step * DEG;
        let go = |v: f32, to: f32| {
            let d = ((to - v) * t.blend).clamp(-max, max);
            // Settle exactly rather than creep forever.
            if (to - v).abs() < 1e-4 { to } else { v + d }
        };
        self.yaw = go(self.yaw, target.yaw);
        self.pitch = go(self.pitch, target.pitch);
    }

    /// The deck's nose and up from the body's (`fwd` square to `up`).
    pub fn apply(&self, fwd: Vec3, up: Vec3) -> (Vec3, Vec3) {
        let f = rotate(fwd, up, self.yaw);
        let (s, c) = (sinf(self.pitch), cosf(self.pitch));
        (f * c + up * s, up * c - f * s)
    }
}

impl Skater {
    /// One tick of board adjust in the air. The action graph decides (its BoardAdjust behaviour
    /// publishing the C-stick, `GraphControl::board_adjust`) and the deck goes where the clips pose it
    /// (`GraphControl::pose_deck`, the imported rider's last pose).
    pub(super) fn update_deck_adjust(&mut self, pad: &Pad) {
        let t = self.tuning.adjust;
        let stick = DeckAdjust::target(&t, pad.flick, self.mirrored());
        let g = self.graph;
        let target = if g.board_adjust {
            g.pose_deck
                .map_or(stick, |nose| DeckAdjust::from_pose(nose).towards(stick))
        } else if g.adjust_posed {
            // Let go: the deck follows the clips back as they blend out, only ever back towards square.
            g.pose_deck.map_or(DeckAdjust::NONE, |nose| {
                DeckAdjust::from_pose(nose).towards(self.deck_adjust)
            })
        } else {
            DeckAdjust::NONE
        };
        self.deck_adjust.step(&t, target);
    }

    /// Touching down (a rail or the ground): the adjusted deck is the board now, and the body rides it.
    pub(super) fn bake_deck_adjust(&mut self) {
        if self.deck_adjust.is_none() {
            return;
        }
        let up = self.normal.normalize_or(Vec3::UP);
        let fwd = super::tangent(self.fwd, up);
        let (fwd, up) = self.deck_adjust.apply(fwd, up);
        self.fwd = fwd;
        self.normal = up;
        self.sync_heading();
        self.deck_adjust = DeckAdjust::NONE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;

    const T: AdjustTuning = AdjustTuning::BUILT_IN;

    #[test]
    fn the_stick_asks_skate_3s_measured_angles() {
        let right = DeckAdjust::target(&T, [1.0, 0.0], false);
        assert!((right.yaw / DEG + 80.0).abs() < 1e-3 && right.pitch == 0.0);
        let left = DeckAdjust::target(&T, [-1.0, 0.0], false);
        assert!((left.yaw / DEG - 88.4).abs() < 1e-3);
        // Mirrored: the same way round, by the other clip's angle.
        let mirrored = DeckAdjust::target(&T, [1.0, 0.0], true);
        assert!((mirrored.yaw / DEG + 88.4).abs() < 1e-3);
        let mirrored = DeckAdjust::target(&T, [-1.0, 0.0], true);
        assert!((mirrored.yaw / DEG - 80.0).abs() < 1e-3);
        let up = DeckAdjust::target(&T, [0.0, 1.0], false);
        assert!((up.pitch / DEG + 19.7).abs() < 1e-3 && up.yaw == 0.0);
        let down = DeckAdjust::target(&T, [0.0, -0.5], false);
        assert!((down.pitch / DEG - 23.9 * 0.5).abs() < 1e-3);
        assert!(DeckAdjust::target(&T, [0.0, 0.0], false).is_none());
    }

    #[test]
    fn the_deck_follows_at_the_clips_pace_and_settles() {
        let mut a = DeckAdjust::NONE;
        let to = DeckAdjust::target(&T, [1.0, 0.0], false);
        a.step(&T, to);
        // The first tick is capped at 13 degrees.
        assert!((a.yaw / DEG + 13.0).abs() < 1e-3, "{}", a.yaw / DEG);
        let mut ticks = 1;
        while (a.yaw - to.yaw).abs() > 8.0 * DEG {
            a.step(&T, to);
            ticks += 1;
        }
        // Skate 3's clips: within 8 degrees of the full turn 7 ticks after it starts moving.
        assert!((5..=8).contains(&ticks), "{ticks} ticks");
        for _ in 0..30 {
            a.step(&T, to);
        }
        assert_eq!(a, to);
        // Let go: back square to the body.
        for _ in 0..40 {
            a.step(&T, DeckAdjust::NONE);
        }
        assert!(a.is_none());
    }

    #[test]
    fn the_adjusted_deck_turns_and_tips_from_the_body() {
        let fwd = v3(0.0, 0.0, 1.0);
        let up = Vec3::UP;
        let a = DeckAdjust {
            yaw: 90.0 * DEG,
            pitch: 0.0,
        };
        let (f, u) = a.apply(fwd, up);
        // + yaw swings the nose towards up x nose (+x here).
        assert!((f - v3(1.0, 0.0, 0.0)).length() < 1e-5 && (u - up).length() < 1e-5);
        let p = DeckAdjust {
            yaw: 0.0,
            pitch: 20.0 * DEG,
        };
        let (f, u) = p.apply(fwd, up);
        assert!((f.y - sinf(20.0 * DEG)).abs() < 1e-5 && f.dot(u).abs() < 1e-5);
        // The posed nose reads back as the same adjust.
        let back = DeckAdjust::from_pose(f);
        assert!(back.yaw.abs() < 1e-5 && (back.pitch - 20.0 * DEG).abs() < 1e-5);
        let side = DeckAdjust::from_pose(v3(-0.982, -0.07, 0.174).normalize_or(fwd));
        assert!((side.yaw / DEG + 80.0).abs() < 0.2, "{}", side.yaw / DEG);
        // Only the way the stick asks counts: the clip's slight nose-down under a sideways adjust, and
        // the take-off's nose-up fading under a nose-down one, are not the adjust.
        let right = DeckAdjust::target(&T, [1.0, 0.0], false);
        let kept = side.towards(right);
        assert!(kept.pitch == 0.0 && kept.yaw == side.yaw, "{kept:?}");
        let up = DeckAdjust::target(&T, [0.0, 1.0], false);
        let fading = DeckAdjust {
            yaw: 0.0,
            pitch: 24.0 * DEG,
        };
        assert!(fading.towards(up).is_none());
    }
}
