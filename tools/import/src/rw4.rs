//! RenderWare 4 arenas for Xbox 360 (`.rx2`, magic `\x89RW4xb2\0\r\n\x1a\n`). Big-endian.
//! Layout as documented by SK8-ENGINE/skate-3-rust-engine (tools/vendor/utt/rx2_parser.py, GPL-3.0).
//!
//! Header: section count @0x20, section table @0x30, data base @0x44 (GPU arena start), arena type @0x58.
//! The table holds 24-byte records `(i32 f0, f1, f2, f3, f4, u32 type)`. Records pair up: for a typed record
//! N, its info block is at N.f0 (absolute), and its payload is at `data base + (N-1).f0`, `(N-1).f2` bytes.

use std::io;

pub const MAGIC: &[u8; 7] = b"\x89RW4xb2";

pub const T_TEXTURE: u32 = 0x0002_00E8;
pub const T_VERTEX_DESC: u32 = 0x0002_00E9;
pub const T_VERTEX_BUFFER: u32 = 0x0002_00EA;
pub const T_INDEX_BUFFER: u32 = 0x0002_00EB;
pub const T_MATERIALS: u32 = 0x00EB_0005;
pub const T_MODEL_DATA: u32 = 0x00EB_0001;

pub fn type_name(t: u32) -> &'static str {
    match t {
        T_TEXTURE => "texture",
        T_VERTEX_DESC => "vertex descriptor",
        T_VERTEX_BUFFER => "vertex buffer",
        T_INDEX_BUFFER => "index buffer",
        T_MATERIALS => "materials",
        T_MODEL_DATA => "model data",
        0x00EB_000B => "external refs",
        0x00EB_0023 => "skin descriptor",
        0x0002_0081 => "decl summary",
        0x0008_0006 => "collision (clustered mesh)",
        0x0001_0031 => "raw buffer",
        _ => "?",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub index: usize,
    pub type_id: u32,
    /// Absolute offset of this section's info block.
    pub info: usize,
    /// Absolute offset and size of the payload (from the preceding record), if any.
    pub data: Option<(usize, usize)>,
    pub size: usize,
}

pub struct Arena<'a> {
    pub bytes: &'a [u8],
    pub arena_type: u32,
    pub data_base: usize,
    pub sections: Vec<Section>,
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

pub fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

pub fn is_rw4(head: &[u8]) -> bool {
    head.starts_with(MAGIC)
}

impl<'a> Arena<'a> {
    pub fn parse(bytes: &'a [u8]) -> io::Result<Self> {
        if bytes.len() < 0x5C || !is_rw4(bytes) {
            return Err(bad("not an Xbox 360 RW4 arena"));
        }
        let count = be32(bytes, 0x20) as usize;
        let table = be32(bytes, 0x30) as usize;
        let data_base = be32(bytes, 0x44) as usize;
        if count > 1 << 20 || table + count * 24 > bytes.len() {
            return Err(bad("RW4 section table runs past the file"));
        }
        let mut sections = Vec::with_capacity(count);
        let mut prev: Option<(u32, u32)> = None;
        for i in 0..count {
            let r = table + i * 24;
            let (f0, f2) = (be32(bytes, r), be32(bytes, r + 8));
            let data = prev.and_then(|(o, n)| {
                let start = data_base.checked_add(o as usize)?;
                (start + n as usize <= bytes.len()).then_some((start, n as usize))
            });
            sections.push(Section {
                index: i,
                type_id: be32(bytes, r + 20),
                info: f0 as usize,
                data,
                size: f2 as usize,
            });
            prev = Some((f0, f2));
        }
        Ok(Arena {
            bytes,
            arena_type: be32(bytes, 0x58),
            data_base,
            sections,
        })
    }

    pub fn of_type(&self, t: u32) -> impl Iterator<Item = &Section> {
        self.sections.iter().filter(move |s| s.type_id == t)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Synthetic arena: one texture section whose info block and payload are `info` and `payload`.
    pub fn build(info: &[u8], payload: &[u8]) -> Vec<u8> {
        let table = 0x60usize;
        let info_at = table + 2 * 24;
        let data_base = (info_at + info.len() + 15) & !15;
        let mut b = vec![0u8; data_base];
        b[..7].copy_from_slice(MAGIC);
        b[0x20..0x24].copy_from_slice(&2u32.to_be_bytes());
        b[0x30..0x34].copy_from_slice(&(table as u32).to_be_bytes());
        b[0x44..0x48].copy_from_slice(&(data_base as u32).to_be_bytes());
        // record 0: raw buffer, payload at data base + 0
        b[table + 8..table + 12].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        b[table + 20..table + 24].copy_from_slice(&0x0001_0031u32.to_be_bytes());
        // record 1: texture, info block
        let r = table + 24;
        b[r..r + 4].copy_from_slice(&(info_at as u32).to_be_bytes());
        b[r + 20..r + 24].copy_from_slice(&T_TEXTURE.to_be_bytes());
        b[info_at..info_at + info.len()].copy_from_slice(info);
        b.extend_from_slice(payload);
        b
    }

    #[test]
    fn pairs_records() {
        let b = build(&[7u8; 40], &[1, 2, 3, 4]);
        let a = Arena::parse(&b).unwrap();
        let t: Vec<_> = a.of_type(T_TEXTURE).collect();
        assert_eq!(t.len(), 1);
        let (at, n) = t[0].data.unwrap();
        assert_eq!(&b[at..at + n], &[1, 2, 3, 4]);
        assert_eq!(b[t[0].info], 7);
        assert!(Arena::parse(b"EB\0\x03 not rw4 padding padding padding padding padding padding padding padding padding").is_err());
    }
}
