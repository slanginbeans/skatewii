//! Skate 3's state graphs (`ActionGraph_OnBoard`, `MotionGraph_OnBoard`), run the way the game runs them.
//!
//! `fg_import motion` compiles both graphs from the player's disc into one `motion/graph` entry of
//! `motion.fgpak` (kind `StateGraph`); nothing of them is in the repo. Without the entry the game keeps
//! its own hand-written riding logic.
//!
//! The runtime model follows SK8-ENGINE/skate-3-rust-engine (GPL-3.0): crates/skate-data state_graph
//! binding (how elements become states, transitions, expressions and operations), crates/skate-core
//! graph/selection.rs, activation.rs, expression.rs and controller.rs (how the next state is chosen each
//! tick and how behaviours begin, update and end). See `docs/graph-runner.md`.
//!
//! - [`Graph`]: one compiled graph (states, transitions, expressions, operations and their attributes).
//! - [`select`]: transition search and re-selection (priorities, interruptability, condition masks).
//! - [`Controller`]: the per-tick driver (exit/enter lists, behaviour lifecycle, state clocks).
//! - [`write`]: the compiler from graph elements (used by the importer and by tests).
//!
//! Entry `motion/graph` (big-endian):
//! ```text
//! 0   "FGSG", u32 version (2), u32 graph count, u32 string count, u32 string offsets at, u32 blob at
//! 24  per graph, 64 bytes: u32 name (string), u32 root state, then (u32 offset, u32 count) for
//!     states, transitions, expressions, operations, parameters, attributes, lists
//!     state 24: u16 name, u16 parent, u16 interrupt ancestor, u16 expression, u8 flags (1 enabled,
//!               2 active), u8 interruptable (0 false, 1 true, 2 ancestor), (u16 start, u16 count) for
//!               children, transitions and behaviours (into lists), u16 0
//!     transition 12: u16 owner, u16 target, u16 expression, u8 enabled, u8 priority (0 default, 1 med,
//!               2 high, 3 urgent), u16 hooks start, u16 hooks count
//!     expression 8: u8 operator (0 none, 1 and, 2 or, 3 not), u8 enabled, u16 0, u16 children start,
//!               u16 count (list entries: bit 15 set = operation, else expression)
//!     operation 24: u8 kind (0 behaviour, 1 condition, 2 hook), u8 enabled, u8 mask (1 precond,
//!               2 sustain, 3 always, 4 postcond), u8 0, u16 name, u16 owner state, u16 bound state
//!               (from `state`, or the owner for InParentStateForTime), u16 0, u32 first attribute,
//!               u16 attribute count, u16 first parameter, u16 parameter count, u16 0
//!     parameter 8: u32 first attribute, u16 count, u16 0
//!     attribute 12: u16 key, u16 text, u32 float word, u8 bool byte, u8[3] 0
//!     lists: u16 each
//! ... string offsets (u32 each into the blob, in id order), the ids sorted by their strings' bytes (u16
//!     each, padded to 4), blob of NUL-terminated strings (in id order)
//! ```
//! Ids are u16; `NONE` (0xFFFF) is "none".

pub mod drive;
pub mod host;
pub mod input;
pub mod opcode;
pub mod ops;
pub mod playback;
pub mod push;
pub mod select;
#[cfg(test)]
mod tests;
pub mod write;

use alloc::vec::Vec;

pub use select::{Controller, Frame, Host, NodeId};

pub const MAGIC: [u8; 4] = *b"FGSG";
pub const VERSION: u32 = 2;
pub const NONE: u16 = 0xFFFF;
pub const HEADER: usize = 24;
pub const GRAPH_HEADER: usize = 64;
pub const STATE_SIZE: usize = 24;
pub const TRANSITION_SIZE: usize = 12;
pub const EXPRESSION_SIZE: usize = 8;
pub const OP_SIZE: usize = 24;
pub const PARAM_SIZE: usize = 8;
pub const ATTR_SIZE: usize = 12;
/// List entry bit: an expression child that is an operation (condition).
pub const LIST_OP: u16 = 0x8000;

pub const OP_BEHAVIOUR: u8 = 0;
pub const OP_CONDITION: u8 = 1;
pub const OP_HOOK: u8 = 2;

pub type StateId = u16;
pub type OpId = u16;
pub type Str = u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    BadMagic,
    BadVersion(u32),
    Truncated,
    /// An index points outside its table.
    Bad,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Range {
    pub start: u16,
    pub count: u16,
}

