//! What the motion graph plays: animation trees joined by transitions (crossfades, sequences), as
//! SK8-ENGINE/skate-3-rust-engine plays them (GPL-3.0; crates/skate-core/src/animation/
//! playback_transition.rs and the `play` of crates/skate-game/src/graph_host/motion_animation.rs).
//!
//! - A blend keeps the outgoing tree playing while the new one fades in; its weight is a smoothstep of
//!   elapsed / time for times of 0.05 s and more (linear below).
//! - `blendWithCurrentFrame` freezes the outgoing tree; match phase / match frame line the new tree up
//!   with the old one.
//! - A sequence plays the old tree to its end, then the new one.
//! - `transitionUnder` starts the new tree under a running transition (the transition's target becomes
//!   a transition itself).
//! - Each tree remembers whether it was started mirrored (the stance), so poses are mirrored per tree
//!   before they are blended.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::ops::TransitionDef;
use crate::motion::tree::{Advance, Attr, Param, Playback, PoseCommand};
use crate::motion::{MotionDb, NameId};

/// A playing clip with its share of the pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leaf {
    pub clip: u16,
    /// Seconds into the clip.
    pub time: f32,
    pub previous: f32,
    pub loops: u32,
    pub weight: f32,
    pub mirror: bool,
}

#[derive(Clone, Debug)]
pub struct Tree {
    pub play: Playback,
    pub mirror: bool,
    /// The graph's name for it (string id), for logs.
    pub name: u16,
}

#[derive(Clone, Debug)]
pub enum Node {
    Tree(Box<Tree>),
    Transition(Box<Transition>),
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub from: Node,
    pub to: Node,
    pub def: TransitionDef,
    elapsed: f32,
    sequence_done: bool,
}

const SMOOTH_FROM: f32 = 0.05;

impl Transition {
    pub fn new(from: Node, to: Node, def: TransitionDef) -> Transition {
        Transition {
            from,
            to,
            def,
            elapsed: 0.0,
            sequence_done: false,
        }
    }

    pub fn complete(&self) -> bool {
        if self.def.kind == 4 {
            self.sequence_done
        } else {
            self.elapsed > self.def.seconds
        }
    }

    pub fn weight(&self) -> f32 {
        let w = if self.def.seconds > 0.0 {
            (self.elapsed / self.def.seconds).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if self.def.seconds < SMOOTH_FROM {
            return w;
        }
        w * w * (3.0 - 2.0 * w)
    }
}

impl Node {
    pub fn tree(t: Tree) -> Node {
        Node::Tree(Box::new(t))
    }

    pub fn length(&self) -> f32 {
        match self {
            Node::Tree(t) => t.play.length(),
            Node::Transition(t) => t.to.length(),
        }
    }

    pub fn time(&self) -> f32 {
        match self {
            Node::Tree(t) => t.play.time(),
            Node::Transition(t) => t.to.time(),
        }
    }

    pub fn set_time(&mut self, time: f32) {
        match self {
            Node::Tree(t) => t.play.set_time(time),
            Node::Transition(t) => t.to.set_time(time),
        }
    }

    pub fn advance(&mut self, dt: f32, result: &mut Advance) {
        match self {
            Node::Tree(t) => t.play.advance(dt, 0.0, result),
            Node::Transition(t) => {
                let mut discard = Advance::default();
                if t.def.kind == 4 {
                    if t.sequence_done {
                        t.to.advance(dt, result);
                    } else {
                        t.from.advance(dt, &mut discard);
                        if discard.crossed_end {
                            t.sequence_done = true;
                            t.to.advance(discard.overshoot.max(0.0), result);
                        } else {
                            result.crossed_end = false;
                            result.remaining_before_wrap =
                                t.to.length() + discard.remaining_before_wrap;
                        }
                    }
                    return;
                }
                if t.def.matching != 1 {
                    t.from.advance(dt, &mut discard);
                }
                if t.def.matching == 3 {
                    let time = t.from.time().min(t.to.length());
                    t.to.set_time(time);
                } else if t.def.matching == 2 && t.from.length() > 0.0 {
                    let f = t.from.time() / t.from.length();
                    let l = t.to.length();
                    t.to.set_time(l * f);
                } else {
                    t.to.advance(dt, result);
                }
                t.elapsed += dt;
            }
        }
    }

