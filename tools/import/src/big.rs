//! EA "EB" BIG v3 archives (Skate 3). Big-endian. Reads entries lazily through a `ReadAt` source, so a
//! BIG can be read straight out of the disc image.
//! Format as documented by SK8-ENGINE/skate-3-rust-engine (tools/owned_game/big.py, GPL-3.0).
//!
//! Header (48 bytes): u16 "EB", u16 version 3, u32 count @4, u16 flags @8, u8 align shift @10,
//! u32 index size @12, u32 names size @16, u8 name record size @20, u8 dir record size @21.
//! Entries @48: u32 offset >> shift, u32 stored, u32 unpacked (+4 more bytes if flags & 1).
//! A u8 compression code per entry follows at align16(entries). Name records (u16 dir index + name)
//! start at `index size`; directory names follow at align16.

use crate::refpack;
use std::io;

pub trait ReadAt {
    fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>>;
    fn len(&self) -> u64;
}

impl ReadAt for &[u8] {
    fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let o = offset as usize;
        self.get(o..o + len)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| io::ErrorKind::UnexpectedEof.into())
    }
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigEntry {
    pub path: String,
    pub offset: u64,
    pub stored: u32,
    pub unpacked: u32,
    pub compression: u8,
}

pub struct Big<S> {
    src: S,
    pub entries: Vec<BigEntry>,
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

const fn align16(n: usize) -> usize {
    (n + 15) & !15
}

struct Be<'a>(&'a [u8]);

impl Be<'_> {
    fn bytes(&self, at: usize, n: usize) -> io::Result<&[u8]> {
        self.0
            .get(at..at + n)
            .ok_or_else(|| bad(format!("BIG metadata truncated at {at:#x}")))
    }
    fn u8(&self, at: usize) -> io::Result<u8> {
        Ok(self.bytes(at, 1)?[0])
    }
    fn u16(&self, at: usize) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.bytes(at, 2)?.try_into().unwrap()))
    }
    fn u32(&self, at: usize) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.bytes(at, 4)?.try_into().unwrap()))
    }
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).replace('\\', "/")
}

pub fn is_big(head: &[u8]) -> bool {
    head.starts_with(&[0x45, 0x42, 0x00, 0x03])
}

impl<S: ReadAt> Big<S> {
    pub fn new(mut src: S) -> io::Result<Self> {
        let total = src.len();
        let head = src.read_at(0, 48)?;
        if !is_big(&head) {
            return Err(bad("not an EB BIG v3 archive"));
        }
        let h = Be(&head);
        let meta_len = h.u32(12)? as u64 + h.u32(16)? as u64;
        if meta_len > total {
            return Err(bad("BIG index runs past the archive"));
        }
        let meta = src.read_at(0, meta_len as usize)?;
        let m = Be(&meta);
        let count = m.u32(4)? as usize;
        if count > 1_000_000 {
            return Err(bad(format!("unreasonable BIG entry count {count}")));
        }
        let flags = m.u16(8)?;
        let shift = m.u8(10)?;
        let index_size = m.u32(12)? as usize;
        let name_rec = m.u8(20)? as usize;
        let dir_rec = m.u8(21)? as usize;
        if shift > 31 || name_rec < 3 || dir_rec < 2 {
            return Err(bad("invalid BIG header fields"));
        }
        let entry_size = if flags & 1 != 0 { 20 } else { 16 };
        let comp_start = 48 + align16(entry_size * count);
        let dirs_start = index_size + align16(name_rec * count);
        let mut dirs = Vec::new();
        let mut at = dirs_start;
        while at + dir_rec <= meta.len() {
            dirs.push(cstr(m.bytes(at, dir_rec)?));
            at += dir_rec;
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let e = 48 + i * entry_size;
            let offset = (m.u32(e)? as u64) << shift;
            let declared = m.u32(e + 4)?;
            let unpacked = match m.u32(e + 8)? {
                0 => declared,
                n => n,
            };
            let stored = if declared == 0 { unpacked } else { declared };
            let rec = index_size + i * name_rec;
            let dir = dirs
                .get(m.u16(rec)? as usize)
                .map(String::as_str)
                .unwrap_or(".");
            let name = cstr(m.bytes(rec + 2, name_rec - 2)?);
            let path = if dir.is_empty() || dir == "." {
                name
            } else {
                format!("{dir}/{name}")
            };
            if offset + stored as u64 > total {
                return Err(bad(format!("{path}: payload runs past the archive")));
            }
            entries.push(BigEntry {
                path,
                offset,
                stored,
                unpacked,
                compression: m.u8(comp_start + i)?,
            });
        }
        Ok(Big { src, entries })
    }

