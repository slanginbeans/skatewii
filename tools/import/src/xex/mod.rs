//! Data that exists only in the game executable: the ocean PCA water table and the front-end menu tables.
//!
//! fg_import does **not** decrypt executables and contains no console keys. The player provides an image
//! they have already decrypted themselves, either:
//! - an unencrypted XEX2 (encryption type 0), stored raw ("basic") or LZX-compressed ("normal"), or
//! - the mapped PE image itself (starts with `MZ`), with its load address (`--base`, usually 0x82000000).
//!
//! XEX2 layout as in the public documentation used by Xenia and xextool, ported from
//! SK8-ENGINE/skate-3-rust-engine crates/skate-data/src/xex (GPL-3.0) without its decryption.

#[allow(clippy::needless_range_loop)] // ported code, kept close to upstream
mod lzx;
#[allow(clippy::type_complexity)] // ported code, kept close to upstream
pub mod menu_tables;
pub mod ocean_pca;

const FILE_FORMAT_INFO: u32 = 0x0000_03FF;
const IMAGE_BASE_ADDRESS: u32 = 0x0001_0201;
pub const DEFAULT_BASE: u32 = 0x8200_0000;

#[derive(Debug)]
pub struct XexImage {
    /// Virtual address of `image[0]`.
    pub base_address: u32,
    pub image: Vec<u8>,
}

fn be16(d: &[u8], at: usize) -> Result<u16, String> {
    d.get(at..at + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or_else(|| format!("truncated XEX at {at}"))
}
fn be32(d: &[u8], at: usize) -> Result<u32, String> {
    d.get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| format!("truncated XEX at {at}"))
}
fn span(d: &[u8], at: usize, n: usize) -> Result<&[u8], String> {
    d.get(at..at.checked_add(n).ok_or("XEX offset overflow")?)
        .ok_or_else(|| format!("truncated XEX at {at} (need {n})"))
}

pub const ENCRYPTED: &str = "this executable is still encrypted. fg_import does not decrypt executables: \
provide an image you have already decrypted (an unencrypted XEX2, or the mapped PE image that starts with MZ)";

impl XexImage {
    /// An unencrypted XEX2, or a mapped PE image loaded at `base`.
    pub fn load(bytes: &[u8], base: u32) -> Result<Self, String> {
        match bytes.get(..4) {
            Some(b"XEX2") => Self::parse_xex(bytes),
            Some([b'M', b'Z', ..]) => Ok(Self {
                base_address: base,
                image: bytes.to_vec(),
            }),
            _ => Err("not an XEX2 executable or a PE image".into()),
        }
    }

    fn parse_xex(xex: &[u8]) -> Result<Self, String> {
        let payload_offset = be32(xex, 8)? as usize;
        let security = be32(xex, 16)? as usize;
        let header_count = be32(xex, 20)? as usize;
        let (mut format, mut base_address) = (None, None);
        for i in 0..header_count.min(4096) {
            match be32(xex, 24 + i * 8)? {
                FILE_FORMAT_INFO => format = Some(be32(xex, 28 + i * 8)? as usize),
                IMAGE_BASE_ADDRESS => base_address = Some(be32(xex, 28 + i * 8)?),
                _ => {}
            }
        }
        let format = format.ok_or("XEX has no file format header")?;
        if be16(xex, format + 4)? != 0 {
            return Err(ENCRYPTED.into());
        }
        let compression = be16(xex, format + 6)?;
        let image_size = be32(xex, security + 4)? as usize;
        let base_address = match base_address {
            Some(a) => a,
            None => be32(xex, security + 0x110)?,
        };
        let payload = span(
            xex,
            payload_offset,
            xex.len().saturating_sub(payload_offset),
        )?;
        let image = match compression {
            0 => payload.to_vec(),
            1 => basic(xex, format, payload, image_size)?,
            2 => normal(xex, format, payload, image_size)?,
            other => return Err(format!("unsupported XEX compression type {other}")),
        };
        if image.get(..2) != Some(b"MZ") {
            return Err("unpacked XEX image does not start with a PE header".into());
        }
        Ok(Self {
            base_address,
            image,
        })
    }