    /// Drops finished transitions (their outgoing side no longer shows).
    pub fn collapse(self) -> Node {
        match self {
            Node::Transition(t) if t.complete() => t.to.collapse(),
            Node::Transition(mut t) => {
                let to = core::mem::replace(&mut t.to, Node::tree(empty()));
                t.to = to.collapse();
                Node::Transition(t)
            }
            n => n,
        }
    }

    pub fn set_parameters(&mut self, db: &MotionDb, params: &[Param]) {
        match self {
            Node::Tree(t) => {
                t.play.set_parameters(db, params);
            }
            Node::Transition(t) => {
                t.to.set_parameters(db, params);
                if t.def.kind == 4 && !t.sequence_done {
                    t.from.set_parameters(db, params);
                }
            }
        }
    }

    /// The tree whose attributes count (the target, or the playing first half of a sequence).
    fn attribute_side(&self) -> &Node {
        match self {
            Node::Transition(t) if t.def.kind == 4 && !t.sequence_done => &t.from,
            Node::Transition(t) => &t.to,
            n => n,
        }
    }

    pub fn query_attribute(&self, db: &MotionDb, name: NameId, mask: u32, out: &mut Attr) -> bool {
        match self.attribute_side() {
            Node::Tree(t) => t.play.query_attribute(db, name, mask, out),
            n => n.query_attribute(db, name, mask, out),
        }
    }

    /// The tree on top (the newest).
    pub fn top(&self) -> &Tree {
        match self {
            Node::Tree(t) => t,
            Node::Transition(t) => t.to.top(),
        }
    }

    pub fn in_transition(&self) -> bool {
        matches!(self, Node::Transition(t) if !t.complete())
    }

    /// What to pose this frame, appended to `out`: each tree's clips as one weighted group, joined by
    /// the transitions' blends (a channel blend takes each bone by its channel weight). Returns whether
    /// it pushed a pose. `update_history` commits the clocks (start the next step's event window).
    pub fn program(
        &mut self,
        update_history: bool,
        out: &mut PoseProgram,
        scratch: &mut Vec<PoseCommand>,
    ) -> bool {
        match self {
            Node::Tree(t) => {
                scratch.clear();
                let start = out.leaves.len();
                if t.play.evaluate(0.01, true, update_history, scratch) {
                    flatten(scratch, t.mirror, &mut out.leaves);
                }
                let end = out.leaves.len();
                if end == start {
                    return false;
                }
                out.ops.push(PoseOp::Group {
                    start: start as u16,
                    end: end as u16,
                });
                true
            }
            Node::Transition(t) => {
                if t.def.kind == 4 {
                    return if t.sequence_done {
                        t.to.program(update_history, out, scratch)
                    } else {
                        t.from.program(update_history, out, scratch)
                    };
                }
                let w = t.weight();
                let from = t.from.program(update_history, out, scratch);
                let to = t.to.program(update_history, out, scratch);
                if from && to {
                    out.ops.push(if t.def.kind == 3 {
                        PoseOp::Channel {
                            weight: w,
                            first: t.def.channels_from_weights,
                        }
                    } else {
                        PoseOp::Blend(w)
                    });
                }
                from || to
            }
        }
    }
}

/// How the rider builds this frame's pose: a small program over a stack of poses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PoseOp {
    /// Push the weighted mix of `leaves[start..end]` (weights summing to 1).
    Group { start: u16, end: u16 },
    /// Pop the newer pose and the older one; push the older blended towards the newer by the weight.
    Blend(f32),
    /// A channel blend (SK8 ACSChannelBlend): as `Blend`, but each bone by the weight times its channel
    /// weight in the older pose (`first`, `useChannelFromWeights`) or the newer one, at most 1. Clips
    /// that aren't channel animations weigh 1 everywhere, so between them it is a crossfade; a channel
    /// animation (the fakie head and torso) takes only its bones.
    Channel { weight: f32, first: bool },
}

