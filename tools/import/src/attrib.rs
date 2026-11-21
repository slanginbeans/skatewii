//! Skate 3's tuning database: EA's AttribSys "vault" pair (`.vlt` + `.bin`, big-endian, 64-bit), as in
//! `data/db/skaterschema` (classes and their fields) and `data/db/skatercollections` (the values).
//!
//! Layout from SK8-ENGINE/skate-3-rust-engine's `tools/asset_pipeline/vlt.py` (GPL-3.0), which credits
//! NFSTools/VaultLib (MIT). Names are 64-bit hashes ([`hash`]): we look values up by hashing the class,
//! collection and field names we want, so no name list is needed.

use crate::Res;
use std::collections::HashMap;

/// AttribSys name hash (Bob Jenkins' lookup8, as the game uses it).
pub fn hash(text: &str) -> u64 {
    fn mix(mut a: u64, mut b: u64, mut c: u64) -> (u64, u64, u64) {
        for (x, y, z) in [(43, 9, 8), (38, 23, 5), (35, 49, 11), (12, 18, 22)] {
            a = a.wrapping_sub(b).wrapping_sub(c) ^ (c >> x);
            b = b.wrapping_sub(c).wrapping_sub(a) ^ (a << y);
            c = c.wrapping_sub(a).wrapping_sub(b) ^ (b >> z);
        }
        (a, b, c)
    }
    if text.is_empty() {
        return 0;
    }
    let (mut a, mut b, mut c) = (
        0xabcd_ef00_1122_3344u64,
        0xabcd_ef00_1122_3344u64,
        0x9e37_79b9_7f4a_7c13u64,
    );
    let mut chunks = text.as_bytes().chunks_exact(24);
    for ch in &mut chunks {
        let w = |i: usize| u64::from_le_bytes(ch[i..i + 8].try_into().unwrap());
        (a, b, c) = mix(
            a.wrapping_add(w(0)),
            b.wrapping_add(w(8)),
            c.wrapping_add(w(16)),
        );
    }
    c = c.wrapping_add(text.len() as u64);
    for (i, &byte) in chunks.remainder().iter().enumerate() {
        let v = u64::from(byte);
        match i {
            0..=7 => a = a.wrapping_add(v << (i * 8)),
            8..=15 => b = b.wrapping_add(v << ((i - 8) * 8)),
            _ => c = c.wrapping_add(v << ((i - 15) * 8)),
        }
    }
    mix(a, b, c).2
}

/// A name or a literal `Hash_XXXXXXXXXXXXXXXX` (for fields the database stores without a name).
pub fn name_hash(name: &str) -> u64 {
    name.strip_prefix("Hash_")
        .and_then(|h| u64::from_str_radix(h, 16).ok())
        .unwrap_or_else(|| hash(name))
}

fn be<const N: usize>(d: &[u8], at: usize) -> Res<[u8; N]> {
    Ok(d.get(at..at + N)
        .ok_or_else(|| format!("tuning database truncated at {at:#x}"))?
        .try_into()
        .unwrap())
}
fn u16_at(d: &[u8], at: usize) -> Res<u16> {
    Ok(u16::from_be_bytes(be(d, at)?))
}
fn u32_at(d: &[u8], at: usize) -> Res<u32> {
    Ok(u32::from_be_bytes(be(d, at)?))
}
fn u64_at(d: &[u8], at: usize) -> Res<u64> {
    Ok(u64::from_be_bytes(be(d, at)?))
}

/// One vault: its `.vlt` and `.bin` with pointers fixed up, and its exports (kind, offset in the vlt).
struct Vault {
    vlt: Vec<u8>,
    bin: Vec<u8>,
    exports: Vec<(u64, usize)>,
}

const TAG_PTRN: u32 = 0x5074_724e;
const TAG_EXPN: u32 = 0x4578_704e;
const TAG_ENDC: u32 = 0x456e_6443;
const KIND_CLASS: u64 = 0x2A78_95AC_4A87_6152;
const KIND_COLLECTION: u64 = 0xAD30_3B8F_42B3_307E;

