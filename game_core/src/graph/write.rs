//! Compiles graph elements into a `motion/graph` entry (layout in the parent module). Used by
//! `fg_import motion` and by tests, so the writer and the reader can't drift apart.
//!
//! The binding rules (which element owns which, `enabled` inheritance, transition target and
//! interrupt-ancestor name lookup) follow SK8-ENGINE/skate-3-rust-engine
//! crates/skate-data/src/state_graph/binding.rs (GPL-3.0).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{LIST_OP, MAGIC, NONE, OP_BEHAVIOUR, OP_CONDITION, OP_HOOK, VERSION};

/// One attribute as stored on the disc: name, text, the float word and the boolean byte.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttrIn {
    pub name: String,
    pub text: String,
    pub float: u32,
    pub flag: u8,
}

impl AttrIn {
    /// An attribute the way the game's tools write one: float and bool parsed from the text.
    pub fn new(name: &str, text: &str) -> AttrIn {
        let float = text.parse::<f32>().unwrap_or(0.0);
        AttrIn {
            name: name.into(),
            text: text.into(),
            float: float.to_bits(),
            flag: u8::from(text == "true" || text == "1" || float != 0.0 && text != "false"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<AttrIn>,
    pub children: Vec<Element>,
}

impl Element {
    /// Test helper: `Element::of("state", &[("name", "A")], vec![...])`.
    pub fn of(tag: &str, attrs: &[(&str, &str)], children: Vec<Element>) -> Element {
        Element {
            tag: tag.into(),
            attrs: attrs.iter().map(|(n, t)| AttrIn::new(n, t)).collect(),
            children,
        }
    }

    fn attr(&self, name: &str) -> Option<&AttrIn> {
        self.attrs.iter().find(|a| a.name == name)
    }

    fn text(&self, name: &str) -> Option<&str> {
        self.attr(name).map(|a| a.text.as_str())
    }

    fn flag(&self, name: &str, default: bool) -> bool {
        self.attr(name).map_or(default, |a| a.flag != 0)
    }
}

#[derive(Default)]
struct Strings {
    ids: BTreeMap<String, u16>,
    list: Vec<String>,
}

impl Strings {
    fn id(&mut self, s: &str) -> Result<u16, String> {
        if let Some(&i) = self.ids.get(s) {
            return Ok(i);
        }
        let i = u16::try_from(self.list.len())
            .ok()
            .filter(|&i| i != NONE)
            .ok_or("too many strings")?;
        self.ids.insert(s.into(), i);
        self.list.push(s.into());
        Ok(i)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Node {
    State(usize),
    Transition(usize),
    Expression(usize),
    Op(usize),
}

struct StateB {
    name: String,
    parent: Option<usize>,
    children: Vec<usize>,
    behaviours: Vec<usize>,
    transitions: Vec<usize>,
    expression: Option<usize>,
    enabled: bool,
    active: bool,
    interruptable: u8,
    interrupt_name: String,
    interrupt: Option<usize>,
}

struct TransitionB {
    owner: usize,
    target_name: String,
    target: Option<usize>,
    enabled: bool,
    priority: u8,
    expression: Option<usize>,
    hooks: Vec<usize>,
}

struct ExprB {
    operator: u8,
    children: Vec<Node>,
}

struct OpB<'e> {
    kind: u8,
    enabled: bool,
    mask: u8,
    name: String,
    owner: usize,
    element: &'e Element,
    params: Vec<&'e Element>,
}

#[derive(Default)]
struct Binding<'e> {
    states: Vec<StateB>,
    transitions: Vec<TransitionB>,
    exprs: Vec<ExprB>,
    ops: Vec<OpB<'e>>,
}

impl<'e> Binding<'e> {
    fn enabled(&self, n: Node) -> bool {
        match n {
            Node::State(i) => self.states[i].enabled,
            Node::Transition(i) => self.transitions[i].enabled,
            Node::Expression(_) => true,
            Node::Op(i) => self.ops[i].enabled,
        }
    }

    /// The state that owns a node (itself for a state, the owner for a transition).
    fn owner(&self, n: Node, expr_owner: &[Node]) -> usize {
        match n {
            Node::State(s) => s,
            Node::Transition(t) => self.transitions[t].owner,
            Node::Expression(e) => self.owner(expr_owner[e], expr_owner),
            Node::Op(o) => self.ops[o].owner,
        }
    }

    fn add(
        &mut self,
        e: &'e Element,
        parent: Option<Node>,
        expr_owner: &mut Vec<Node>,
    ) -> Result<(), String> {
        let enabled = if parent.is_some_and(|p| !self.enabled(p)) {
            false
        } else {
            e.flag("enabled", true)
        };
        let name = e.text("name").unwrap_or("__unknown__");
        let node = match e.tag.as_str() {
            "state" => {
                let parent = match parent {
                    None if self.states.is_empty() => None,
                    Some(Node::State(p)) => Some(p),
                    _ => return Err(format!("state {name} needs a state parent")),
                };
                let it = e.text("interruptable").unwrap_or("true");
                let id = self.states.len();
                self.states.push(StateB {
                    name: name.into(),
                    parent,
                    children: Vec::new(),
                    behaviours: Vec::new(),
                    transitions: Vec::new(),
                    expression: None,
                    enabled,
                    active: e.flag("active", true),
                    interruptable: match it {
                        "true" => 1,
                        "false" => 0,
                        _ => 2,
                    },
                    interrupt_name: it.into(),
                    interrupt: None,
                });
                if let Some(p) = parent {
                    self.states[p].children.push(id);
                }
                Node::State(id)
            }
            "transition" => {
                let Some(Node::State(owner)) = parent else {
                    return Err("transition needs a state parent".into());
                };
                let id = self.transitions.len();
                self.transitions.push(TransitionB {
                    owner,
                    target_name: e.text("target").unwrap_or("").into(),
                    target: None,
                    enabled,
                    priority: match e.text("priority") {
                        Some("med") => 1,
                        Some("high") => 2,
                        Some("urgent") => 3,
                        _ => 0,
                    },
                    expression: None,
                    hooks: Vec::new(),
                });
                self.states[owner].transitions.push(id);
                Node::Transition(id)
            }
            "expression" => {
                let id = self.exprs.len();
                self.exprs.push(ExprB {
                    operator: match e.text("op") {
                        Some("and") => 1,
                        Some("or") => 2,
                        Some("not") => 3,
                        _ => 0,
                    },
                    children: Vec::new(),
                });
                match parent {
                    Some(Node::State(o)) => self.states[o].expression = Some(id),
                    Some(Node::Transition(o)) => self.transitions[o].expression = Some(id),
                    Some(Node::Expression(o)) => self.exprs[o].children.push(Node::Expression(id)),
                    _ => return Err("expression needs a state, transition or expression".into()),
                }
                expr_owner.push(parent.unwrap());
                Node::Expression(id)
            }
            "param" => {
                let Some(Node::Op(o)) = parent else {
                    return Err("param outside an operation".into());
                };
                self.ops[o].params.push(e);
                return Ok(());
            }
            "behaviour" | "condition" | "hook" => {
                let kind = match e.tag.as_str() {
                    "behaviour" => OP_BEHAVIOUR,
                    "condition" => OP_CONDITION,
                    _ => OP_HOOK,
                };
                let parent = parent.ok_or("operation without a parent")?;
                let id = self.ops.len();
                match (kind, parent) {
                    (OP_BEHAVIOUR, Node::State(o)) => self.states[o].behaviours.push(id),
                    (OP_CONDITION, Node::Expression(o)) => {
                        self.exprs[o].children.push(Node::Op(id))
                    }
                    (OP_HOOK, Node::Transition(o)) => self.transitions[o].hooks.push(id),
                    _ => return Err(format!("{} {name} has the wrong parent", e.tag)),
                }
                let owner = self.owner(parent, expr_owner);
                self.ops.push(OpB {
                    kind,
                    enabled,
                    mask: match e.text("mask") {
                        Some("sustain") => 2,
                        Some("always") => 3,
                        Some("postcond") => 4,
                        _ => 1,
                    },
                    name: name.into(),
                    owner,
                    element: e,
                    params: Vec::new(),
                });
                Node::Op(id)
            }
            other => return Err(format!("unknown graph element `{other}`")),
        };
        for c in &e.children {
            self.add(c, Some(node), expr_owner)?;
        }
        Ok(())
    }

    /// SK8's `find_state`: children of `start` first, then (ascending) `start` itself, then upwards.
    /// `A.B` finds `A`, then `B` among its children.
    fn find_state(&self, start: usize, name: &str, ascend: bool) -> Option<usize> {
        if let Some((first, rest)) = name.split_once('.') {
            let found = self.find_state(start, first, true)?;
            return self.find_state(found, rest, false);
        }
        let mut state = start;
        loop {
            for &c in &self.states[state].children {
                if self.states[c].name == name {
                    return Some(c);
                }
            }
            if !ascend {
                return None;
            }
            if self.states[state].name == name {
                return Some(state);
            }
            state = self.states[state].parent?;
        }
    }

    fn resolve(&mut self) -> Result<(), String> {
        for id in 0..self.states.len() {
            if self.states[id].interruptable == 2 {
                let name = self.states[id].interrupt_name.clone();
                let target = self
                    .find_state(id, &name, true)
                    .ok_or_else(|| format!("unresolved interrupt ancestor {name}"))?;
                let mut a = Some(id);
                while a.is_some() && a != Some(target) {
                    a = self.states[a.unwrap()].parent;
                }
                if a.is_none() {
                    return Err(format!("interrupt target {name} is not an ancestor"));
                }
                self.states[id].interrupt = Some(target);
            }
        }
        for id in 0..self.transitions.len() {
            let t = &self.transitions[id];
            let target = self.find_state(t.owner, &t.target_name, true);
            self.transitions[id].target = target;
            if target.is_none_or(|i| !self.states[i].enabled) {
                self.transitions[id].enabled = false;
            }
        }
        Ok(())
    }
}

fn u16_of(n: usize, what: &str) -> Result<u16, String> {
    u16::try_from(n)
        .ok()
        .filter(|&v| v != NONE)
        .ok_or_else(|| format!("too many {what}"))
}

fn opt(n: Option<usize>) -> u16 {
    n.map_or(NONE, |v| v as u16)
}

/// Compiles `(name, root element)` graphs into one entry.
pub fn compile(graphs: &[(&str, &Element)]) -> Result<Vec<u8>, String> {
    let mut strings = Strings::default();
    let mut bodies = Vec::new();
    for (name, root) in graphs {
        let mut b = Binding::default();
        let mut expr_owner = Vec::new();
        if root.tag != "state" {
            return Err(format!("graph {name}: the root is not a state"));
        }
        b.add(root, None, &mut expr_owner)?;
        b.resolve()?;
        u16_of(b.states.len(), "states")?;
        u16_of(b.transitions.len(), "transitions")?;
        u16_of(b.exprs.len(), "expressions")?;
        u16_of(b.ops.len(), "operations")?;

        let mut lists: Vec<u16> = Vec::new();
        let mut list = |items: &mut dyn Iterator<Item = u16>| -> Result<[u8; 4], String> {
            let start = u16_of(lists.len(), "list entries")?;
            lists.extend(items);
            let count = u16_of(lists.len() - start as usize, "list entries")?;
            u16_of(lists.len(), "list entries")?;
            let mut r = [0; 4];
            r[..2].copy_from_slice(&start.to_be_bytes());
            r[2..].copy_from_slice(&count.to_be_bytes());
            Ok(r)
        };
        let mut states = Vec::new();
        for s in &b.states {
            states.extend_from_slice(&strings.id(&s.name)?.to_be_bytes());
            states.extend_from_slice(&opt(s.parent).to_be_bytes());
            states.extend_from_slice(&opt(s.interrupt).to_be_bytes());
            states.extend_from_slice(&opt(s.expression).to_be_bytes());
            states.push(u8::from(s.enabled) | (u8::from(s.active) << 1));
            states.push(s.interruptable);
            states.extend_from_slice(&list(&mut s.children.iter().map(|&c| c as u16))?);
            states.extend_from_slice(&list(&mut s.transitions.iter().map(|&c| c as u16))?);
            states.extend_from_slice(&list(&mut s.behaviours.iter().map(|&c| c as u16))?);
            states.extend_from_slice(&[0, 0]);
        }
        let mut transitions = Vec::new();
        for t in &b.transitions {
            let target = t
                .target
                .or(if t.enabled { None } else { Some(t.owner) })
                .ok_or_else(|| format!("unresolved transition target {}", t.target_name))?;
            transitions.extend_from_slice(&(t.owner as u16).to_be_bytes());
            transitions.extend_from_slice(&(target as u16).to_be_bytes());
            transitions.extend_from_slice(&opt(t.expression).to_be_bytes());
            transitions.push(u8::from(t.enabled));
            transitions.push(t.priority);
            transitions.extend_from_slice(&list(&mut t.hooks.iter().map(|&c| c as u16))?);
        }
        let mut exprs = Vec::new();
        for e in &b.exprs {
            exprs.push(e.operator);
            exprs.extend_from_slice(&[1, 0, 0]);
            let mut children = Vec::new();
            for c in &e.children {
                children.push(match *c {
                    Node::Op(o) => o as u16 | LIST_OP,
                    Node::Expression(x) => x as u16,
                    _ => return Err("bad expression child".into()),
                });
            }
            exprs.extend_from_slice(&list(&mut children.into_iter())?);
        }
        let mut attrs: Vec<u8> = Vec::new();
        let mut n_attrs = 0u32;
        let mut params: Vec<u8> = Vec::new();
        let mut n_params = 0usize;
        let mut add_attrs =
            |e: &Element, skip_name: bool, strings: &mut Strings| -> Result<(u32, u16), String> {
                let first = n_attrs;
                for a in &e.attrs {
                    if skip_name && a.name == "name" {
                        continue;
                    }
                    attrs.extend_from_slice(&strings.id(&a.name)?.to_be_bytes());
                    attrs.extend_from_slice(&strings.id(&a.text)?.to_be_bytes());
                    attrs.extend_from_slice(&a.float.to_be_bytes());
                    attrs.extend_from_slice(&[a.flag, 0, 0, 0]);
                    n_attrs += 1;
                }
                Ok((first, u16_of((n_attrs - first) as usize, "attributes")?))
            };
        let mut ops = Vec::new();
        for o in &b.ops {
            // Conditions that name a state are bound here (SK8 bind_states): `state` is looked up from
            // the owning state; InParentStateForTime is the owning state itself.
            let bound = if o.kind != OP_CONDITION {
                None
            } else if o.name == "InParentStateForTime" {
                Some(o.owner)
            } else if let Some(n) = o.element.text("state") {
                b.find_state(o.owner, n, true)
            } else {
                None
            };
            ops.push(o.kind);
            ops.push(u8::from(o.enabled));
            ops.push(o.mask);
            ops.push(0);
            ops.extend_from_slice(&strings.id(&o.name)?.to_be_bytes());
            ops.extend_from_slice(&(o.owner as u16).to_be_bytes());
            ops.extend_from_slice(&opt(bound).to_be_bytes());
            ops.extend_from_slice(&[0, 0]);
            let (first, count) = add_attrs(o.element, true, &mut strings)?;
            ops.extend_from_slice(&first.to_be_bytes());
            ops.extend_from_slice(&count.to_be_bytes());
            ops.extend_from_slice(&u16_of(n_params, "parameters")?.to_be_bytes());
            ops.extend_from_slice(&u16_of(o.params.len(), "parameters")?.to_be_bytes());
            ops.extend_from_slice(&[0, 0]);
            for p in &o.params {
                let (first, count) = add_attrs(p, false, &mut strings)?;
                params.extend_from_slice(&first.to_be_bytes());
                params.extend_from_slice(&count.to_be_bytes());
                params.extend_from_slice(&[0, 0]);
                n_params += 1;
            }
        }
        let mut list_bytes = Vec::new();
        for v in &lists {
            list_bytes.extend_from_slice(&v.to_be_bytes());
        }
        let name = strings.id(name)?;
        bodies.push((
            name,
            0u32,
            [
                (states, b.states.len()),
                (transitions, b.transitions.len()),
                (exprs, b.exprs.len()),
                (ops, b.ops.len()),
                (params, n_params),
                (attrs, n_attrs as usize),
                (list_bytes, lists.len()),
            ],
        ));
    }

    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_be_bytes());
    out.extend_from_slice(&(bodies.len() as u32).to_be_bytes());
    out.extend_from_slice(&(strings.list.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0; 8]);
    let headers = out.len();
    out.resize(headers + bodies.len() * super::GRAPH_HEADER, 0);
    for (g, (name, root, sections)) in bodies.iter().enumerate() {
        let h = headers + g * super::GRAPH_HEADER;
        out[h..h + 4].copy_from_slice(&(*name as u32).to_be_bytes());
        out[h + 4..h + 8].copy_from_slice(&root.to_be_bytes());
        for (i, (bytes, count)) in sections.iter().enumerate() {
            while out.len() % 4 != 0 {
                out.push(0);
            }
            let at = out.len() as u32;
            out[h + 8 + i * 8..h + 12 + i * 8].copy_from_slice(&at.to_be_bytes());
            out[h + 12 + i * 8..h + 16 + i * 8].copy_from_slice(&(*count as u32).to_be_bytes());
            out.extend_from_slice(bytes);
        }
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }
    let offsets_at = out.len() as u32;
    let mut blob = Vec::new();
    for s in &strings.list {
        out.extend_from_slice(&(blob.len() as u32).to_be_bytes());
        blob.extend_from_slice(s.as_bytes());
        blob.push(0);
    }
    let blob_at = out.len() as u32;
    out.extend_from_slice(&blob);
    out[16..20].copy_from_slice(&offsets_at.to_be_bytes());
    out[20..24].copy_from_slice(&blob_at.to_be_bytes());
    Ok(out)
}