/// The clips and the program to pose them (see [`PoseOp`]).
#[derive(Clone, Debug, Default)]
pub struct PoseProgram {
    pub leaves: Vec<Leaf>,
    pub ops: Vec<PoseOp>,
}

impl PoseProgram {
    pub fn clear(&mut self) {
        self.leaves.clear();
        self.ops.clear();
    }

    /// Each clip's share of the final pose, ignoring channel weights (for logs and tests).
    pub fn shares(&self) -> Vec<(u16, f32)> {
        let mut stack: Vec<Vec<(u16, f32)>> = Vec::new();
        for op in &self.ops {
            match *op {
                PoseOp::Group { start, end } => stack.push(
                    self.leaves[start as usize..end as usize]
                        .iter()
                        .map(|l| (l.clip, l.weight))
                        .collect(),
                ),
                PoseOp::Blend(w) | PoseOp::Channel { weight: w, .. } => {
                    let (Some(b), Some(mut a)) = (stack.pop(), stack.pop()) else {
                        return Vec::new();
                    };
                    a.iter_mut().for_each(|x| x.1 *= 1.0 - w);
                    a.extend(b.into_iter().map(|x| (x.0, x.1 * w)));
                    stack.push(a);
                }
            }
        }
        stack.pop().unwrap_or_default()
    }
}

/// Runs a tree's pose commands as weights: each clip's share of the tree's pose.
fn flatten(commands: &[PoseCommand], mirror: bool, out: &mut Vec<Leaf>) {
    // Stack of (start index into `out`) for each pushed pose.
    let base = out.len();
    let mut stack: Vec<usize> = Vec::new();
    for c in commands {
        match *c {
            PoseCommand::Clip {
                clip,
                previous,
                time,
                loops,
            } => {
                stack.push(out.len());
                out.push(Leaf {
                    clip,
                    time,
                    previous,
                    loops,
                    weight: 1.0,
                    mirror,
                });
            }
            PoseCommand::Blend { weight } => {
                let (Some(b), Some(a)) = (stack.pop(), stack.pop()) else {
                    out.truncate(base);
                    return;
                };
                for l in &mut out[a..b] {
                    l.weight *= 1.0 - weight;
                }
                for l in &mut out[b..] {
                    l.weight *= weight;
                }
                stack.push(a);
            }
            PoseCommand::WeightedBlend { weights, count } => {
                let n = count as usize;
                if stack.len() < n {
                    out.truncate(base);
                    return;
                }
                let parts = stack.split_off(stack.len() - n);
                for (k, &s) in parts.iter().enumerate() {
                    let e = parts.get(k + 1).copied().unwrap_or(out.len());
                    for l in &mut out[s..e] {
                        l.weight *= weights[k];
                    }
                }
                stack.push(parts[0]);
            }
        }
    }
    // Drop clips that ended up with no weight.
    let mut i = base;
    while i < out.len() {
        if out[i].weight <= 1e-4 {
            out.remove(i);
        } else {
            i += 1;
        }
    }
}

fn empty() -> Tree {
    Tree {
        play: Playback::Clip {
            clip: 0,
            clock: crate::motion::tree::Clock {
                frames: 1.0,
                fps: 30.0,
                base_speed: 1.0,
                speed: 1.0,
                length: 0.0,
                time: 0.0,
                previous: 0.0,
                loops: 0,
                looping: false,
                phase_controlled: false,
            },
        },
        mirror: false,
        name: super::NONE,
    }
}

