//! Read-only access to an Xbox / Xbox 360 disc image (XDVDFS / GDFX file system).
//!
//! The game partition starts at one of a few fixed offsets depending on the disc generation. Its volume
//! descriptor sits at partition + 32 sectors and points at the root directory table. A directory table is
//! a binary tree of entries whose `left`/`right` links are offsets in 4-byte units; 0xFF bytes are padding.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub const SECTOR: u64 = 2048;
const MAGIC: &[u8; 20] = b"MICROSOFT*XBOX*MEDIA";
/// Plain XDVDFS image, then the game-partition offsets of full XGD3, XGD2 and XGD1 images.
const PARTITION_OFFSETS: [u64; 4] = [0, 0x0208_0000, 0x0FD9_0000, 0x1830_0000];
const ATTR_DIRECTORY: u8 = 0x10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub sector: u32,
    pub size: u32,
    pub is_dir: bool,
}

pub struct Disc<R> {
    r: R,
    base: u64,
    root: Entry,
}

impl Disc<File> {
    pub fn open(path: &Path) -> io::Result<Self> {
        Disc::new(File::open(path)?)
    }
}

fn bad(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

impl<R: Read + Seek> Disc<R> {
    pub fn new(mut r: R) -> io::Result<Self> {
        for base in PARTITION_OFFSETS {
            let mut hdr = [0u8; 28];
            r.seek(SeekFrom::Start(base + 32 * SECTOR))?;
            if r.read_exact(&mut hdr).is_err() || &hdr[..20] != MAGIC {
                continue;
            }
            let root = Entry {
                name: String::new(),
                sector: u32::from_le_bytes(hdr[20..24].try_into().unwrap()),
                size: u32::from_le_bytes(hdr[24..28].try_into().unwrap()),
                is_dir: true,
            };
            return Ok(Disc { r, base, root });
        }
        Err(bad(
            "not an Xbox / Xbox 360 disc image (no XDVDFS volume found)".into(),
        ))
    }

    pub fn partition_offset(&self) -> u64 {
        self.base
    }

    pub fn root(&self) -> &Entry {
        &self.root
    }

    /// Entries of a directory, sorted case-insensitively by name.
    pub fn read_dir(&mut self, dir: &Entry) -> io::Result<Vec<Entry>> {
        if !dir.is_dir {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a directory", dir.name),
            ));
        }
        if dir.size == 0 {
            return Ok(Vec::new());
        }
        let table = self.read_range(dir.sector, 0, dir.size as usize)?;
        let mut out = Vec::new();
        let mut stack = vec![0usize];
        let mut seen = HashSet::new();
        while let Some(off) = stack.pop() {
            if off + 14 > table.len() || !seen.insert(off) {
                continue;
            }
            let e = &table[off..];
            let left = u16::from_le_bytes([e[0], e[1]]);
            let right = u16::from_le_bytes([e[2], e[3]]);
            if left == 0xFFFF && right == 0xFFFF {
                continue;
            }
            let len = e[13] as usize;
            if off + 14 + len > table.len() {
                return Err(bad(format!(
                    "directory entry at {off:#x} runs past its table"
                )));
            }
            out.push(Entry {
                name: String::from_utf8_lossy(&e[14..14 + len]).into_owned(),
                sector: u32::from_le_bytes(e[4..8].try_into().unwrap()),
                size: u32::from_le_bytes(e[8..12].try_into().unwrap()),
                is_dir: e[12] & ATTR_DIRECTORY != 0,
            });
            for link in [left, right] {
                if link != 0 && link != 0xFFFF {
                    stack.push(link as usize * 4);
                }
            }
        }
        out.sort_by_key(|e| e.name.to_ascii_lowercase());
        Ok(out)
    }

    /// Looks up a `/`- or `\`-separated path, case-insensitively.
    pub fn find(&mut self, path: &str) -> io::Result<Entry> {
        let mut cur = self.root.clone();
        for part in path.split(['/', '\\']).filter(|p| !p.is_empty()) {
            cur = self
                .read_dir(&cur)?
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(part))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("{path}: no `{part}` on the disc"),
                    )
                })?;
        }
        Ok(cur)
    }

    /// Calls `f(path, entry)` for every file and directory below `dir`, depth first, sorted.
    pub fn walk(
        &mut self,
        dir: &Entry,
        prefix: &str,
        f: &mut dyn FnMut(&str, &Entry),
    ) -> io::Result<()> {
        for e in self.read_dir(dir)? {
            let path = if prefix.is_empty() {
                e.name.clone()
            } else {
                format!("{prefix}/{}", e.name)
            };
            f(&path, &e);
            if e.is_dir {
                self.walk(&e, &path, f)?;
            }
        }
        Ok(())
    }

    pub fn read_range(&mut self, sector: u32, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.r
            .seek(SeekFrom::Start(self.base + sector as u64 * SECTOR + offset))?;
        self.r.read_exact(&mut buf)?;
        Ok(buf)
    }

    pub fn read_file(&mut self, file: &Entry) -> io::Result<Vec<u8>> {
        if file.is_dir {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is a directory", file.name),
            ));
        }
        self.read_range(file.sector, 0, file.size as usize)
    }
}