impl Vault {
    fn parse(mut vlt: Vec<u8>, mut bin: Vec<u8>) -> Res<Vault> {
        let mut exports = Vec::new();
        let mut at = 0;
        while at < vlt.len() {
            let tag = u32_at(&vlt, at)?;
            let size = u32_at(&vlt, at + 4)? as usize;
            if size < 8 || at + size > vlt.len() {
                return Err("bad tuning database chunk".into());
            }
            let body = at + 8;
            match tag {
                TAG_PTRN => {
                    // Pointer fix-ups: 16-byte records (offset, kind, index, destination).
                    let mut into_vlt = false;
                    for p in (body..at + size).step_by(16) {
                        let offset = u32_at(&vlt, p)? as usize;
                        let kind = u16_at(&vlt, p + 4)?;
                        let index = u16_at(&vlt, p + 6)?;
                        let dest = u64_at(&vlt, p + 8)?;
                        match kind {
                            0 => break,
                            2 => into_vlt = index == 0,
                            1 | 3 => {
                                let target = if into_vlt { &mut vlt } else { &mut bin };
                                let v = if kind == 1 { 0 } else { dest as u32 };
                                target
                                    .get_mut(offset..offset + 4)
                                    .ok_or("tuning database pointer out of range")?
                                    .copy_from_slice(&v.to_be_bytes());
                            }
                            4 => {}
                            k => return Err(format!("unknown tuning pointer kind {k}").into()),
                        }
                    }
                }
                TAG_EXPN => {
                    let count = u64_at(&vlt, body)? as usize;
                    if count > (size - 8) / 24 {
                        return Err("bad tuning export count".into());
                    }
                    for i in 0..count {
                        let e = body + 8 + i * 24;
                        exports.push((u64_at(&vlt, e + 8)?, u32_at(&vlt, e + 20)? as usize));
                    }
                }
                _ => {}
            }
            at += size;
            if tag == TAG_ENDC {
                break;
            }
        }
        Ok(Vault { vlt, bin, exports })
    }
}

/// A field definition: where its value sits and how big it is.
#[derive(Clone, Copy, Debug)]
struct FieldDef {
    offset: usize,
    size: usize,
    flags: u8,
}

/// The values of one collection, by field hash (raw big-endian bytes), and its parent.
#[derive(Debug, Default)]
struct Collection {
    parent: u64,
    fields: HashMap<u64, Vec<u8>>,
}

pub struct Database {
    /// (class hash, collection hash) -> collection.
    collections: HashMap<(u64, u64), Collection>,
}

impl Database {
    /// From the four files: schema `.vlt`/`.bin`, collections `.vlt`/`.bin`.
    pub fn parse(schema: (Vec<u8>, Vec<u8>), values: (Vec<u8>, Vec<u8>)) -> Res<Database> {
        let s = Vault::parse(schema.0, schema.1)?;
        let mut classes: HashMap<u64, (HashMap<u64, FieldDef>, usize)> = HashMap::new();
        for &(kind, at) in &s.exports {
            if kind != KIND_CLASS {
                continue;
            }
            let key = u64_at(&s.vlt, at)?;
            let count = u32_at(&s.vlt, at + 12)? as usize;
            let defs = u32_at(&s.vlt, at + 16)? as usize;
            let mut fields = HashMap::new();
            for i in 0..count {
                let d = defs + i * 24;
                let fkey = u64_at(&s.bin, d)?;
                fields.insert(
                    fkey,
                    FieldDef {
                        offset: u16_at(&s.bin, d + 16)? as usize,
                        size: u16_at(&s.bin, d + 18)? as usize,
                        flags: *s.bin.get(d + 22).ok_or("truncated field")?,
                    },
                );
            }
            classes.insert(key, (fields, 0));
        }
        let c = Vault::parse(values.0, values.1)?;
        let mut collections = HashMap::new();
        for &(kind, at) in &c.exports {
            if kind != KIND_COLLECTION {
                continue;
            }
            let key = u64_at(&c.vlt, at)?;
            let class = u64_at(&c.vlt, at + 8)?;
            let parent = u64_at(&c.vlt, at + 16)?;
            let count = u32_at(&c.vlt, at + 32)? as usize;
            let types_len = u16_at(&c.vlt, at + 38)? as usize;
            let layout = u32_at(&c.vlt, at + 40)? as usize;
            let Some((fields, _)) = classes.get(&class) else {
                continue;
            };
            let mut out = Collection {
                parent,
                fields: HashMap::new(),
            };
            // Fields stored in the collection's fixed layout block.
            if layout != 0 {
                for (k, f) in fields {
                    if f.flags & 2 != 0
                        && let Some(b) = c.bin.get(layout + f.offset..layout + f.offset + f.size)
                    {
                        out.fields.insert(*k, b.to_vec());
                    }
                }
            }
            // Fields set by this collection: inline when small, else pointed to in the bin.
            let entries = at + 48 + types_len * 8;
            for i in 0..count {
                let e = entries + i * 16;
                let fkey = u64_at(&c.vlt, e)?;
                let ptr = u32_at(&c.vlt, e + 8)? as usize;
                let Some(f) = fields.get(&fkey) else {
                    continue;
                };
                let bytes = if f.size <= 4 && f.flags & 1 == 0 {
                    c.vlt
                        .get(e + 8..e + 12)
                        .map(|b| b[..f.size.clamp(1, 4)].to_vec())
                } else {
                    c.bin.get(ptr..ptr + f.size).map(<[u8]>::to_vec)
                };
                if let Some(b) = bytes {
                    out.fields.insert(fkey, b);
                }
            }
            collections.insert((class, key), out);
        }
        Ok(Database { collections })
    }