/// How a persistent channel plays (SK8 ChannelSettings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelSettings {
    pub priority: i32,
    /// Stays (at full weight) after its tree ends, until ended.
    pub keep_alive: bool,
    pub speed: f32,
    pub blend_in: f32,
    /// Its tree doesn't advance while fading in.
    pub hold_in: bool,
    pub blend_out: f32,
    /// Fade out only once ended, holding the last frame.
    pub hold_out: bool,
}

/// A channel's fades (SK8 channel_playback.rs: NewChannel, AddTime, RemoveExpiredChannels,
/// EndChannelPrematurely, StartTransition).
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    pub settings: ChannelSettings,
    pub weight: f32,
    pub influence: f32,
    fade_in: f32,
    fade_out: f32,
    revive: f32,
    revive_len: f32,
    ending: bool,
    crossed_end: bool,
}

impl Fade {
    pub fn new(settings: ChannelSettings) -> Fade {
        Fade {
            settings,
            weight: 0.0,
            influence: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            revive: 0.0,
            revive_len: 0.0,
            ending: false,
            crossed_end: false,
        }
    }

    /// Faded out (or, not kept alive and without a fade-out, its tree ended).
    pub fn expired(&self) -> bool {
        if self.settings.blend_out > 0.0 {
            self.fade_out >= self.settings.blend_out
        } else {
            !self.settings.keep_alive && self.crossed_end
        }
    }

    /// Ends the channel: it fades out (or goes at once without a fade-out).
    pub fn end(&mut self) {
        if self.fade_out == 0.0 {
            self.ending = true;
            if self.settings.blend_out <= 0.0 {
                self.settings.keep_alive = false;
                self.crossed_end = true;
            }
        }
    }

    pub fn can_transition(&self, revive: bool) -> bool {
        self.fade_out <= 0.0 || revive
    }

    /// A new tree for the channel: fading out, it comes back from where it was (`revive`).
    pub fn transition(&mut self, settings: ChannelSettings, revive: bool) {
        if self.fade_out > 0.0 && revive {
            self.revive_len = settings.blend_in;
            let left = if self.settings.blend_out > 0.0 {
                1.0 - self.fade_out / self.settings.blend_out
            } else {
                0.0
            };
            self.revive = left.clamp(0.0, 1.0) * settings.blend_in;
        }
        self.fade_out = 0.0;
        self.ending = false;
        self.settings.keep_alive = settings.keep_alive;
        self.settings.blend_out = settings.blend_out;
        self.settings.hold_out = settings.hold_out;
    }

    /// Updates the weight for a step of `dt` (the fades use the times before the step). Returns
    /// whether the channel's tree advances.
    pub fn advance(&mut self, dt: f32, remaining: f32) -> bool {
        let st = self.settings;
        let mut fade_in = 1.0;
        if st.blend_in > 0.0 && self.fade_in < st.blend_in {
            fade_in = (self.fade_in / st.blend_in).clamp(0.0, 1.0);
            self.fade_in += dt;
        }
        let automatic = if st.hold_out {
            self.crossed_end
        } else {
            remaining.max(0.0) < st.blend_out
        };
        let mut fade_out = 1.0;
        if st.blend_out > 0.0 && (self.ending || (!st.keep_alive && automatic)) {
            fade_out = (1.0 - self.fade_out / st.blend_out).clamp(0.0, 1.0);
            self.fade_out += dt;
        }
        let mut revive = 1.0;
        if self.revive_len > 0.0 && self.revive < self.revive_len {
            revive = (self.revive / self.revive_len).clamp(0.0, 1.0);
            self.revive += dt;
        }
        self.weight = (fade_in.min(fade_out).min(revive) * self.influence).clamp(0.0, 1.0);
        !(st.hold_in && self.fade_in < st.blend_in) && !(st.hold_out && self.fade_out > 0.0)
    }
}

