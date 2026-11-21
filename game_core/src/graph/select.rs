//! Next-state selection and the per-tick controller, as SK8-ENGINE/skate-3-rust-engine reads Skate 3's
//! (crates/skate-core/src/graph: selection.rs, activation.rs, expression.rs, controller.rs; GPL-3.0).
//!
//! Each tick: search the current state's transitions, then its parents', for an active one, urgent
//! priority first; else keep the current state if it and its parents still hold (`sustain` conditions),
//! else re-resolve (from the root, or from the interrupt ancestor) by descending through the first
//! active children in authored order. Conditions carry a mask: `precond` (1) is checked when a state is
//! entered or a transition is taken, `sustain` (2) while staying, `always` (3) both.

use alloc::vec::Vec;

use super::{Graph, LIST_OP, NONE, OpId, StateId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeId {
    State(StateId),
    Transition(u16),
}

/// What conditions and behaviours see of the controller.
#[derive(Clone, Debug)]
pub struct Frame {
    pub dt: f32,
    pub current: Option<StateId>,
    pub last: Option<StateId>,
    /// Seconds in each state on the current path (negative: not in it).
    pub state_times: Vec<f32>,
}

impl Frame {
    pub fn time_in(&self, s: StateId) -> Option<f32> {
        self.state_times
            .get(s as usize)
            .copied()
            .filter(|t| *t >= 0.0)
    }
}

/// The game side of a graph: condition values and behaviour/hook execution.
pub trait Host {
    fn condition(&mut self, op: OpId, frame: &Frame) -> bool;
    fn begin(&mut self, op: OpId, frame: &Frame);
    fn update(&mut self, op: OpId, frame: &Frame);
    fn end(&mut self, op: OpId, frame: &Frame);
    fn hook(&mut self, op: OpId, frame: &Frame);
}

/// Evaluates node activation for one mask (expression trees of conditions).
pub fn activation(g: &Graph, node: NodeId, mask: u8, frame: &Frame, host: &mut impl Host) -> bool {
    let e = match node {
        NodeId::State(s) => g.states[s as usize].expression,
        NodeId::Transition(t) => g.transitions[t as usize].expression,
    };
    if e == NONE {
        return true;
    }
    let mut excluded = true;
    let r = expression(g, e, mask, frame, &mut excluded, host);
    excluded || r
}

fn expression(
    g: &Graph,
    id: u16,
    mask: u8,
    frame: &Frame,
    excluded: &mut bool,
    host: &mut impl Host,
) -> bool {
    let e = g.expressions[id as usize];
    let children = e.children.of(&g.lists);
    let mut child = |i: usize, ex: &mut bool| -> bool {
        let c = children[i];
        if c & LIST_OP != 0 {
            let op = c & !LIST_OP;
            let o = &g.ops[op as usize];
            if !o.enabled || o.mask & mask == 0 {
                *ex = true;
                true
            } else {
                *ex = false;
                host.condition(op, frame)
            }
        } else {
            expression(g, c, mask, frame, ex, host)
        }
    };
    match e.operator {
        3 => {
            if children.is_empty() {
                return true;
            }
            let mut ex = true;
            let v = child(0, &mut ex);
            *excluded = ex;
            ex || !v
        }
        0..=2 => {
            let or = e.operator == 2;
            let mut value = !or;
            let mut all_excluded = true;
            for i in 0..children.len() {
                if value == or {
                    break;
                }
                let mut ex = true;
                let r = child(i, &mut ex);
                if !ex {
                    all_excluded = false;
                    value = if or { r } else { r && value };
                }
            }
            *excluded = all_excluded;
            all_excluded || value
        }
        _ => true,
    }
}

struct Search<'g, 'h, H: Host> {
    g: &'g Graph,
    current: Option<StateId>,
    frame: &'g Frame,
    host: &'h mut H,
}

impl<H: Host> Search<'_, '_, H> {
    fn active(&mut self, node: NodeId, mask: u8) -> bool {
        activation(self.g, node, mask, self.frame, self.host)
    }

    fn next(&mut self, target: &mut Option<StateId>) -> Option<u16> {
        let root = self.g.root;
        let Some(current) = self.current else {
            *target = self.descend(root, 1);
            return None;
        };
        for priority in (0..=3).rev() {
            if let Some((t, s)) = self.search(current, priority) {
                *target = Some(s);
                return Some(t);
            }
        }
        let st = self.g.states[current as usize];
        match st.interruptable {
            0 if self.ancestors_active(current, 2) => *target = Some(current),
            0 | 1 => *target = self.descend(root, 2),
            _ => {
                let ancestor = st.interrupt_ancestor;
                if self.ancestors_active(ancestor, 2) {
                    *target = self.descend(ancestor, 2);
                    if target.is_some() {
                        return None;
                    }
                }
                *target = self.descend(root, 2);
            }
        }
        None
    }

    fn descend(&mut self, state: StateId, mask: u8) -> Option<StateId> {
        let m = if mask == 2 && self.g.is_within(self.current, state) {
            2
        } else {
            1
        };
        if !self.active(NodeId::State(state), m) {
            return None;
        }
        self.first_leaf(state, mask)
    }

    fn first_leaf(&mut self, state: StateId, mask: u8) -> Option<StateId> {
        let g = self.g;
        let children = g.states[state as usize].children.of(&g.lists);
        if children.is_empty() {
            return Some(state);
        }
        for &c in children {
            let n = g.states[c as usize];
            if n.enabled
                && n.active
                && let Some(leaf) = self.descend(c, mask)
            {
                return Some(leaf);
            }
        }
        None
    }

    fn search(&mut self, mut state: StateId, priority: u8) -> Option<(u16, StateId)> {
        let g = self.g;
        loop {
            for &id in g.states[state as usize].transitions.of(&g.lists) {
                let t = g.transitions[id as usize];
                if t.priority != priority || !t.enabled || !g.states[t.target as usize].enabled {
                    continue;
                }
                if !self.active(NodeId::Transition(id), 1)
                    || !self.valid_target(t.target)
                    || !self.active(NodeId::State(t.target), 1)
                {
                    continue;
                }
                if let Some(leaf) = self.first_leaf(t.target, 1) {
                    return Some((id, leaf));
                }
            }
            state = g.parent(state)?;
        }
    }

    fn valid_target(&mut self, target: StateId) -> bool {
        let mut next = Some(target);
        while let Some(s) = next {
            if s != target && self.g.is_within(self.current, s) {
                return self.ancestors_active(s, 2);
            }
            if !self.active(NodeId::State(s), 1) {
                return false;
            }
            next = self.g.parent(s);
        }
        true
    }

    fn ancestors_active(&mut self, mut s: StateId, mask: u8) -> bool {
        loop {
            if !self.active(NodeId::State(s), mask) {
                return false;
            }
            match self.g.parent(s) {
                Some(p) => s = p,
                None => return true,
            }
        }
    }
}

/// The next state (and the transition taken, if any) from `frame.current`.
pub fn next_state(
    g: &Graph,
    frame: &Frame,
    host: &mut impl Host,
) -> (Option<u16>, Option<StateId>) {
    let mut target = frame.current;
    let mut s = Search {
        g,
        current: frame.current,
        frame,
        host,
    };
    let t = s.next(&mut target);
    (t, target)
}

/// Runs one graph: the current state, its clocks and its active behaviours.
#[derive(Clone, Debug)]
pub struct Controller {
    pub frame: Frame,
    /// Behaviour operations that have begun and not ended, in begin order.
    pub active: Vec<OpId>,
    /// The transition taken on the last update, if any.
    pub last_transition: Option<u16>,
    path_old: Vec<StateId>,
    path_new: Vec<StateId>,
}

fn path(g: &Graph, mut s: Option<StateId>, out: &mut Vec<StateId>) {
    out.clear();
    while let Some(id) = s {
        out.push(id);
        s = g.parent(id);
    }
    out.reverse();
}

impl Controller {
    pub fn new(g: &Graph) -> Controller {
        Controller {
            frame: Frame {
                dt: 0.0,
                current: None,
                last: None,
                state_times: alloc::vec![-1.0; g.states.len()],
            },
            active: Vec::new(),
            last_transition: None,
            path_old: Vec::new(),
            path_new: Vec::new(),
        }
    }

    pub fn update(&mut self, g: &Graph, dt: f32, host: &mut impl Host) {
        self.frame.dt = dt;
        let (transition, next) = next_state(g, &self.frame, host);
        self.last_transition = transition;
        // The boundary: a transition to a state on the current path re-enters that state.
        path(g, self.frame.current, &mut self.path_old);
        let boundary = match transition {
            None => next,
            Some(id) => {
                let target = g.transitions[id as usize].target;
                if self.path_old.contains(&target) {
                    g.parent(target)
                } else {
                    Some(target)
                }
            }
        };
        path(g, boundary, &mut self.path_new);
        let common = self
            .path_old
            .iter()
            .zip(&self.path_new)
            .take_while(|(a, b)| a == b)
            .count();
        path(g, next, &mut self.path_new);
        self.frame.last = self.frame.current;
        self.frame.current = next;
        for t in self.frame.state_times.iter_mut() {
            if *t >= 0.0 {
                *t += dt;
            }
        }
        // Exit: behaviours owned by leaving states end, newest first.
        let exiting = &self.path_old[common.min(self.path_old.len())..];
        let mut i = self.active.len();
        while i > 0 {
            i -= 1;
            let op = self.active[i];
            if exiting.contains(&g.ops[op as usize].owner) {
                host.end(op, &self.frame);
                self.active.remove(i);
            }
        }
        for &s in exiting {
            self.frame.state_times[s as usize] = -1.0;
        }
        if let Some(id) = transition {
            for &h in g.transitions[id as usize].hooks.of(&g.lists) {
                host.hook(h, &self.frame);
            }
        }
        let entering = common.min(self.path_new.len());
        for k in entering..self.path_new.len() {
            self.frame.state_times[self.path_new[k] as usize] = 0.0;
        }
        for k in entering..self.path_new.len() {
            let s = self.path_new[k];
            for &op in g.states[s as usize].behaviours.of(&g.lists) {
                if g.ops[op as usize].enabled {
                    self.active.push(op);
                    host.begin(op, &self.frame);
                }
            }
        }
        for k in 0..self.active.len() {
            let op = self.active[k];
            host.update(op, &self.frame);
        }
    }

    pub fn end_all(&mut self, host: &mut impl Host) {
        while let Some(op) = self.active.pop() {
            host.end(op, &self.frame);
        }
    }
}
