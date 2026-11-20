//! Skate 3's motion library, as `fg_import motion` converts it from the player's disc (`motion.fgpak`):
//! every clip's timing, attribute tracks and events, and the animation trees (blend spaces, phase blends,
//! selectors, selection spaces) that pick and blend clips. The clips' poses are separate pack entries
//! (`motion/clip/<NAME>`, `anim::Clip`).
//!
//! The meaning of the data follows SK8-ENGINE/skate-3-rust-engine (GPL-3.0; crates/skate-core/src/animation:
//! playback_clip.rs, clip_clock.rs, playback_attributes.rs, playback_tree.rs). [`tree`] evaluates trees.
//!
//! Units: seconds for clip and tree time; attribute begin/end are stored as fractions of the clip's
//! length (-1 = untimed, always on) and reported in seconds; curve attributes are sampled by frame
//! (`time * fps * speed`).
//!
//! Entry `motion/db` (kind `MotionDb`), big-endian, read once at load:
//! ```text
//! 0   "FGMD", u32 version (1)
//! 8   u32 names, u32 clips, u32 attributes, u32 words, u32 trees, u32 tree bytes
//! 32  names: u32 offset into the string blob each, sorted by name (upper case)
//!     clips: 28 bytes each, sorted by name: u16 name, u16 attribute count, u32 first attribute,
//!            f32 fps, f32 frames, f32 base speed, u32 Andale flags, u32 flags (bit 0: pose in the pack)
//!     attributes: 20 bytes each, in clip order then authored order: u16 name, u8 kind, u8 0,
//!            f32 begin, f32 end, u32 first word, u32 word count
//!     words: u32 each (attribute payloads as authored)
//!     trees: 12 bytes each, sorted by name: u16 name, u8 type (6, 7, 8, 11), u8 0, u32 offset, u32 size
//!     tree bytes (layouts at `read_tree`), padded to 4
//!     string blob: NUL-terminated names
//! ```

pub mod tree;
pub mod write;

use alloc::string::String;
use alloc::vec::Vec;

use crate::anim::Clip;
use crate::pack::{Kind, Pack};

pub const MAGIC: [u8; 4] = *b"FGMD";
pub const VERSION: u32 = 1;
pub const HEADER: usize = 32;
pub const CLIP_SIZE: usize = 28;
pub const ATTR_SIZE: usize = 20;
pub const TREE_SIZE: usize = 12;
/// Clip record flag: the pose data is in the pack as `motion/clip/<NAME>`.
pub const HAS_POSE: u32 = 1;
/// Andale clip flags.
pub const ANDALE_LOOP: u32 = 0x1000_0000;
pub const ANDALE_PHASE: u32 = 0x4000_0000;

pub const TREE_BLEND_SPACE: u8 = 6;
pub const TREE_PHASE_BLEND: u8 = 7;
pub const TREE_SELECTOR: u8 = 8;
pub const TREE_SELECTION_SPACE: u8 = 11;

/// Attribute kinds.
pub const KIND_SCALAR: u8 = 0;
pub const KIND_VECTOR: u8 = 1;
pub const KIND_CURVE: u8 = 2;
pub const KIND_BONE: u8 = 3;

/// Index into the name table.
pub type NameId = u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    BadMagic,
    BadVersion(u32),
    Truncated,
    /// A record points outside its table.
    Bad,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipInfo {
    pub name: NameId,
    pub fps: f32,
    pub frames: f32,
    pub base_speed: f32,
    pub andale_flags: u32,
    pub has_pose: bool,
    first_attr: u32,
    attr_count: u16,
}