/// A persistent channel over the motion graph's animation (the fakie head and torso, ...).
pub struct Channel {
    /// Graph string id of its name.
    pub name: u16,
    pub node: Node,
    pub fade: Fade,
}

/// The motion graph's player: the current node and the advance result conditions read.
#[derive(Default)]
pub struct Player {
    pub current: Option<Node>,
    /// Channels over it, lowest priority first.
    pub channels: Vec<Channel>,
    /// The last advance (WillExpire, ExpireInTime).
    pub property: Option<Advance>,
    /// Parameters set on trees (latest value per name).
    pub params: Vec<Param>,
    /// Attribute values of the tree that was playing before the last `play` (`lastAnim` sources).
    pub last_attrs: Vec<(NameId, f32)>,
    /// Trees started this tick (graph string of the name, the parameters they read), checked at the
    /// end of the tick for parameters no behaviour has set.
    started: Vec<(u16, Vec<NameId>)>,
    /// (tree, parameter) pairs played unset, each reported once; the ones not yet taken.
    reported: Vec<NameId>,
    pub unset: Vec<(u16, NameId)>,
}

impl Player {
    pub fn set_param(&mut self, name: NameId, value: f32, normalized: bool) {
        match self.params.iter_mut().find(|p| p.name == name) {
            Some(p) => {
                p.value = value;
                p.normalized = normalized;
            }
            None => self.params.push(Param {
                name,
                value,
                normalized,
            }),
        }
    }

    /// Starts `tree` with `def`. A cut (`play`) drops what was playing.
    pub fn play(&mut self, db: &MotionDb, mut tree: Tree, speed: f32, def: TransitionDef) {
        tree.play.set_speed(speed);
        tree.play.set_parameters(db, &self.params);
        self.started_tree(&tree);
        let new = Node::tree(tree);
        let prior = if def.kind == 1 {
            None
        } else {
            self.current.take()
        };
        self.current = Some(match prior {
            Some(Node::Transition(mut t)) if def.under => {
                let to = core::mem::replace(&mut t.to, Node::tree(empty()));
                t.to = Node::Transition(Box::new(Transition::new(to, new, def)));
                Node::Transition(t)
            }
            Some(from) => Node::Transition(Box::new(Transition::new(from, new, def))),
            None => new,
        });
        self.property = None;
    }

    /// Notes a tree started this tick, for the unset-parameter check in [`Player::advance`].
    fn started_tree(&mut self, tree: &Tree) {
        let mut names = Vec::new();
        tree.play.parameters(&mut names);
        if !names.is_empty() {
            self.started.push((tree.name, names));
        }
    }

    pub fn advance(&mut self, db: &MotionDb, dt: f32) {
        // Trees started this tick, after every behaviour has run: a parameter nothing has set is a gap
        // in the behaviours (reported once per parameter).
        for (tree, names) in core::mem::take(&mut self.started) {
            for n in names {
                if !self.reported.contains(&n) && !self.params.iter().any(|p| p.name == n) {
                    self.reported.push(n);
                    self.unset.push((tree, n));
                }
            }
        }
        if let Some(c) = self.current.take() {
            let mut c = c.collapse();
            c.set_parameters(db, &self.params);
            let mut r = Advance::default();
            c.advance(dt, &mut r);
            self.property = Some(r);
            self.current = Some(c);
        }
        // Channels: finished transitions collapse, faded ones go, the rest advance over the base.
        self.channels.retain(|c| !c.fade.expired());
        for c in &mut self.channels {
            let node = core::mem::replace(&mut c.node, Node::tree(empty()));
            c.node = node.collapse();
            c.node.set_parameters(db, &self.params);
            if c.fade.advance(dt, c.node.length() - c.node.time()) {
                let mut r = Advance::default();
                c.node.advance(dt, &mut r);
                c.fade.crossed_end = r.crossed_end;
            }
        }
    }

