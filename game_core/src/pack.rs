//! `.fgpak`: flatground's asset pack, read in place (no copies) on the Wii. All integers big-endian.
//!
//! Packs are made on the PC by `tools/import` from data the player supplies; none ship with the game.
//! When no pack is present the game uses its built-in placeholder art.
//!
//! ```text
//! 0x00  magic "FGPA", u32 version (1), u32 entry count, u32 string table offset
//! 0x10  entries, 32 bytes each:
//!       u32 name hash (FNV-1a 32 of the lower-case name), u32 name offset (into the string table),
//!       u32 kind, u32 data offset (32-byte aligned, from file start), u32 data size,
//!       u16 width, u16 height, u8 GX texture format, u8 reserved[3], u32 reserved
//! ....  string table: NUL-terminated names
//! ....  data blobs, each 32-byte aligned (GX texture DMA requirement)
//! ```

pub const MAGIC: [u8; 4] = *b"FGPA";
pub const VERSION: u32 = 1;
pub const HEADER_SIZE: usize = 16;
pub const ENTRY_SIZE: usize = 32;
pub const DATA_ALIGN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
    Texture = 1,
    /// Opaque bytes; the entry name says what they are (e.g. `xex/ocean_pca`).
    Data = 2,
    /// `game_core::anim::Skeleton`
    Skeleton = 3,
    /// `game_core::anim::Clip`
    Animation = 4,
    /// `game_core::skinned::SkinnedMesh`
    SkinnedMesh = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    BadMagic,
    BadVersion(u32),
    Truncated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    pub name: &'a str,
    pub kind: u32,
    pub data: &'a [u8],
    pub width: u16,
    pub height: u16,
    pub gx_format: u8,
}

pub struct Pack<'a> {
    bytes: &'a [u8],
    count: usize,
    strings: usize,
}

/// FNV-1a 32 of the ASCII-lower-cased name.
pub fn name_hash(name: &str) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in name.bytes() {
        h ^= b.to_ascii_lowercase() as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

impl<'a> Pack<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.get(0..4) != Some(&MAGIC[..]) {
            return Err(Error::BadMagic);
        }
        let version = be32(bytes, 4).ok_or(Error::Truncated)?;
        if version != VERSION {
            return Err(Error::BadVersion(version));
        }
        let count = be32(bytes, 8).ok_or(Error::Truncated)? as usize;
        let strings = be32(bytes, 12).ok_or(Error::Truncated)? as usize;
        if count
            .checked_mul(ENTRY_SIZE)
            .and_then(|n| n.checked_add(HEADER_SIZE))
            .is_none_or(|end| end > bytes.len())
            || strings > bytes.len()
        {
            return Err(Error::Truncated);
        }
        let pack = Pack {
            bytes,
            count,
            strings,
        };
        for i in 0..count {
            pack.entry(i)?;
        }
        Ok(pack)
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn entry(&self, i: usize) -> Result<Entry<'a>, Error> {
        let b = self.bytes;
        let e = HEADER_SIZE + i * ENTRY_SIZE;
        let name_at = self.strings + be32(b, e + 4).ok_or(Error::Truncated)? as usize;
        let tail = b.get(name_at..).ok_or(Error::Truncated)?;
        let len = tail.iter().position(|&c| c == 0).ok_or(Error::Truncated)?;
        let name = core::str::from_utf8(&tail[..len]).map_err(|_| Error::Truncated)?;
        let (off, size) = (
            be32(b, e + 12).ok_or(Error::Truncated)? as usize,
            be32(b, e + 16).ok_or(Error::Truncated)? as usize,
        );
        let data = b
            .get(off..off.checked_add(size).ok_or(Error::Truncated)?)
            .ok_or(Error::Truncated)?;
        Ok(Entry {
            name,
            kind: be32(b, e + 8).ok_or(Error::Truncated)?,
            data,
            width: u16::from_be_bytes([b[e + 20], b[e + 21]]),
            height: u16::from_be_bytes([b[e + 22], b[e + 23]]),
            gx_format: b[e + 24],
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = Entry<'a>> + '_ {
        (0..self.count).filter_map(|i| self.entry(i).ok())
    }

    /// Finds an entry by name (case-insensitive) via its hash.
    pub fn find(&self, name: &str) -> Option<Entry<'a>> {
        let h = name_hash(name);
        (0..self.count)
            .filter(|&i| be32(self.bytes, HEADER_SIZE + i * ENTRY_SIZE) == Some(h))
            .filter_map(|i| self.entry(i).ok())
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn hash_is_case_insensitive_fnv1a() {
        assert_eq!(name_hash(""), 0x811C_9DC5);
        assert_eq!(name_hash("a"), 0xE40C_292C);
        assert_eq!(name_hash("Board/Grip"), name_hash("board/grip"));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Pack::parse(b"nope").err(), Some(Error::BadMagic));
        let mut v: Vec<u8> = b"FGPA".to_vec();
        v.extend_from_slice(&2u32.to_be_bytes());
        assert_eq!(Pack::parse(&v).err(), Some(Error::BadVersion(2)));
        let mut v: Vec<u8> = b"FGPA".to_vec();
        v.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 9, 0, 0, 0, 16]);
        assert_eq!(Pack::parse(&v).err(), Some(Error::Truncated));
    }
}