    /// The raw value of `class/collection/field`, looked up through the collection's parents.
    pub fn raw(&self, class: &str, collection: &str, field: &str) -> Option<&[u8]> {
        let (class, field) = (name_hash(class), name_hash(field));
        let mut key = name_hash(collection);
        for _ in 0..16 {
            let c = self.collections.get(&(class, key))?;
            if let Some(v) = c.fields.get(&field) {
                return Some(v);
            }
            if c.parent == 0 {
                return None;
            }
            key = c.parent;
        }
        None
    }

    /// A value as floats (see [`decode_values`]).
    pub fn values(&self, path: &str) -> Option<Vec<f32>> {
        let mut it = path.splitn(3, '/');
        let (class, coll, field) = (it.next()?, it.next()?, it.next()?);
        decode_values(self.raw(class, coll, field)?)
    }
}

/// A raw field as floats: a float scalar, a small integer (one byte), a 4-float vector or controller (16
/// bytes; used as a scalar, its first value counts), or a point graph as its x values then its y values
/// (8 points: 64 bytes; 16 points: 128 bytes; with a 4-float header: 4 points in 48 bytes, 8 in 80, 16
/// in 144).
pub fn decode_values(raw: &[u8]) -> Option<Vec<f32>> {
    let floats = |b: &[u8]| -> Vec<f32> {
        b.chunks_exact(4)
            .map(|w| f32::from_be_bytes(w.try_into().unwrap()))
            .collect()
    };
    let v = match raw.len() {
        1 => vec![raw[0] as f32],
        4 | 16 | 64 | 128 => floats(raw),
        48 | 80 | 144 => floats(&raw[16..]),
        _ => return None,
    };
    v.iter().all(|f| f.is_finite()).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_hashes() {
        assert_eq!(hash(""), 0);
        // Literal hashes pass straight through; names hash.
        assert_eq!(name_hash("Hash_4F08D9BAE6831524"), 0x4F08_D9BA_E683_1524);
        assert_ne!(hash("physics_push"), hash("physics_jump"));
        // Long names go through the 24-byte block path.
        assert_ne!(
            hash("NumTicksPatternNotInRangeBeforeCulling"),
            hash("NumTicksPatternNotInRangeBeforeCullinG")
        );
    }

    /// A tiny synthetic vault pair: one class with a float field and a 64-byte curve, a parent
    /// collection that sets both and a child that overrides the float.
    fn synthetic() -> Database {
        let chunk = |tag: u32, body: &[u8]| {
            let mut c = Vec::new();
            c.extend_from_slice(&tag.to_be_bytes());
            c.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
            c.extend_from_slice(body);
            c
        };
        let exports = |list: &[(u64, u32)]| {
            let mut b = (list.len() as u64).to_be_bytes().to_vec();
            for (kind, at) in list {
                b.extend_from_slice(&0u64.to_be_bytes());
                b.extend_from_slice(&kind.to_be_bytes());
                b.extend_from_slice(&0u32.to_be_bytes());
                b.extend_from_slice(&at.to_be_bytes());
            }
            b
        };
        let (cls, f_speed, f_curve) = (hash("physics_test"), hash("Speed"), hash("Curve"));
        // Schema: class record at vlt offset 0x100 (after the chunks); field defs at bin 0.
        let mut sbin = Vec::new();
        for (k, size) in [(f_speed, 4u16), (f_curve, 64)] {
            sbin.extend_from_slice(&k.to_be_bytes());
            sbin.extend_from_slice(&0u64.to_be_bytes()); // type
            sbin.extend_from_slice(&0u16.to_be_bytes()); // offset
            sbin.extend_from_slice(&size.to_be_bytes());
            sbin.extend_from_slice(&0u16.to_be_bytes()); // max
            sbin.extend_from_slice(&[0, 0]); // flags, alignment
        }
        let mut svlt = chunk(TAG_EXPN, &exports(&[(KIND_CLASS, 0x100)]));
        svlt.extend(chunk(TAG_ENDC, &[]));
        svlt.resize(0x100, 0);
        svlt.extend_from_slice(&cls.to_be_bytes());
        for v in [0u32, 2, 0, 0, 0, 0] {
            svlt.extend_from_slice(&v.to_be_bytes());
        }
        // Patch the defs offset (word 3 after the key) to 0.
        // Values: parent "default" at 0x100, child "normal" at 0x200.
        let curve: Vec<u8> = (0..16).flat_map(|i| (i as f32).to_be_bytes()).collect();
        let mut cbin = vec![0u8; 16];
        cbin.extend_from_slice(&curve);
        let record = |key: u64, parent: u64, entries: &[(u64, [u8; 4])]| {
            let mut r = Vec::new();
            r.extend_from_slice(&key.to_be_bytes());
            r.extend_from_slice(&cls.to_be_bytes());
            r.extend_from_slice(&parent.to_be_bytes());
            r.extend_from_slice(&[0; 8]);
            r.extend_from_slice(&(entries.len() as u32).to_be_bytes());
            r.extend_from_slice(&[0, 0, 0, 0]); // ntypes, typeslen
            r.extend_from_slice(&[0; 8]); // layout, pad
            for (k, v) in entries {
                r.extend_from_slice(&k.to_be_bytes());
                r.extend_from_slice(v);
                r.extend_from_slice(&[0; 4]);
            }
            r
        };
        let mut cvlt = chunk(
            TAG_EXPN,
            &exports(&[(KIND_COLLECTION, 0x100), (KIND_COLLECTION, 0x200)]),
        );
        cvlt.extend(chunk(TAG_ENDC, &[]));
        cvlt.resize(0x100, 0);
        cvlt.extend(record(
            hash("default"),
            0,
            &[
                (f_speed, 2.5f32.to_be_bytes()),
                (f_curve, 16u32.to_be_bytes()),
            ],
        ));
        cvlt.resize(0x200, 0);
        cvlt.extend(record(
            hash("normal"),
            hash("default"),
            &[(f_speed, 7.0f32.to_be_bytes())],
        ));
        Database::parse((svlt, sbin), (cvlt, cbin)).unwrap()
    }

    #[test]
    fn decodes_every_value_size() {
        let be = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|f| f.to_be_bytes()).collect() };
        assert_eq!(decode_values(&[3]), Some(vec![3.0]));
        assert_eq!(decode_values(&be(&[0.5])), Some(vec![0.5]));
        assert_eq!(
            decode_values(&be(&[0.25, 0.5, 0.75, 1.0])).unwrap()[0],
            0.25
        );
        // A 4-point graph after a 4-float header: x values then y values.
        let mut g = be(&[9.0; 4]);
        g.extend(be(&[0.0, 1.0, 2.0, 3.0, 5.0, 6.0, 7.0, 8.0]));
        assert_eq!(
            decode_values(&g).unwrap(),
            vec![0.0, 1.0, 2.0, 3.0, 5.0, 6.0, 7.0, 8.0]
        );
        // A 16-point graph after a 4-float header.
        let mut g = be(&[9.0; 4]);
        g.extend(be(&(0..32).map(|i| i as f32).collect::<Vec<_>>()));
        let v = decode_values(&g).unwrap();
        assert_eq!((v.len(), v[0], v[31]), (32, 0.0, 31.0));
        assert_eq!(decode_values(&[0; 12]), None);
        assert_eq!(decode_values(&be(&[f32::NAN])), None);
    }

    #[test]
    fn reads_values_through_parents() {
        let db = synthetic();
        assert_eq!(db.values("physics_test/normal/Speed"), Some(vec![7.0]));
        assert_eq!(db.values("physics_test/default/Speed"), Some(vec![2.5]));
        let c = db.values("physics_test/normal/Curve").unwrap();
        assert_eq!(c.len(), 16);
        assert_eq!((c[0], c[8], c[15]), (0.0, 8.0, 15.0));
        assert_eq!(db.values("physics_test/normal/Missing"), None);
        assert_eq!(db.values("nothing/normal/Speed"), None);
    }
}