    /// Scales a channel's weight (SK8 `influence`).
    pub fn set_channel_influence(&mut self, name: u16, influence: f32) {
        if let Some(c) = self.channels.iter_mut().find(|c| c.name == name) {
            c.fade.influence = influence.clamp(0.0, 1.0);
        }
    }

    pub fn has_channel(&self, name: u16) -> bool {
        self.channels.iter().any(|c| c.name == name)
    }

    /// The channel's weight now (0 without it).
    pub fn channel_weight(&self, name: u16) -> f32 {
        self.channels
            .iter()
            .find(|c| c.name == name)
            .map_or(0.0, |c| c.fade.weight)
    }

    /// Plays `tree` on channel `name`: a new channel when there is none (`create`), else a transition
    /// to it (`revive`: a fading channel comes back).
    #[allow(clippy::too_many_arguments)]
    pub fn transition_channel(
        &mut self,
        db: &MotionDb,
        name: u16,
        mut tree: Tree,
        settings: ChannelSettings,
        def: TransitionDef,
        revive: bool,
        create: bool,
    ) {
        tree.play.set_speed(settings.speed);
        tree.play.set_parameters(db, &self.params);
        self.started_tree(&tree);
        match self.channels.iter().position(|c| c.name == name) {
            None if create => {
                let at = self
                    .channels
                    .partition_point(|c| c.fade.settings.priority <= settings.priority);
                self.channels.insert(
                    at,
                    Channel {
                        name,
                        node: Node::tree(tree),
                        fade: Fade::new(settings),
                    },
                );
            }
            Some(i) if self.channels[i].fade.can_transition(revive) => {
                let c = &mut self.channels[i];
                let old = match core::mem::replace(&mut c.node, Node::tree(empty())) {
                    Node::Transition(t) => t.to,
                    n => n,
                };
                c.node = Node::Transition(Box::new(Transition::new(old, Node::tree(tree), def)));
                c.fade.transition(settings, revive);
            }
            _ => {}
        }
    }

    /// Seconds left of the channel's tree (0 without the channel).
    pub fn channel_remaining(&self, name: u16) -> f32 {
        self.channels
            .iter()
            .find(|c| c.name == name)
            .map_or(0.0, |c| c.node.length() - c.node.time())
    }

    /// The channel is crossfading between two trees.
    pub fn channel_in_transition(&self, name: u16) -> bool {
        self.channels
            .iter()
            .find(|c| c.name == name)
            .is_some_and(|c| c.node.in_transition())
    }

    pub fn end_channel(&mut self, name: u16) {
        if let Some(c) = self.channels.iter_mut().find(|c| c.name == name) {
            c.fade.end();
        }
    }

    /// This frame's pose: the base, then each channel with weight over it (see [`PoseOp`]).
    pub fn program(&mut self, out: &mut PoseProgram, scratch: &mut Vec<PoseCommand>) {
        out.clear();
        let Some(c) = &mut self.current else { return };
        if !c.program(true, out, scratch) {
            return;
        }
        for ch in &mut self.channels {
            if ch.fade.weight > 0.0 && ch.node.program(true, out, scratch) {
                out.ops.push(PoseOp::Channel {
                    weight: ch.fade.weight,
                    first: false,
                });
            }
        }
    }

    pub fn in_transition(&self) -> bool {
        self.current.as_ref().is_some_and(Node::in_transition)
    }

    /// Seconds into the top tree.
    pub fn time(&self) -> f32 {
        self.current.as_ref().map_or(0.0, Node::time)
    }

    pub fn length(&self) -> f32 {
        self.current.as_ref().map_or(0.0, Node::length)
    }

    pub fn attribute(&self, db: &MotionDb, name: NameId) -> Option<Attr> {
        let mut out = Attr::scalar(name, 0.0);
        self.current
            .as_ref()
            .filter(|c| c.query_attribute(db, name, 15, &mut out))
            .map(|_| out)
    }
}