impl ClipInfo {
    /// Length in seconds at playback speed 1 (SK8 `PlaybackClip::new`).
    pub fn length(&self) -> f32 {
        (self.frames - 1.0) / (self.base_speed * self.fps)
    }
    pub fn looping(&self) -> bool {
        self.andale_flags & ANDALE_LOOP != 0
    }
    /// Time comes from a phase parameter instead of a clock.
    pub fn phase_controlled(&self) -> bool {
        self.andale_flags & ANDALE_PHASE != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttrDef {
    pub name: NameId,
    pub kind: u8,
    /// Fractions of the clip's length; -1 = untimed.
    pub begin: f32,
    pub end: f32,
    first_word: u32,
    word_count: u32,
}

impl AttrDef {
    pub fn untimed(&self) -> bool {
        self.begin == -1.0
    }
}

/// What a tree child (or any name) refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    Clip(u16),
    Tree(u16),
    /// Not in the library (not converted, or an unsupported tree type).
    Missing(NameId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimplexDef {
    /// Indices into the tree's children.
    pub children: Vec<u16>,
    pub vertices: Vec<Vec<f32>>,
    pub normals: Vec<Vec<f32>>,
    pub scales: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionParam {
    pub name: NameId,
    /// 0 plain, 1 one-way wrap, 2 circular (SK8 selection_space::delta).
    pub mode: u32,
    pub weight: f32,
    pub minimum: f32,
    pub maximum: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TreeDef {
    BlendSpace {
        parameters: Vec<NameId>,
        children: Vec<Node>,
        simplexes: Vec<SimplexDef>,
    },
    PhaseBlend {
        parameter: NameId,
        children: Vec<Node>,
    },
    /// Picks one child when the tree is built, by the construction value named `parameter`.
    Selector {
        parameter: NameId,
        default: Node,
        options: Vec<(NameId, Node)>,
    },
    SelectionSpace {
        parameters: Vec<SelectionParam>,
        candidates: Vec<(Node, Vec<f32>)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeInfo {
    pub name: NameId,
    pub def: TreeDef,
}

/// An attribute's value at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttrValue {
    pub name: NameId,
    pub kind: u8,
    /// Scalar value: the constant, the curve sampled at the time, or a bone reference's number.
    pub value: f32,
    /// A vector attribute's four lanes (`value` is lane 0).
    pub vector: [f32; 4],
    /// A bone reference's bone name (FastString words; [`fast_string`] decodes them).
    pub bone: Option<[u32; 5]>,
    /// Begin and end in seconds (-1 for untimed).
    pub begin: f32,
    pub end: f32,
    /// SK8's status byte: 6 untimed, 5 active now, 9 crossed since the previous time, 17 not touched.
    pub status: u8,
}

pub struct MotionDb<'a> {
    blob: &'a [u8],
    strings: usize,
    names: Vec<u32>,
    clips: Vec<ClipInfo>,
    attrs: Vec<AttrDef>,
    words: Vec<u32>,
    trees: Vec<TreeInfo>,
    /// The trees' names, in table order (known before their definitions are read, which refer to
    /// each other).
    tree_names: Vec<NameId>,
}

fn be32(b: &[u8], at: usize) -> Result<u32, Error> {
    Ok(u32::from_be_bytes(
        b.get(at..at + 4)
            .ok_or(Error::Truncated)?
            .try_into()
            .unwrap(),
    ))
}
fn be16(b: &[u8], at: usize) -> Result<u16, Error> {
    Ok(u16::from_be_bytes(
        b.get(at..at + 2)
            .ok_or(Error::Truncated)?
            .try_into()
            .unwrap(),
    ))
}
fn bef(b: &[u8], at: usize) -> Result<f32, Error> {
    be32(b, at).map(f32::from_bits)
}

/// SK8 attribute status (ClipClock::attribute_status) for begin/end in seconds over the step from
/// `previous` to `time`, with `loops` wraps in between.
pub fn status(begin: f32, end: f32, previous: f32, time: f32, loops: u32) -> u8 {
    if begin == -1.0 {
        return 6;
    }
    let hit = if loops == 0 {
        end > previous && begin <= time
    } else {
        end > previous || begin <= time
    };
    if !hit {
        17
    } else if time < begin || time > end {
        9
    } else {
        5
    }
}

/// SK8's mask test (PlaybackClip::attribute): 15 = active now or crossed this step, 31 = all.
pub fn passes(mask: u32, status: u8) -> bool {
    let s = status as u32;
    mask & 0x1c & s != 0 && mask & 3 & s != 0
}

/// A curve payload: an opaque word, a point count, then (frame, value) pairs; linear between points, held
/// at the ends (SK8 playback_clip::sample_curve).
pub fn sample_curve(words: &[u32], frame: f32) -> Option<f32> {
    let count = *words.get(1)? as usize;
    if count == 0 || count > words.len().saturating_sub(2) / 2 {
        return None;
    }
    let point = |i: usize| {
        (
            f32::from_bits(words[2 + 2 * i]),
            f32::from_bits(words[3 + 2 * i]),
        )
    };
    let (x0, y0) = point(0);
    if frame <= x0 {
        return Some(y0);
    }
    let (xn, yn) = point(count - 1);
    if frame >= xn {
        return Some(yn);
    }
    // The last point at or before `frame`.
    let (mut lo, mut hi) = (0, count - 1);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if point(mid).0 <= frame {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let ((ax, ay), (bx, by)) = (point(lo), point(hi));
    if bx == ax {
        return Some(ay);
    }
    Some(ay + (by - ay) * (frame - ax) / (bx - ax))
}

/// Decodes a FastString (Skate 3's packed upper-case names: up to 6 characters per word, base 38).
pub fn fast_string(words: &[u32]) -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_";
    let mut out = String::new();
    for &w in words {
        let (mut value, mut divisor) = (w, 38u32.pow(5));
        for _ in 0..6 {
            let digit = value / divisor;
            value %= divisor;
            if digit == 0 {
                break;
            }
            if let Some(&c) = ALPHABET.get(digit as usize - 1) {
                out.push(c as char);
            }
            divisor /= 38;
        }
    }
    out
}

impl<'a> MotionDb<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Self, Error> {
        if b.get(..4) != Some(&MAGIC[..]) {
            return Err(Error::BadMagic);
        }
        let version = be32(b, 4)?;
        if version != VERSION {
            return Err(Error::BadVersion(version));
        }
        let [n_names, n_clips, n_attrs, n_words, n_trees, tree_bytes] =
            core::array::from_fn(|i| be32(b, 8 + 4 * i).unwrap_or(u32::MAX) as usize);
        let mut at = HEADER;
        let mut take = |len: Option<usize>| -> Result<usize, Error> {
            let start = at;
            at = len
                .and_then(|l| at.checked_add(l))
                .filter(|&e| e <= b.len())
                .ok_or(Error::Truncated)?;
            Ok(start)
        };
        let names_at = take(n_names.checked_mul(4))?;
        let clips_at = take(n_clips.checked_mul(CLIP_SIZE))?;
        let attrs_at = take(n_attrs.checked_mul(ATTR_SIZE))?;
        let words_at = take(n_words.checked_mul(4))?;
        let trees_at = take(n_trees.checked_mul(TREE_SIZE))?;
        let tree_blob = take(Some(tree_bytes.next_multiple_of(4)))?;
        let strings = take(Some(0))?;
        if n_names > u16::MAX as usize + 1 || n_clips > u16::MAX as usize {
            return Err(Error::Bad);
        }
        let names = (0..n_names)
            .map(|i| be32(b, names_at + 4 * i))
            .collect::<Result<Vec<_>, _>>()?;
        let mut db = MotionDb {
            blob: b,
            strings,
            names,
            clips: Vec::with_capacity(n_clips),
            attrs: Vec::with_capacity(n_attrs),
            words: (0..n_words)
                .map(|i| be32(b, words_at + 4 * i))
                .collect::<Result<_, _>>()?,
            trees: Vec::with_capacity(n_trees),
            tree_names: Vec::with_capacity(n_trees),
        };
        for i in 0..n_names {
            db.str_at(db.names[i]).ok_or(Error::Bad)?;
        }
        let name = |v: u16| {
            if (v as usize) < n_names {
                Ok(v)
            } else {
                Err(Error::Bad)
            }
        };
        for i in 0..n_clips {
            let c = clips_at + i * CLIP_SIZE;
            let clip = ClipInfo {
                name: name(be16(b, c)?)?,
                attr_count: be16(b, c + 2)?,
                first_attr: be32(b, c + 4)?,
                fps: bef(b, c + 8)?,
                frames: bef(b, c + 12)?,
                base_speed: bef(b, c + 16)?,
                andale_flags: be32(b, c + 20)?,
                has_pose: be32(b, c + 24)? & HAS_POSE != 0,
            };
            if clip.first_attr as usize + clip.attr_count as usize > n_attrs
                || !(clip.fps > 0.0 && clip.frames >= 1.0 && clip.base_speed > 0.0)
            {
                return Err(Error::Bad);
            }
            db.clips.push(clip);
        }
        for i in 0..n_attrs {
            let a = attrs_at + i * ATTR_SIZE;
            let attr = AttrDef {
                name: name(be16(b, a)?)?,
                kind: b[a + 2],
                begin: bef(b, a + 4)?,
                end: bef(b, a + 8)?,
                first_word: be32(b, a + 12)?,
                word_count: be32(b, a + 16)?,
            };
            let lanes = match attr.kind {
                KIND_SCALAR => 1,
                KIND_VECTOR => 4,
                KIND_CURVE => 2,
                KIND_BONE => 6,
                _ => 0,
            };
            if attr.first_word as usize + attr.word_count as usize > n_words
                || (attr.word_count as usize) < lanes
            {
                return Err(Error::Bad);
            }
            db.attrs.push(attr);
        }
        let blob = &b[tree_blob..tree_blob + tree_bytes];
        for i in 0..n_trees {
            let tree_name = name(be16(b, trees_at + i * TREE_SIZE)?)?;
            if db.tree_names.last().is_some_and(|&l| l >= tree_name) {
                return Err(Error::Bad);
            }
            db.tree_names.push(tree_name);
        }
        for i in 0..n_trees {
            let t = trees_at + i * TREE_SIZE;
            let (off, size) = (be32(b, t + 4)? as usize, be32(b, t + 8)? as usize);
            let data = off
                .checked_add(size)
                .and_then(|e| blob.get(off..e))
                .ok_or(Error::Truncated)?;
            let def = db.read_tree(b[t + 2], data).ok_or(Error::Bad)?;
            db.trees.push(TreeInfo {
                name: db.tree_names[i],
                def,
            });
        }
        Ok(db)
    }

    fn str_at(&self, offset: u32) -> Option<&'a str> {
        let tail = self.blob.get(self.strings + offset as usize..)?;
        let len = tail.iter().position(|&c| c == 0)?;
        core::str::from_utf8(&tail[..len]).ok()
    }

    /// Tree layouts (u16 values are name ids unless said otherwise):
    /// - blend space (6): u16 d, u16 children, u16 simplexes, u16 0, d params, children, then per
    ///   simplex: d+1 u16 child indices (padded to 4 bytes), (d+1)*d f32 vertices, (d+1)*d f32 normals,
    ///   d+1 f32 scales
    /// - phase blend (7): u16 parameter, u16 count, children
    /// - selector (8): u16 parameter, u16 default, u16 count, u16 0, count x (u16 value, u16 child)
    /// - selection space (11): u16 params, u16 candidates, params x (u16 name, u16 mode, f32 weight,
    ///   f32 min, f32 max), candidates x (u16 child, u16 0, params x f32)
    fn read_tree(&self, kind: u8, d: &[u8]) -> Option<TreeDef> {
        let h = |at: usize| be16(d, at).ok();
        let f = |at: usize| bef(d, at).ok();
        let node = |v: u16| self.resolve_id(v);
        let names = self.names.len();
        let id = |v: u16| ((v as usize) < names).then_some(v);
        Some(match kind {
            TREE_PHASE_BLEND => {
                let count = h(2)? as usize;
                TreeDef::PhaseBlend {
                    parameter: id(h(0)?)?,
                    children: (0..count)
                        .map(|i| Some(node(id(h(4 + 2 * i)?)?)))
                        .collect::<Option<Vec<_>>>()?,
                }
            }
            TREE_SELECTOR => {
                let count = h(4)? as usize;
                TreeDef::Selector {
                    parameter: id(h(0)?)?,
                    default: node(id(h(2)?)?),
                    options: (0..count)
                        .map(|i| Some((id(h(8 + 4 * i)?)?, node(id(h(10 + 4 * i)?)?))))
                        .collect::<Option<Vec<_>>>()?,
                }
            }
            TREE_BLEND_SPACE => {
                let (dim, count, simplexes) = (h(0)? as usize, h(2)? as usize, h(4)? as usize);
                if dim == 0 || dim > 4 || count < dim + 1 || simplexes == 0 {
                    return None;
                }
                let mut at = 8;
                let parameters = (0..dim)
                    .map(|i| id(h(at + 2 * i)?))
                    .collect::<Option<Vec<_>>>()?;
                at += 2 * dim;
                let children = (0..count)
                    .map(|i| Some(node(id(h(at + 2 * i)?)?)))
                    .collect::<Option<Vec<_>>>()?;
                at += 2 * count;
                at = at.next_multiple_of(4);
                let mut list = Vec::with_capacity(simplexes);
                for _ in 0..simplexes {
                    let n = dim + 1;
                    let idx = (0..n)
                        .map(|i| h(at + 2 * i).filter(|&c| (c as usize) < count))
                        .collect::<Option<Vec<_>>>()?;
                    at = (at + 2 * n).next_multiple_of(4);
                    let rows = |at: &mut usize| {
                        (0..n)
                            .map(|_| {
                                let row =
                                    (0..dim).map(|j| f(*at + 4 * j)).collect::<Option<Vec<_>>>();
                                *at += 4 * dim;
                                row
                            })
                            .collect::<Option<Vec<_>>>()
                    };
                    let vertices = rows(&mut at)?;
                    let normals = rows(&mut at)?;
                    let scales = (0..n).map(|i| f(at + 4 * i)).collect::<Option<Vec<_>>>()?;
                    at += 4 * n;
                    list.push(SimplexDef {
                        children: idx,
                        vertices,
                        normals,
                        scales,
                    });
                }
                TreeDef::BlendSpace {
                    parameters,
                    children,
                    simplexes: list,
                }
            }
            TREE_SELECTION_SPACE => {
                let (np, nc) = (h(0)? as usize, h(2)? as usize);
                if np > 10 || nc == 0 {
                    return None;
                }
                let mut at = 4;
                let parameters = (0..np)
                    .map(|i| {
                        let p = at + 16 * i;
                        Some(SelectionParam {
                            name: id(h(p)?)?,
                            mode: h(p + 2)? as u32,
                            weight: f(p + 4)?,
                            minimum: f(p + 8)?,
                            maximum: f(p + 12)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                at += 16 * np;
                let candidates = (0..nc)
                    .map(|i| {
                        let c = at + i * (4 + 4 * np);
                        Some((
                            node(id(h(c)?)?),
                            (0..np)
                                .map(|j| f(c + 4 + 4 * j))
                                .collect::<Option<Vec<_>>>()?,
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                TreeDef::SelectionSpace {
                    parameters,
                    candidates,
                }
            }
            _ => return None,
        })
    }

    pub fn name(&self, id: NameId) -> &'a str {
        self.names
            .get(id as usize)
            .and_then(|&o| self.str_at(o))
            .unwrap_or("")
    }

    /// The id of `name` (any case), if the library uses it.
    pub fn name_id(&self, name: &str) -> Option<NameId> {
        let (mut lo, mut hi) = (0usize, self.names.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match cmp_upper(self.name(mid as NameId), name) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return Some(mid as NameId),
            }
        }
        None
    }

    /// Clips and trees by name id (both tables are sorted by name, and so by id).
    fn resolve_id(&self, id: NameId) -> Node {
        if let Ok(i) = self.clips.binary_search_by_key(&id, |c| c.name) {
            Node::Clip(i as u16)
        } else if let Ok(i) = self.tree_names.binary_search(&id) {
            Node::Tree(i as u16)
        } else {
            Node::Missing(id)
        }
    }

    /// A clip or tree by name (any case). A name that is both is the tree (the bank's later record wins;
    /// the importer keeps only that one).
    pub fn find(&self, name: &str) -> Option<Node> {
        match self.resolve_id(self.name_id(name)?) {
            Node::Missing(_) => None,
            n => Some(n),
        }
    }

    pub fn clip_index(&self, name: &str) -> Option<u16> {
        match self.find(name)? {
            Node::Clip(i) => Some(i),
            _ => None,
        }
    }

    pub fn tree_index(&self, name: &str) -> Option<u16> {
        match self.find(name)? {
            Node::Tree(i) => Some(i),
            _ => None,
        }
    }

    pub fn clips(&self) -> &[ClipInfo] {
        &self.clips
    }

    pub fn clip(&self, i: u16) -> &ClipInfo {
        &self.clips[i as usize]
    }

    pub fn trees(&self) -> &[TreeInfo] {
        &self.trees
    }

    pub fn tree(&self, i: u16) -> &TreeInfo {
        &self.trees[i as usize]
    }

    pub fn names(&self) -> usize {
        self.names.len()
    }

    /// Every attribute record of a clip, in authored order.
    pub fn attributes(&self, clip: u16) -> &[AttrDef] {
        let c = &self.clips[clip as usize];
        &self.attrs[c.first_attr as usize..c.first_attr as usize + c.attr_count as usize]
    }

    pub fn attribute_count(&self) -> usize {
        self.attrs.len()
    }

    pub fn words(&self, a: &AttrDef) -> &[u32] {
        &self.words[a.first_word as usize..(a.first_word + a.word_count) as usize]
    }

    /// An attribute's value over the step from `previous` to `time` (seconds, at playback `speed`, with
    /// `loops` wraps in between), with SK8's status (`status`). `length` is the clip's current length.
    pub fn materialize(
        &self,
        clip: u16,
        a: &AttrDef,
        previous: f32,
        time: f32,
        loops: u32,
        speed: f32,
    ) -> AttrValue {
        let c = &self.clips[clip as usize];
        let length = c.length() / speed;
        let (begin, end) = if a.untimed() {
            (-1.0, -1.0)
        } else {
            (a.begin * length, a.end * length)
        };
        let st = status(begin, end, previous, time, loops);
        let w = self.words(a);
        let lane = |i: usize| w.get(i).map_or(0.0, |&v| f32::from_bits(v));
        let mut v = AttrValue {
            name: a.name,
            kind: a.kind,
            value: lane(0),
            vector: [lane(0), lane(1), lane(2), lane(3)],
            bone: None,
            begin: if st & 2 != 0 { -1.0 } else { begin },
            end: if st & 2 != 0 { -1.0 } else { end },
            status: st,
        };
        match a.kind {
            KIND_CURVE => {
                // SK8 samples by time * fps * playback speed (base speed is not a factor).
                v.value = sample_curve(w, time * c.fps * speed).unwrap_or(0.0);
                v.vector = [v.value, 0.0, 0.0, 0.0];
            }
            KIND_BONE => {
                v.bone = Some([w[0], w[1], w[2], w[3], w[4]]);
                v.value = lane(5);
                v.vector = [v.value, 0.0, 0.0, 0.0];
            }
            _ => {}
        }
        v
    }

    /// The first attribute named `name` that passes `mask` over the step (SK8 PlaybackClip::attribute).
    #[allow(clippy::too_many_arguments)]
    pub fn attribute_step(
        &self,
        clip: u16,
        name: NameId,
        previous: f32,
        time: f32,
        loops: u32,
        speed: f32,
        mask: u32,
    ) -> Option<AttrValue> {
        self.attributes(clip)
            .iter()
            .filter(|a| a.name == name)
            .map(|a| self.materialize(clip, a, previous, time, loops, speed))
            .find(|v| passes(mask, v.status))
    }

    /// An attribute's value at `time` (seconds from the clip's start, speed 1): untimed ones always,
    /// timed ones while `time` is inside them.
    pub fn attribute_at(&self, clip: u16, name: &str, time: f32) -> Option<AttrValue> {
        let id = self.name_id(name)?;
        self.attribute_step(clip, id, time, time, 0, 1.0, 15)
            .filter(|v| v.status != 9)
    }

    /// Timed attributes (events and windows) touched by the step from `previous` to `time` (seconds,
    /// speed 1, `loops` wraps in between): status 5 (inside now) or 9 (crossed during the step).
    pub fn events(
        &self,
        clip: u16,
        previous: f32,
        time: f32,
        loops: u32,
    ) -> impl Iterator<Item = AttrValue> + '_ {
        self.attributes(clip)
            .iter()
            .filter(|a| !a.untimed())
            .map(move |a| self.materialize(clip, a, previous, time, loops, 1.0))
            .filter(|v| v.status == 5 || v.status == 9)
    }
}

/// A loaded `motion.fgpak`: the database and the clips' poses.
pub struct Library<'a> {
    pub db: MotionDb<'a>,
    pack: Pack<'a>,
    /// Per clip of the database: its pose entry in the pack.
    poses: Vec<Option<u32>>,
}

/// Pack entry name prefix of the clip poses.
pub const POSE_PREFIX: &str = "motion/clip/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryError {
    Pack(crate::pack::Error),
    NoDb,
    Db(Error),
    /// A pose entry that doesn't parse.
    BadPose,
}

impl<'a> Library<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, LibraryError> {
        let pack = Pack::parse(bytes).map_err(LibraryError::Pack)?;
        let entry = pack
            .find("motion/db")
            .filter(|e| e.kind == Kind::MotionDb as u32)
            .ok_or(LibraryError::NoDb)?;
        let db = MotionDb::parse(entry.data).map_err(LibraryError::Db)?;
        let mut poses = alloc::vec![None; db.clips().len()];
        for i in 0..pack.len() {
            let Ok(e) = pack.entry(i) else { continue };
            if e.kind != Kind::Animation as u32 {
                continue;
            }
            if let Some(name) = e.name.strip_prefix(POSE_PREFIX)
                && let Some(c) = db.clip_index(name)
            {
                Clip::parse(e.data).map_err(|_| LibraryError::BadPose)?;
                poses[c as usize] = Some(i as u32);
            }
        }
        Ok(Library { db, pack, poses })
    }

    /// The pose data of a clip, if it was converted.
    pub fn pose(&self, clip: u16) -> Option<Clip<'a>> {
        let i = (*self.poses.get(clip as usize)?)?;
        Clip::parse(self.pack.entry(i as usize).ok()?.data).ok()
    }

    /// How many clips have their poses here.
    pub fn pose_count(&self) -> usize {
        self.poses.iter().filter(|p| p.is_some()).count()
    }
}

/// Compares `a` with `b` upper-cased (names are stored upper case).
fn cmp_upper(a: &str, b: &str) -> core::cmp::Ordering {
    a.bytes().cmp(b.bytes().map(|c| c.to_ascii_uppercase()))
}

/// Mirrors an attribute the way SK8 does for a mirrored tree (Attribute::Mirror): a vector's x lane
/// changes sign; a bone reference becomes the mirror bone (`mirror_bone` maps a bone name to its partner).
pub fn mirror_attribute(v: &mut AttrValue, mirror_bone: impl Fn(&str) -> Option<[u32; 5]>) {
    match v.kind {
        KIND_VECTOR => {
            v.vector[0] = -v.vector[0];
            v.value = v.vector[0];
        }
        KIND_BONE => {
            if let Some(bone) = v.bone
                && let Some(m) = mirror_bone(&fast_string(&bone))
            {
                v.bone = Some(m);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod tests;