    /// Bytes at a virtual address.
    pub fn at(&self, address: u32, len: usize) -> Option<&[u8]> {
        let offset = address.checked_sub(self.base_address)? as usize;
        self.image.get(offset..offset.checked_add(len)?)
    }
}

/// Basic compression: (data size, zero size) pairs; the zeros are not stored.
fn basic(xex: &[u8], format: usize, payload: &[u8], image_size: usize) -> Result<Vec<u8>, String> {
    let blocks = (be32(xex, format)? as usize).saturating_sub(8) / 8;
    let mut image = Vec::with_capacity(image_size);
    let mut at = 0;
    for i in 0..blocks {
        let data = be32(xex, format + 8 + i * 8)? as usize;
        let zeros = be32(xex, format + 12 + i * 8)? as usize;
        image.extend_from_slice(span(payload, at, data)?);
        at += data;
        image.resize(image.len() + zeros, 0);
    }
    image.resize(image_size, 0);
    Ok(image)
}

/// Normal compression: a chain of blocks, each starting with the next block's size and hash, then
/// u16-length-prefixed LZX chunks.
fn normal(xex: &[u8], format: usize, payload: &[u8], image_size: usize) -> Result<Vec<u8>, String> {
    let window_size = be32(xex, format + 8)? as usize;
    let mut block_size = be32(xex, format + 12)? as usize;
    let mut at = 0;
    let mut stream = Vec::with_capacity(payload.len());
    while block_size != 0 {
        let block = span(payload, at, block_size)?;
        let next = be32(block, 0)? as usize;
        let mut p = 24;
        loop {
            let chunk = be16(block, p)? as usize;
            p += 2;
            if chunk == 0 {
                break;
            }
            stream.extend_from_slice(span(block, p, chunk)?);
            p += chunk;
        }
        at += block_size;
        block_size = next;
    }
    lzx::decompress(&stream, window_size, image_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal XEX2 with a file-format header saying `encryption` and basic compression.
    fn xex(encryption: u16, payload: &[u8]) -> Vec<u8> {
        let mut x = vec![0u8; 0x200];
        x[0..4].copy_from_slice(b"XEX2");
        x[8..12].copy_from_slice(&0x200u32.to_be_bytes()); // payload offset
        x[16..20].copy_from_slice(&0x100u32.to_be_bytes()); // security info
        x[20..24].copy_from_slice(&2u32.to_be_bytes());
        x[24..28].copy_from_slice(&FILE_FORMAT_INFO.to_be_bytes());
        x[28..32].copy_from_slice(&0x80u32.to_be_bytes());
        x[32..36].copy_from_slice(&IMAGE_BASE_ADDRESS.to_be_bytes());
        x[36..40].copy_from_slice(&0x8200_0000u32.to_be_bytes());
        // format info @0x80: size 16 (one block), encryption, compression 1, block (len, zeros)
        x[0x80..0x84].copy_from_slice(&16u32.to_be_bytes());
        x[0x84..0x86].copy_from_slice(&encryption.to_be_bytes());
        x[0x86..0x88].copy_from_slice(&1u16.to_be_bytes());
        x[0x88..0x8C].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        x[0x8C..0x90].copy_from_slice(&4u32.to_be_bytes());
        x[0x104..0x108].copy_from_slice(&((payload.len() + 4) as u32).to_be_bytes()); // image size
        x.extend_from_slice(payload);
        x
    }

    #[test]
    fn loads_unencrypted_basic_xex() {
        let img = XexImage::load(&xex(0, b"MZ\x90\x00"), 0).unwrap();
        assert_eq!(img.base_address, 0x8200_0000);
        assert_eq!(img.image, b"MZ\x90\x00\0\0\0\0");
        assert_eq!(img.at(0x8200_0002, 2), Some(&b"\x90\x00"[..]));
    }

    #[test]
    fn refuses_encrypted_xex() {
        assert_eq!(XexImage::load(&xex(1, b"????"), 0).unwrap_err(), ENCRYPTED);
    }

    #[test]
    fn takes_pe_images_at_a_base() {
        let img = XexImage::load(b"MZ rest", DEFAULT_BASE).unwrap();
        assert_eq!(img.at(DEFAULT_BASE + 3, 4), Some(&b"rest"[..]));
        assert!(XexImage::load(b"ELF?", DEFAULT_BASE).is_err());
    }
}
