//! Writes `motion/db` entries (the layout is in the parent module). Used by `fg_import motion` and by
//! tests, so the writer and the reader can't drift apart. Not used by the game itself.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::{
    HAS_POSE, MAGIC, TREE_BLEND_SPACE, TREE_PHASE_BLEND, TREE_SELECTION_SPACE, TREE_SELECTOR,
    VERSION,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttrIn {
    pub name: String,
    pub kind: u8,
    pub begin: f32,
    pub end: f32,
    pub words: Vec<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipIn {
    pub name: String,
    pub fps: f32,
    pub frames: f32,
    pub base_speed: f32,
    pub andale_flags: u32,
    pub has_pose: bool,
    pub attrs: Vec<AttrIn>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SimplexIn {
    pub children: Vec<u16>,
    pub vertices: Vec<Vec<f32>>,
    pub normals: Vec<Vec<f32>>,
    pub scales: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TreeIn {
    BlendSpace {
        parameters: Vec<String>,
        children: Vec<String>,
        simplexes: Vec<SimplexIn>,
    },
    PhaseBlend {
        parameter: String,
        children: Vec<String>,
    },
    Selector {
        parameter: String,
        default: String,
        /// (value, child)
        options: Vec<(String, String)>,
    },
    SelectionSpace {
        /// (name, mode, weight, minimum, maximum)
        parameters: Vec<(String, u32, f32, f32, f32)>,
        candidates: Vec<(String, Vec<f32>)>,
    },
}

impl TreeIn {
    fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        match self {
            TreeIn::BlendSpace {
                parameters,
                children,
                ..
            } => v.extend(parameters.iter().chain(children).map(String::as_str)),
            TreeIn::PhaseBlend {
                parameter,
                children,
            } => {
                v.push(parameter);
                v.extend(children.iter().map(String::as_str));
            }
            TreeIn::Selector {
                parameter,
                default,
                options,
            } => {
                v.push(parameter);
                v.push(default);
                for (a, b) in options {
                    v.push(a);
                    v.push(b);
                }
            }
            TreeIn::SelectionSpace {
                parameters,
                candidates,
            } => {
                v.extend(parameters.iter().map(|p| p.0.as_str()));
                v.extend(candidates.iter().map(|c| c.0.as_str()));
            }
        }
        v
    }
}

fn u16s(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn u32s(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn f32s(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn pad4(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(4), 0);
}

/// The entry bytes. Names are stored upper case; a name that is both a clip and a tree keeps the tree.
/// Panics if more than 65536 distinct names are used.
pub fn write(clips: &[ClipIn], trees: &[(String, TreeIn)]) -> Vec<u8> {
    let up = |s: &str| s.to_ascii_uppercase();
    let tree_names: BTreeMap<String, &TreeIn> = trees.iter().map(|(n, t)| (up(n), t)).collect();
    let clips: BTreeMap<String, &ClipIn> = clips
        .iter()
        .map(|c| (up(&c.name), c))
        .filter(|(n, _)| !tree_names.contains_key(n))
        .collect();
    let mut names: BTreeMap<String, u16> = BTreeMap::new();
    for (n, c) in &clips {
        names.insert(n.clone(), 0);
        for a in &c.attrs {
            names.insert(up(&a.name), 0);
        }
    }
    for (n, t) in &tree_names {
        names.insert(n.clone(), 0);
        for x in t.names() {
            names.insert(up(x), 0);
        }
    }
    assert!(names.len() <= 65536, "too many names");
    for (i, v) in names.values_mut().enumerate() {
        *v = i as u16;
    }
    let id = |s: &str| names[&up(s)];

    let mut strings = Vec::new();
    let mut name_table = Vec::new();
    for n in names.keys() {
        u32s(&mut name_table, strings.len() as u32);
        strings.extend_from_slice(n.as_bytes());
        strings.push(0);
    }
    let (mut clip_table, mut attr_table, mut words) = (Vec::new(), Vec::new(), Vec::new());
    let mut n_attrs = 0u32;
    let mut n_words = 0u32;
    for (n, c) in &clips {
        u16s(&mut clip_table, id(n));
        u16s(&mut clip_table, c.attrs.len() as u16);
        u32s(&mut clip_table, n_attrs);
        f32s(&mut clip_table, c.fps);
        f32s(&mut clip_table, c.frames);
        f32s(&mut clip_table, c.base_speed);
        u32s(&mut clip_table, c.andale_flags);
        u32s(&mut clip_table, if c.has_pose { HAS_POSE } else { 0 });
        for a in &c.attrs {
            u16s(&mut attr_table, id(&a.name));
            attr_table.push(a.kind);
            attr_table.push(0);
            f32s(&mut attr_table, a.begin);
            f32s(&mut attr_table, a.end);
            u32s(&mut attr_table, n_words);
            u32s(&mut attr_table, a.words.len() as u32);
            for &w in &a.words {
                u32s(&mut words, w);
            }
            n_words += a.words.len() as u32;
            n_attrs += 1;
        }
    }
    let (mut tree_table, mut blob) = (Vec::new(), Vec::new());
    for (n, t) in &tree_names {
        let start = blob.len();
        let kind = match t {
            TreeIn::PhaseBlend {
                parameter,
                children,
            } => {
                u16s(&mut blob, id(parameter));
                u16s(&mut blob, children.len() as u16);
                for c in children {
                    u16s(&mut blob, id(c));
                }
                TREE_PHASE_BLEND
            }
            TreeIn::Selector {
                parameter,
                default,
                options,
            } => {
                u16s(&mut blob, id(parameter));
                u16s(&mut blob, id(default));
                u16s(&mut blob, options.len() as u16);
                u16s(&mut blob, 0);
                for (v, c) in options {
                    u16s(&mut blob, id(v));
                    u16s(&mut blob, id(c));
                }
                TREE_SELECTOR
            }
            TreeIn::BlendSpace {
                parameters,
                children,
                simplexes,
            } => {
                u16s(&mut blob, parameters.len() as u16);
                u16s(&mut blob, children.len() as u16);
                u16s(&mut blob, simplexes.len() as u16);
                u16s(&mut blob, 0);
                for p in parameters.iter().chain(children) {
                    u16s(&mut blob, id(p));
                }
                pad4(&mut blob);
                for s in simplexes {
                    for &c in &s.children {
                        u16s(&mut blob, c);
                    }
                    pad4(&mut blob);
                    for row in s.vertices.iter().chain(&s.normals) {
                        for &v in row {
                            f32s(&mut blob, v);
                        }
                    }
                    for &v in &s.scales {
                        f32s(&mut blob, v);
                    }
                }
                TREE_BLEND_SPACE
            }
            TreeIn::SelectionSpace {
                parameters,
                candidates,
            } => {
                u16s(&mut blob, parameters.len() as u16);
                u16s(&mut blob, candidates.len() as u16);
                for (name, mode, weight, min, max) in parameters {
                    u16s(&mut blob, id(name));
                    u16s(&mut blob, *mode as u16);
                    f32s(&mut blob, *weight);
                    f32s(&mut blob, *min);
                    f32s(&mut blob, *max);
                }
                for (child, values) in candidates {
                    u16s(&mut blob, id(child));
                    u16s(&mut blob, 0);
                    for &v in values {
                        f32s(&mut blob, v);
                    }
                }
                TREE_SELECTION_SPACE
            }
        };
        pad4(&mut blob);
        u16s(&mut tree_table, id(n));
        tree_table.push(kind);
        tree_table.push(0);
        u32s(&mut tree_table, start as u32);
        u32s(&mut tree_table, (blob.len() - start) as u32);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    u32s(&mut out, VERSION);
    for v in [
        names.len(),
        clips.len(),
        n_attrs as usize,
        n_words as usize,
        tree_names.len(),
        blob.len(),
    ] {
        u32s(&mut out, v as u32);
    }
    for part in [
        name_table, clip_table, attr_table, words, tree_table, blob, strings,
    ] {
        out.extend_from_slice(&part);
    }
    out
}
