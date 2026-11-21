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

    /// The clips to blend this frame, with weights summing to 1. `update_history` commits the clocks
    /// (start the next step's event window).
    pub fn leaves(
        &mut self,
        update_history: bool,
        out: &mut Vec<Leaf>,
        scratch: &mut Vec<PoseCommand>,
    ) {
        match self {
            Node::Tree(t) => {
                scratch.clear();
                if t.play.evaluate(0.01, true, update_history, scratch) {
                    flatten(scratch, t.mirror, out);
                }
            }
            Node::Transition(t) => {
                if t.def.kind == 4 {
                    if t.sequence_done {
                        t.to.leaves(update_history, out, scratch)
                    } else {
                        t.from.leaves(update_history, out, scratch)
                    }
                    return;
                }
                let w = t.weight();
                let start = out.len();
                t.from.leaves(update_history, out, scratch);
                let mid = out.len();
                t.to.leaves(update_history, out, scratch);
                if mid == start || mid == out.len() {
                    return;
                }
                for l in &mut out[start..mid] {
                    l.weight *= 1.0 - w;
                }
                for l in &mut out[mid..] {
                    l.weight *= w;
                }
            }
        }
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

/// The motion graph's player: the current node and the advance result conditions read.
#[derive(Default)]
pub struct Player {
    pub current: Option<Node>,
    /// The last advance (WillExpire, ExpireInTime).
    pub property: Option<Advance>,
    /// Parameters set on trees (latest value per name).
    pub params: Vec<Param>,
    /// Attribute values of the tree that was playing before the last `play` (`lastAnim` sources).
    pub last_attrs: Vec<(NameId, f32)>,
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

    pub fn advance(&mut self, db: &MotionDb, dt: f32) {
        if let Some(c) = self.current.take() {
            let mut c = c.collapse();
            c.set_parameters(db, &self.params);
            let mut r = Advance::default();
            c.advance(dt, &mut r);
            self.property = Some(r);
            self.current = Some(c);
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