/// One file on the disc, readable at offsets without loading it whole (BIG archives run to 800 MB).
pub struct DiscFile<'d, R> {
    pub disc: &'d mut Disc<R>,
    pub entry: Entry,
}

impl<R: Read + Seek> crate::big::ReadAt for DiscFile<'_, R> {
    fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        if offset + len as u64 > self.entry.size as u64 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.disc.read_range(self.entry.sector, offset, len)
    }
    fn len(&self) -> u64 {
        self.entry.size as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn entry(right: u16, sector: u32, size: u32, attr: u8, name: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&right.to_le_bytes());
        v.extend_from_slice(&sector.to_le_bytes());
        v.extend_from_slice(&size.to_le_bytes());
        v.push(attr);
        v.push(name.len() as u8);
        v.extend_from_slice(name.as_bytes());
        while v.len() % 4 != 0 {
            v.push(0xFF);
        }
        v
    }

    fn put(img: &mut [u8], base: u64, sector: u64, bytes: &[u8]) {
        let at = (base + sector * SECTOR) as usize;
        img[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A tiny synthetic image built in the test: root (sector 40) holds `data/` and `readme.txt`.
    fn image(base: u64) -> Vec<u8> {
        let mut img = vec![0u8; (base + 64 * SECTOR) as usize];
        let mut vd = MAGIC.to_vec();
        vd.extend_from_slice(&40u32.to_le_bytes());
        vd.extend_from_slice(&(SECTOR as u32).to_le_bytes());
        put(&mut img, base, 32, &vd);
        let first_len = entry(0, 41, SECTOR as u32, ATTR_DIRECTORY, "data").len();
        let mut root = entry(
            (first_len / 4) as u16,
            41,
            SECTOR as u32,
            ATTR_DIRECTORY,
            "data",
        );
        root.extend(entry(0, 42, 5, 0x80, "readme.txt"));
        root.resize(SECTOR as usize, 0xFF);
        put(&mut img, base, 40, &root);
        let mut sub = entry(0, 43, 3, 0x80, "x.bin");
        sub.resize(SECTOR as usize, 0xFF);
        put(&mut img, base, 41, &sub);
        put(&mut img, base, 42, b"hello");
        put(&mut img, base, 43, b"abc");
        img
    }

    #[test]
    fn reads_tree_and_files() {
        for base in [0, 0x0FD9_0000] {
            let mut d = Disc::new(Cursor::new(image(base))).unwrap();
            assert_eq!(d.partition_offset(), base);
            let root = d.root().clone();
            let names: Vec<_> = d
                .read_dir(&root)
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert_eq!(names, ["data", "readme.txt"]);
            let f = d.find("README.TXT").unwrap();
            assert_eq!(d.read_file(&f).unwrap(), b"hello");
            let f = d.find("data\\x.bin").unwrap();
            assert_eq!(d.read_file(&f).unwrap(), b"abc");
            let mut all = Vec::new();
            d.walk(&root, "", &mut |p, _| all.push(p.to_string()))
                .unwrap();
            assert_eq!(all, ["data", "data/x.bin", "readme.txt"]);
            assert!(d.find("nope").is_err());
        }
    }

    #[test]
    fn rejects_non_disc() {
        assert!(Disc::new(Cursor::new(vec![0u8; 0x10000])).is_err());
    }
}
