//! Clip metadata (attribute tracks and events) and animation trees from an Andale bank.
//!
//! Ported from SK8-ENGINE/skate-3-rust-engine (GPL-3.0): crates/skate-data/src/animation_metadata/native.rs,
//! blend_space.rs and selection_space.rs (record layouts), with the type meanings from
//! crates/skate-core/src/animation (playback_clip.rs, playback_tree.rs, phase_blend.rs). Values are kept as
//! authored (raw words); `game_core::motion` interprets them.

use super::abin::{Bank, Clip, Error, Reader, RecordData, RecordHeader, Result};

/// One attribute record of a clip (Attribute::Init in SK8's notes).
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    /// 0 scalar, 1 vector (4 lanes), 2 curve over frames, 3 bone reference (5 name words + scalar).
    pub kind: u8,
    /// Begin/end as fractions of the clip's length; -1 = untimed (always on).
    pub begin: f32,
    pub end: f32,
    /// The aligned payload words as stored.
    pub words: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipMeta {
    pub name: String,
    pub fps: f32,
    pub frames: f32,
    pub base_speed: f32,
    pub flags: u32,
    pub attributes: Vec<Attribute>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Simplex {
    /// Indices into the tree's children, d + 1 of them.
    pub children: Vec<u32>,
    /// d + 1 vertices of d coordinates.
    pub vertices: Vec<Vec<f32>>,
    pub normals: Vec<Vec<f32>>,
    pub scales: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionParameter {
    pub name: String,
    pub mode: u32,
    pub weight: f32,
    pub minimum: f32,
    pub maximum: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tree {
    /// Type 6: children blended by barycentric weights inside the simplex holding the parameters.
    BlendSpace {
        parameters: Vec<String>,
        children: Vec<String>,
        simplexes: Vec<Simplex>,
    },
    /// Type 7: two neighbouring children (sorted by their own value of `parameter`) blended, clocks in phase.
    PhaseBlend {
        parameter: String,
        children: Vec<String>,
    },
    /// Type 8: one child chosen when the tree is built, by a named construction value.
    Selector {
        parameter: String,
        default: String,
        children: Vec<String>,
        values: Vec<String>,
    },
    /// Type 11: the candidate nearest the parameters, chosen once.
    SelectionSpace {
        parameters: Vec<SelectionParameter>,
        candidates: Vec<(String, Vec<f32>)>,
    },
}

impl Tree {
    /// Every child name the tree may play.
    pub fn children(&self) -> Vec<&str> {
        let mut v: Vec<&str> = match self {
            Tree::BlendSpace { children, .. }
            | Tree::PhaseBlend { children, .. }
            | Tree::Selector { children, .. } => children.iter().map(String::as_str).collect(),
            Tree::SelectionSpace { candidates, .. } => {
                candidates.iter().map(|c| c.0.as_str()).collect()
            }
        };
        if let Tree::Selector { default, .. } = self {
            v.push(default);
        }
        v
    }
}

/// What a bank animation name resolves to. Later records with the same name win (the native map
/// overwrites in file order).
#[allow(dead_code)] // the payloads are for callers that need them
pub enum Animation<'a> {
    Clip(&'a ClipMeta),
    Tree(&'a Tree),
    /// A tree type nobody has decoded yet (9, 10).
    Unsupported(u32),
}

#[derive(Default)]
pub struct Meta {
    pub clips: Vec<ClipMeta>,
    pub trees: Vec<(String, Tree)>,
    pub unsupported: Vec<(String, u32)>,
    /// Upper-case name -> what it is, last record wins.
    names: std::collections::BTreeMap<String, (u8, usize)>,
}

impl Meta {
    pub fn read(bank: &Bank) -> Result<Meta> {
        let mut m = Meta::default();
        let reader = Reader::new(bank.bytes());
        for record in bank.records() {
            let h = &record.header;
            let r = reader.bounded(h.range())?;
            let (what, index) = match &record.data {
                RecordData::Clip(clip) => {
                    m.clips.push(read_clip(r, h, clip)?);
                    (0, m.clips.len() - 1)
                }
                RecordData::Opaque => {
                    let tree = match h.type_id {
                        6 => Some(read_blend_space(r, h)?),
                        7 => Some(read_phase_blend(r, h)?),
                        8 => Some(read_selector(r, h)?),
                        11 => Some(read_selection_space(r, h)?),
                        _ => None,
                    };
                    match tree {
                        Some(t) => {
                            m.trees.push((h.name.clone(), t));
                            (1, m.trees.len() - 1)
                        }
                        None if (9..=10).contains(&h.type_id) => {
                            m.unsupported.push((h.name.clone(), h.type_id));
                            (2, m.unsupported.len() - 1)
                        }
                        None => continue,
                    }
                }
                _ => continue,
            };
            m.names.insert(h.name.to_ascii_uppercase(), (what, index));
        }
        Ok(m)
    }

    pub fn get(&self, name: &str) -> Option<Animation<'_>> {
        let &(what, i) = self.names.get(&name.to_ascii_uppercase())?;
        Some(match what {
            0 => Animation::Clip(&self.clips[i]),
            1 => Animation::Tree(&self.trees[i].1),
            _ => Animation::Unsupported(self.unsupported[i].1),
        })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.names.contains_key(&name.to_ascii_uppercase())
    }

    /// Adds `other`'s animations under the names this one doesn't have (this one wins a clash), and
    /// returns those names (upper case), in record order.
    pub fn merge(&mut self, other: Meta) -> Vec<String> {
        let mut added = Vec::new();
        let Meta {
            clips,
            trees,
            unsupported,
            names,
        } = other;
        let mut clips: Vec<Option<ClipMeta>> = clips.into_iter().map(Some).collect();
        let mut trees: Vec<Option<(String, Tree)>> = trees.into_iter().map(Some).collect();
        // Only the record each name resolves to in `other` (its last), so a name keeps its kind.
        for (n, (what, i)) in names {
            if self.names.contains_key(&n) {
                continue;
            }
            let at = match what {
                0 => {
                    self.clips
                        .push(clips[i].take().expect("one name per record"));
                    self.clips.len() - 1
                }
                1 => {
                    self.trees
                        .push(trees[i].take().expect("one name per record"));
                    self.trees.len() - 1
                }
                _ => {
                    self.unsupported.push(unsupported[i].clone());
                    self.unsupported.len() - 1
                }
            };
            self.names.insert(n.clone(), (what, at));
            if what < 2 {
                added.push(n);
            }
        }
        added
    }
}

fn read_clip(r: Reader<'_>, h: &RecordHeader, clip: &Clip) -> Result<ClipMeta> {
    let mut at = clip.attribute_offset;
    let mut attributes = Vec::with_capacity(clip.attribute_count as usize);
    for _ in 0..clip.attribute_count {
        // Each record starts with its size; the payload starts 16-byte aligned after a 48-byte header.
        let size = r.u32(at)? as usize;
        if size < 48 {
            return Err(Error::new(at, "attribute record smaller than header"));
        }
        let end = at
            .checked_add(size)
            .ok_or_else(|| Error::new(at, "attribute size overflow"))?;
        let a = r.bounded(at..end)?;
        let data = (at + 51) & !15;
        let payload = a.take(
            data,
            end.checked_sub(data)
                .ok_or_else(|| Error::new(at, "attribute payload outside record"))?,
        )?;
        if !payload.len().is_multiple_of(4) {
            return Err(Error::new(at, "unaligned attribute payload"));
        }
        attributes.push(Attribute {
            name: a.name(at + 12, 5)?,
            kind: a.u8(at + 32)?,
            begin: f32::from_bits(a.u32(at + 4)?),
            end: f32::from_bits(a.u32(at + 8)?),
            words: payload
                .chunks_exact(4)
                .map(|w| u32::from_be_bytes(w.try_into().unwrap()))
                .collect(),
        });
        at = end;
    }
    Ok(ClipMeta {
        name: h.name.clone(),
        fps: f32::from_bits(clip.fps_bits),
        frames: f32::from_bits(clip.frame_count_bits),
        base_speed: f32::from_bits(clip.base_speed_bits),
        flags: clip.flags,
        attributes,
    })
}

fn read_phase_blend(r: Reader<'_>, h: &RecordHeader) -> Result<Tree> {
    // Count at +8, parameter at +16, child names (24 bytes each) from +36.
    let p = h.payload_offset;
    let count = r.u32(p + 8)? as usize;
    r.take(
        p + 36,
        count
            .checked_mul(24)
            .ok_or_else(|| Error::new(p, "PhaseBlend child count overflow"))?,
    )?;
    if count < 2 {
        return Err(Error::new(p, "PhaseBlend needs two children"));
    }
    Ok(Tree::PhaseBlend {
        parameter: r.name(p + 16, 5)?,
        children: (0..count)
            .map(|i| r.name(p + 36 + 24 * i, 6))
            .collect::<Result<_>>()?,
    })
}

fn read_selector(r: Reader<'_>, h: &RecordHeader) -> Result<Tree> {
    // The first child name is the default; children and keys are matched in order.
    let p = h.payload_offset;
    let count = r.u32(p + 28)? as usize;
    let names_size = count
        .max(1)
        .checked_mul(24)
        .ok_or_else(|| Error::new(p, "Selector child count overflow"))?;
    let keys_size = count
        .checked_mul(20)
        .ok_or_else(|| Error::new(p, "Selector key count overflow"))?;
    let names = r.relative(p, r.u32(p + 20)?, names_size)?.start;
    let keys = r.relative(p, r.u32(p + 24)?, keys_size)?.start;
    Ok(Tree::Selector {
        parameter: r.name(p, 5)?,
        default: r.name(names, 6)?,
        children: (0..count)
            .map(|i| r.name(names + 24 * i, 6))
            .collect::<Result<_>>()?,
        values: (0..count)
            .map(|i| r.name(keys + 20 * i, 5))
            .collect::<Result<_>>()?,
    })
}

fn read_blend_space(r: Reader<'_>, h: &RecordHeader) -> Result<Tree> {
    let p = h.payload_offset;
    let count = r.u32(p)? as usize;
    let names_size = count
        .checked_mul(24)
        .ok_or_else(|| Error::new(p, "BlendSpace count overflow"))?;
    let names = r.relative(p, r.u32(p + 4)?, names_size)?.start;
    let q = r.relative(p, r.u32(p + 12)?, 44)?.start;
    let space = r.bounded(r.relative(q, 0, r.u32(q)? as usize)?)?;
    let d = space.u32(q + 28)? as usize;
    if d == 0 || d > 4 || d > r.u32(p + 16)? as usize {
        return Err(Error::new(q + 28, "invalid BlendSpace dimension"));
    }
    let params = space.relative(q, space.u32(q + 32)?, d * 20)?.start;
    let simplex_count = space.u32(q + 36)? as usize;
    space.relative(
        q,
        space.u32(q + 40)?,
        simplex_count
            .checked_mul(20)
            .ok_or_else(|| Error::new(q, "BlendSpace simplex count overflow"))?,
    )?;
    let mut at = space.relative(q, space.u32(q + 40)?, 20)?.start;
    let mut simplexes = Vec::new();
    let f = |s: Reader<'_>, at: usize| s.u32(at).map(f32::from_bits);
    for _ in 0..simplex_count {
        let size = space.u32(at)? as usize;
        let s = space.bounded(space.relative(at, 0, size)?)?;
        if s.u32(at + 16)? as usize != d + 1 {
            return Err(Error::new(at + 16, "BlendSpace matrix dimension mismatch"));
        }
        s.take(at + 20, (d + 1) * (d + 1) * 4)?;
        let indices = s.relative(at, s.u32(at + 4)?, (d + 1) * 4)?.start;
        let scales = s.relative(at, s.u32(at + 8)?, (d + 1) * 4)?.start;
        let normals = s
            .relative(at, s.u32(at + 12)?, (d + 1) * (d + 1) * 4)?
            .start;
        let mut simplex = Simplex {
            children: Vec::new(),
            vertices: Vec::new(),
            normals: Vec::new(),
            scales: Vec::new(),
        };
        for i in 0..=d {
            let normal = normals + i * (d + 1) * 4;
            if s.u32(normal)? as usize != d {
                return Err(Error::new(normal, "BlendSpace normal dimension mismatch"));
            }
            let child = s.u32(indices + i * 4)?;
            if child as usize >= count {
                return Err(Error::new(indices, "BlendSpace child index out of range"));
            }
            simplex.children.push(child);
            simplex.scales.push(f(s, scales + i * 4)?);
            simplex.vertices.push(
                (0..d)
                    .map(|j| f(s, at + 20 + (i * (d + 1) + j) * 4))
                    .collect::<Result<_>>()?,
            );
            simplex.normals.push(
                (0..d)
                    .map(|j| f(s, normal + 4 + j * 4))
                    .collect::<Result<_>>()?,
            );
        }
        simplexes.push(simplex);
        at += size;
    }
    if simplexes.is_empty() || count < d + 1 {
        return Err(Error::new(p, "BlendSpace without simplexes"));
    }
    Ok(Tree::BlendSpace {
        parameters: (0..d)
            .map(|i| space.name(params + i * 20, 5))
            .collect::<Result<_>>()?,
        children: (0..count)
            .map(|i| r.name(names + i * 24, 6))
            .collect::<Result<_>>()?,
        simplexes,
    })
}

fn read_selection_space(r: Reader<'_>, h: &RecordHeader) -> Result<Tree> {
    let p = h.payload_offset;
    let count = r.u32(p)? as usize;
    if count > 10 {
        return Err(Error::new(p, "selection parameter array exceeds capacity"));
    }
    let parameters_at = r.relative(p, r.u32(p + 4)?, count * 40)?.start;
    let candidates_count = r.u32(p + 8)? as usize;
    let candidates_at = r.relative(p, r.u32(p + 12)?, 4)?.start;
    let stride = r.u32(candidates_at)? as usize;
    r.take(
        candidates_at,
        stride
            .checked_mul(candidates_count)
            .ok_or_else(|| Error::new(p, "selection candidate size overflow"))?,
    )?;
    if stride < 32 || candidates_count == 0 {
        return Err(Error::new(candidates_at, "selection candidates malformed"));
    }
    let parameters = (0..count)
        .map(|i| {
            let at = parameters_at + i * 40;
            Ok(SelectionParameter {
                name: r.name(at, 6)?,
                mode: r.u32(at + 24)?,
                weight: f32::from_bits(r.u32(at + 28)?),
                minimum: f32::from_bits(r.u32(at + 32)?),
                maximum: f32::from_bits(r.u32(at + 36)?),
            })
        })
        .collect::<Result<_>>()?;
    let candidates = (0..candidates_count)
        .map(|i| {
            let at = candidates_at + i * stride;
            let c = r.bounded(at..at + stride)?;
            let values = c.relative(at, c.u32(at + 28)?, count * 4)?.start;
            Ok((
                c.name(at + 4, 6)?,
                (0..count)
                    .map(|j| c.u32(values + j * 4).map(f32::from_bits))
                    .collect::<Result<_>>()?,
            ))
        })
        .collect::<Result<_>>()?;
    Ok(Tree::SelectionSpace {
        parameters,
        candidates,
    })
}