    pub fn find(&self, path: &str) -> Option<&BigEntry> {
        self.entries
            .iter()
            .find(|e| e.path.eq_ignore_ascii_case(path))
    }

    pub fn read(&mut self, e: &BigEntry) -> io::Result<Vec<u8>> {
        let packed = self.src.read_at(e.offset, e.stored as usize)?;
        let out = match e.compression {
            0 => packed,
            1 => refpack::decompress(&packed, Some(e.unpacked as usize))
                .map_err(|x| bad(format!("{}: {x}", e.path)))?,
            2..=4 => chunkref(&packed, e.unpacked as usize)
                .map_err(|x| bad(format!("{}: {x}", e.path)))?,
            c => return Err(bad(format!("{}: unsupported compression {c}", e.path))),
        };
        if out.len() != e.unpacked as usize {
            return Err(bad(format!(
                "{}: decoded {} bytes, expected {}",
                e.path,
                out.len(),
                e.unpacked
            )));
        }
        Ok(out)
    }
}

/// `chunkref` v2 container: total/chunk size/count/alignment, then (u32 packed size, u32 method) + payload
/// per chunk, each payload aligned. Methods: 0/4 stored, 2 RefPack, 3 zlib.
pub fn chunkref(data: &[u8], expected: usize) -> io::Result<Vec<u8>> {
    let b = Be(data);
    if b.bytes(0, 8)? != b"chunkref" || b.u32(8)? != 2 {
        return Err(bad("invalid chunkref header"));
    }
    let (total, chunk, count, align) = (
        b.u32(12)? as usize,
        b.u32(16)? as usize,
        b.u32(20)?,
        b.u32(24)? as usize,
    );
    if total != expected || chunk == 0 || count == 0 || !align.is_power_of_two() || align > 4096 {
        return Err(bad("invalid chunkref dimensions"));
    }
    let mut out = Vec::with_capacity(total);
    let mut cur = 28;
    for _ in 0..count {
        let payload = (cur + 8 + align - 1) & !(align - 1);
        let (size, method) = (b.u32(payload - 8)? as usize, b.u32(payload - 4)?);
        let packed = b.bytes(payload, size)?;
        let want = chunk.min(total - out.len());
        let decoded = match method {
            0 | 4 => packed.to_vec(),
            2 => refpack::decompress(packed, Some(want)).map_err(|e| bad(e.to_string()))?,
            3 => miniz_oxide::inflate::decompress_to_vec_zlib(packed)
                .map_err(|e| bad(format!("chunkref zlib: {e:?}")))?,
            m => return Err(bad(format!("unsupported chunkref method {m}"))),
        };
        if decoded.len() != want {
            return Err(bad(format!(
                "chunkref chunk decoded to {} bytes, expected {want}",
                decoded.len()
            )));
        }
        out.extend_from_slice(&decoded);
        cur = payload + size;
    }
    if out.len() != total {
        return Err(bad("chunkref output size mismatch"));
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Builds a synthetic BIG v3 holding `files` (path, compression code, stored bytes, unpacked size).
    pub fn build(files: &[(&str, &str, u8, Vec<u8>, u32)]) -> Vec<u8> {
        let n = files.len();
        let (name_rec, dir_rec) = (32usize, 32usize);
        let mut dirs: Vec<&str> = Vec::new();
        for f in files {
            if !dirs.contains(&f.0) {
                dirs.push(f.0);
            }
        }
        let comp_start = 48 + align16(16 * n);
        let index_size = align16(comp_start + n);
        let names_size = align16(name_rec * n) + dir_rec * dirs.len();
        let mut out = vec![0u8; index_size + names_size];
        out[0..4].copy_from_slice(&[0x45, 0x42, 0, 3]);
        out[4..8].copy_from_slice(&(n as u32).to_be_bytes());
        out[10] = 4; // 16-byte alignment
        out[12..16].copy_from_slice(&(index_size as u32).to_be_bytes());
        out[16..20].copy_from_slice(&(names_size as u32).to_be_bytes());
        out[20] = name_rec as u8;
        out[21] = dir_rec as u8;
        for (i, d) in dirs.iter().enumerate() {
            let at = index_size + align16(name_rec * n) + i * dir_rec;
            out[at..at + d.len()].copy_from_slice(d.as_bytes());
        }
        for (i, (dir, name, comp, data, unpacked)) in files.iter().enumerate() {
            while !out.len().is_multiple_of(16) {
                out.push(0);
            }
            let off = out.len();
            out.extend_from_slice(data);
            let e = 48 + i * 16;
            out[e..e + 4].copy_from_slice(&((off >> 4) as u32).to_be_bytes());
            out[e + 4..e + 8].copy_from_slice(&(data.len() as u32).to_be_bytes());
            out[e + 8..e + 12].copy_from_slice(&unpacked.to_be_bytes());
            out[comp_start + i] = *comp;
            let r = index_size + i * name_rec;
            let di = dirs.iter().position(|d| d == dir).unwrap() as u16;
            out[r..r + 2].copy_from_slice(&di.to_be_bytes());
            out[r + 2..r + 2 + name.len()].copy_from_slice(name.as_bytes());
        }
        out
    }

    fn chunkref_of(parts: &[(u32, Vec<u8>)], total: u32, chunk: u32) -> Vec<u8> {
        let mut v = b"chunkref".to_vec();
        for x in [2, total, chunk, parts.len() as u32, 16] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        for (method, data) in parts {
            while !(v.len() + 8).is_multiple_of(16) {
                v.push(0);
            }
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(&method.to_be_bytes());
            v.extend_from_slice(data);
        }
        v
    }

    #[test]
    fn reads_raw_refpack_and_chunkref_entries() {
        let rp = vec![
            0x10, 0xFB, 0, 0, 8, 0xE0, b'w', b'x', b'y', b'z', 0x80, 0, 3, 0xFC,
        ];
        let zl = miniz_oxide::deflate::compress_to_vec_zlib(b"5678", 6);
        let cr = chunkref_of(&[(0, b"1234".to_vec()), (3, zl)], 8, 4);
        let img = build(&[
            ("data/a", "plain.txt", 0, b"hello".to_vec(), 5),
            ("data/b", "packed.bin", 1, rp, 8),
            ("data/a", "chunks.bin", 2, cr, 8),
        ]);
        let mut big = Big::new(img.as_slice()).unwrap();
        let paths: Vec<_> = big.entries.iter().map(|e| e.path.clone()).collect();
        assert_eq!(
            paths,
            ["data/a/plain.txt", "data/b/packed.bin", "data/a/chunks.bin"]
        );
        let e = big.find("DATA/A/PLAIN.TXT").unwrap().clone();
        assert_eq!(big.read(&e).unwrap(), b"hello");
        let e = big.entries[1].clone();
        assert_eq!(big.read(&e).unwrap(), b"wxyzwxyz");
        let e = big.entries[2].clone();
        assert_eq!(big.read(&e).unwrap(), b"12345678");
    }

    #[test]
    fn rejects_other_files() {
        assert!(
            Big::new(&b"\x89RW4xb2\0 not a big archive at all, padding padding padding"[..])
                .is_err()
        );
    }
}