impl Range {
    pub fn of<'a>(&self, lists: &'a [u16]) -> &'a [u16] {
        let s = self.start as usize;
        &lists[s..s + self.count as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    pub name: Str,
    pub parent: StateId,
    pub interrupt_ancestor: StateId,
    pub expression: u16,
    pub enabled: bool,
    pub active: bool,
    /// 0: not interruptable, 1: interruptable, 2: re-resolve under `interrupt_ancestor`.
    pub interruptable: u8,
    pub children: Range,
    pub transitions: Range,
    pub behaviours: Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub owner: StateId,
    pub target: StateId,
    pub expression: u16,
    pub enabled: bool,
    pub priority: u8,
    pub hooks: Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expression {
    pub operator: u8,
    pub children: Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Op {
    pub kind: u8,
    pub enabled: bool,
    pub mask: u8,
    pub name: Str,
    pub owner: StateId,
    pub bound: StateId,
    pub attrs: (u32, u16),
    pub params: Range,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attr {
    pub key: Str,
    pub text: Str,
    pub float: f32,
    pub flag: bool,
}

/// One compiled graph.
#[derive(Debug, Clone)]
pub struct Graph {
    pub name: Str,
    pub root: StateId,
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
    pub expressions: Vec<Expression>,
    pub ops: Vec<Op>,
    /// Parameter elements of operations: (first attribute, count).
    pub params: Vec<(u32, u16)>,
    pub attrs: Vec<Attr>,
    pub lists: Vec<u16>,
}

/// Both graphs and their shared string table.
pub struct Pack<'a> {
    pub graphs: Vec<Graph>,
    offsets: &'a [u8],
    /// Ids in byte order of their strings.
    sorted: &'a [u8],
    blob: &'a [u8],
    count: usize,
}

fn be32(b: &[u8], at: usize) -> Result<u32, Error> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(Error::Truncated)
}

fn be16(b: &[u8], at: usize) -> Result<u16, Error> {
    b.get(at..at + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
        .ok_or(Error::Truncated)
}

fn range(b: &[u8], at: usize) -> Result<Range, Error> {
    Ok(Range {
        start: be16(b, at)?,
        count: be16(b, at + 2)?,
    })
}

fn table<T>(
    b: &[u8],
    header: usize,
    index: usize,
    size: usize,
    read: impl Fn(&[u8], usize) -> Result<T, Error>,
) -> Result<Vec<T>, Error> {
    let at = be32(b, header + 8 + index * 8)? as usize;
    let count = be32(b, header + 12 + index * 8)? as usize;
    let end = count
        .checked_mul(size)
        .and_then(|l| l.checked_add(at))
        .ok_or(Error::Truncated)?;
    if end > b.len() {
        return Err(Error::Truncated);
    }
    (0..count).map(|i| read(b, at + i * size)).collect()
}

impl Graph {
    fn parse(b: &[u8], h: usize) -> Result<Graph, Error> {
        let flag = |b: &[u8], at: usize| b.get(at).copied().ok_or(Error::Truncated);
        let states = table(b, h, 0, STATE_SIZE, |b, a| {
            let flags = flag(b, a + 8)?;
            Ok(State {
                name: be16(b, a)?,
                parent: be16(b, a + 2)?,
                interrupt_ancestor: be16(b, a + 4)?,
                expression: be16(b, a + 6)?,
                enabled: flags & 1 != 0,
                active: flags & 2 != 0,
                interruptable: flag(b, a + 9)?,
                children: range(b, a + 10)?,
                transitions: range(b, a + 14)?,
                behaviours: range(b, a + 18)?,
            })
        })?;
        let transitions = table(b, h, 1, TRANSITION_SIZE, |b, a| {
            Ok(Transition {
                owner: be16(b, a)?,
                target: be16(b, a + 2)?,
                expression: be16(b, a + 4)?,
                enabled: flag(b, a + 6)? != 0,
                priority: flag(b, a + 7)?,
                hooks: range(b, a + 8)?,
            })
        })?;
        let expressions = table(b, h, 2, EXPRESSION_SIZE, |b, a| {
            Ok(Expression {
                operator: flag(b, a)?,
                children: range(b, a + 4)?,
            })
        })?;
        let ops = table(b, h, 3, OP_SIZE, |b, a| {
            Ok(Op {
                kind: flag(b, a)?,
                enabled: flag(b, a + 1)? != 0,
                mask: flag(b, a + 2)?,
                name: be16(b, a + 4)?,
                owner: be16(b, a + 6)?,
                bound: be16(b, a + 8)?,
                attrs: (be32(b, a + 12)?, be16(b, a + 16)?),
                params: range(b, a + 18)?,
            })
        })?;
        let params = table(b, h, 4, PARAM_SIZE, |b, a| {
            Ok((be32(b, a)?, be16(b, a + 4)?))
        })?;
        let attrs = table(b, h, 5, ATTR_SIZE, |b, a| {
            Ok(Attr {
                key: be16(b, a)?,
                text: be16(b, a + 2)?,
                float: f32::from_bits(be32(b, a + 4)?),
                flag: flag(b, a + 8)? != 0,
            })
        })?;
        let lists = table(b, h, 6, 2, be16)?;
        let g = Graph {
            name: be32(b, h)? as u16,
            root: be32(b, h + 4)? as u16,
            states,
            transitions,
            expressions,
            ops,
            params,
            attrs,
            lists,
        };
        g.validate()?;
        Ok(g)
    }

    /// Every index in range, so the runner can index without checks.
    fn validate(&self) -> Result<(), Error> {
        let ns = self.states.len();
        let state_ok = |s: u16| s == NONE || (s as usize) < ns;
        let list_ok = |r: Range| r.start as usize + r.count as usize <= self.lists.len();
        let expr_ok = |e: u16| e == NONE || (e as usize) < self.expressions.len();
        let ok = (self.root as usize) < ns
            && self.states.iter().all(|s| {
                state_ok(s.parent)
                    && state_ok(s.interrupt_ancestor)
                    && expr_ok(s.expression)
                    && list_ok(s.children)
                    && list_ok(s.transitions)
                    && list_ok(s.behaviours)
                    && s.children
                        .of(&self.lists)
                        .iter()
                        .all(|&c| (c as usize) < ns)
                    && s.transitions
                        .of(&self.lists)
                        .iter()
                        .all(|&t| (t as usize) < self.transitions.len())
                    && s.behaviours
                        .of(&self.lists)
                        .iter()
                        .all(|&o| (o as usize) < self.ops.len())
                    && (s.interruptable != 2 || s.interrupt_ancestor != NONE)
            })
            && self.transitions.iter().all(|t| {
                (t.owner as usize) < ns
                    && (t.target as usize) < ns
                    && expr_ok(t.expression)
                    && list_ok(t.hooks)
                    && t.hooks
                        .of(&self.lists)
                        .iter()
                        .all(|&o| (o as usize) < self.ops.len())
            })
            && self.expressions.iter().all(|e| {
                list_ok(e.children)
                    && e.children.of(&self.lists).iter().all(|&c| {
                        if c & LIST_OP != 0 {
                            ((c & !LIST_OP) as usize) < self.ops.len()
                        } else {
                            (c as usize) < self.expressions.len()
                        }
                    })
            })
            && self.ops.iter().all(|o| {
                state_ok(o.owner)
                    && state_ok(o.bound)
                    && o.attrs.0 as usize + o.attrs.1 as usize <= self.attrs.len()
                    && o.params.start as usize + o.params.count as usize <= self.params.len()
            })
            && self
                .params
                .iter()
                .all(|p| p.0 as usize + p.1 as usize <= self.attrs.len());
        if ok { Ok(()) } else { Err(Error::Bad) }
    }

    pub fn op_attrs(&self, op: OpId) -> &[Attr] {
        let (s, c) = self.ops[op as usize].attrs;
        &self.attrs[s as usize..s as usize + c as usize]
    }

    pub fn op_params(&self, op: OpId) -> impl Iterator<Item = &[Attr]> + '_ {
        let r = self.ops[op as usize].params;
        (r.start..r.start + r.count).map(move |p| {
            let (s, c) = self.params[p as usize];
            &self.attrs[s as usize..s as usize + c as usize]
        })
    }

    pub fn parent(&self, s: StateId) -> Option<StateId> {
        let p = self.states[s as usize].parent;
        (p != NONE).then_some(p)
    }

    /// Whether `ancestor` is `state` or one of its parents.
    pub fn is_within(&self, mut state: Option<StateId>, ancestor: StateId) -> bool {
        while let Some(s) = state {
            if s == ancestor {
                return true;
            }
            state = self.parent(s);
        }
        false
    }

    /// Bytes the parsed graph takes in memory (for the log).
    pub fn memory(&self) -> usize {
        use core::mem::size_of;
        self.states.len() * size_of::<State>()
            + self.transitions.len() * size_of::<Transition>()
            + self.expressions.len() * size_of::<Expression>()
            + self.ops.len() * size_of::<Op>()
            + self.params.len() * size_of::<(u32, u16)>()
            + self.attrs.len() * size_of::<Attr>()
            + self.lists.len() * 2
    }
}

impl<'a> Pack<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Pack<'a>, Error> {
        if b.get(..4) != Some(&MAGIC[..]) {
            return Err(Error::BadMagic);
        }
        let version = be32(b, 4)?;
        if version != VERSION {
            return Err(Error::BadVersion(version));
        }
        let graphs = be32(b, 8)? as usize;
        let count = be32(b, 12)? as usize;
        let offsets_at = be32(b, 16)? as usize;
        let blob_at = be32(b, 20)? as usize;
        let offsets = b
            .get(offsets_at..offsets_at + count * 4)
            .ok_or(Error::Truncated)?;
        let sorted = b
            .get(offsets_at + count * 4..offsets_at + count * 6)
            .ok_or(Error::Truncated)?;
        let blob = b.get(blob_at..).ok_or(Error::Truncated)?;
        let graphs = (0..graphs.min(16))
            .map(|g| Graph::parse(b, HEADER + g * GRAPH_HEADER))
            .collect::<Result<Vec<_>, _>>()?;
        let p = Pack {
            graphs,
            offsets,
            sorted,
            blob,
            count,
        };
        // The strings lie in id order, each ending where the next starts.
        let mut prev = 0;
        for i in 0..count {
            let at = be32(offsets, i * 4)? as usize;
            let end = if i + 1 < count {
                be32(offsets, i * 4 + 4)? as usize
            } else {
                blob.len()
            };
            if at < prev || at >= end || end > blob.len() || blob[end - 1] != 0 && i + 1 < count {
                return Err(Error::Bad);
            }
            if be16(sorted, i * 2)? as usize >= count {
                return Err(Error::Bad);
            }
            prev = at;
        }
        let strings_ok = p.graphs.iter().all(|g| {
            let s = |i: u16| (i as usize) < count;
            s(g.name)
                && g.states.iter().all(|x| s(x.name))
                && g.ops.iter().all(|o| s(o.name))
                && g.attrs.iter().all(|a| s(a.key) && s(a.text))
        });
        if !strings_ok {
            return Err(Error::Bad);
        }
        Ok(p)
    }

    pub fn strings(&self) -> usize {
        self.count
    }

    pub fn str(&self, id: Str) -> &'a str {
        let i = id as usize;
        let off = |k: usize| {
            u32::from_be_bytes([
                self.offsets[k * 4],
                self.offsets[k * 4 + 1],
                self.offsets[k * 4 + 2],
                self.offsets[k * 4 + 3],
            ]) as usize
        };
        let at = off(i);
        let bytes = if i + 1 < self.count {
            &self.blob[at..off(i + 1) - 1]
        } else {
            let rest = &self.blob[at..];
            &rest[..rest.iter().position(|&c| c == 0).unwrap_or(rest.len())]
        };
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// The string's id, if any element uses it (binary search over the sorted ids).
    pub fn find(&self, s: &str) -> Option<Str> {
        let id = |k: usize| u16::from_be_bytes([self.sorted[k * 2], self.sorted[k * 2 + 1]]);
        let (mut lo, mut hi) = (0usize, self.count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.str(id(mid)).as_bytes().cmp(s.as_bytes()) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return Some(id(mid)),
            }
        }
        None
    }

    pub fn graph(&self, name: &str) -> Option<&Graph> {
        self.graphs.iter().find(|g| self.str(g.name) == name)
    }

    pub fn state_name(&self, g: &Graph, s: StateId) -> &'a str {
        self.str(g.states[s as usize].name)
    }

    /// `Parent/Child/Leaf` names of a state, for logs.
    pub fn state_path(&self, g: &Graph, s: Option<StateId>) -> alloc::string::String {
        let mut parts = Vec::new();
        let mut c = s;
        while let Some(id) = c {
            parts.push(self.state_name(g, id));
            c = g.parent(id);
        }
        parts.reverse();
        // Skip the root's name: every path starts there.
        parts.iter().skip(1).copied().collect::<Vec<_>>().join("/")
    }

    /// The attribute named `key` of a list (first one wins, as in the game).
    pub fn attr<'g>(&self, attrs: &'g [Attr], key: &str) -> Option<&'g Attr> {
        attrs.iter().find(|a| self.str(a.key) == key)
    }

    pub fn text(&self, attrs: &[Attr], key: &str) -> Option<&'a str> {
        self.attr(attrs, key).map(|a| self.str(a.text))
    }

    pub fn float(&self, attrs: &[Attr], key: &str, default: f32) -> f32 {
        self.attr(attrs, key).map_or(default, |a| a.float)
    }

    pub fn flag(&self, attrs: &[Attr], key: &str, default: bool) -> bool {
        self.attr(attrs, key).map_or(default, |a| a.flag)
    }
}
