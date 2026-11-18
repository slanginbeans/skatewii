//! EA RefPack (QFS) decompression, with or without the `10 FB` / `90 FB` size header.
//! Format as documented by SK8-ENGINE/skate-3-rust-engine (tools/owned_game/refpack.py, GPL-3.0).

use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RefPack: {}", self.0)
    }
}

impl std::error::Error for Error {}

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

/// Size the stream says it decodes to, if it has a header.
pub fn header_size(data: &[u8]) -> Option<(usize, usize)> {
    match data {
        [0x90, 0xFB, a, b, c, d, ..] => Some((u32::from_be_bytes([*a, *b, *c, *d]) as usize, 6)),
        [0x10, 0xFB, a, b, c, ..] => Some((u32::from_be_bytes([0, *a, *b, *c]) as usize, 5)),
        _ => None,
    }
}

pub fn decompress(data: &[u8], expected: Option<usize>) -> Result<Vec<u8>, Error> {
    let (mut pos, size) = match header_size(data) {
        Some((n, at)) => {
            if expected.is_some_and(|e| e != n) {
                return err(format!(
                    "header says {n} bytes, archive says {}",
                    expected.unwrap()
                ));
            }
            (at, Some(n))
        }
        None => (0, expected),
    };
    let mut out: Vec<u8> = Vec::with_capacity(size.unwrap_or(data.len() * 2));

    fn literal(out: &mut Vec<u8>, data: &[u8], pos: &mut usize, n: usize) -> Result<(), Error> {
        let Some(src) = data.get(*pos..*pos + n) else {
            return err("truncated literal");
        };
        out.extend_from_slice(src);
        *pos += n;
        Ok(())
    }
    fn backref(out: &mut Vec<u8>, dist: usize, n: usize) -> Result<(), Error> {
        if dist == 0 || dist > out.len() {
            return err(format!(
                "back-reference {dist} past {} output bytes",
                out.len()
            ));
        }
        let start = out.len() - dist;
        for i in 0..n {
            out.push(out[start + i]);
        }
        Ok(())
    }
    let byte = |pos: usize| {
        data.get(pos)
            .copied()
            .ok_or_else(|| Error("truncated command".into()))
    };

    while pos < data.len() {
        let c = data[pos] as usize;
        pos += 1;
        if c < 0x80 {
            let b1 = byte(pos)? as usize;
            pos += 1;
            literal(&mut out, data, &mut pos, c & 3)?;
            backref(&mut out, ((c & 0x60) << 3) + b1 + 1, ((c >> 2) & 7) + 3)?;
        } else if c < 0xC0 {
            let (b1, b2) = (byte(pos)? as usize, byte(pos + 1)? as usize);
            pos += 2;
            literal(&mut out, data, &mut pos, b1 >> 6)?;
            backref(&mut out, ((b1 & 0x3F) << 8) + b2 + 1, (c & 0x3F) + 4)?;
        } else if c < 0xE0 {
            let (b1, b2, b3) = (
                byte(pos)? as usize,
                byte(pos + 1)? as usize,
                byte(pos + 2)? as usize,
            );
            pos += 3;
            literal(&mut out, data, &mut pos, c & 3)?;
            backref(
                &mut out,
                ((c & 0x10) << 12) + (b1 << 8) + b2 + 1,
                ((c & 0x0C) << 6) + b3 + 5,
            )?;
        } else if c < 0xFC {
            literal(&mut out, data, &mut pos, ((c & 0x1F) << 2) + 4)?;
        } else {
            literal(&mut out, data, &mut pos, c & 3)?;
            break;
        }
        if size.is_some_and(|s| out.len() > s) {
            return err("output exceeds declared size");
        }
    }
    if let Some(s) = size
        && out.len() != s
    {
        return err(format!("decoded {} bytes, expected {s}", out.len()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_backrefs() {
        // header: 10 FB, size 12
        // 0xE0 -> 4 literals "abcd"; 2-byte cmd: 0 literals, copy 3+? ... build: c = (n-3)<<2, b1 = dist-1
        let mut s = vec![0x10, 0xFB, 0, 0, 12, 0xE0, b'a', b'b', b'c', b'd'];
        // copy 8 bytes from distance 4 via 3-byte command: c=0x80|(8-4), b1=(0 lits<<6)|0, b2=3
        s.extend_from_slice(&[0x80 | 4, 0x00, 3]);
        s.push(0xFC); // stop, 0 literals
        assert_eq!(decompress(&s, Some(12)).unwrap(), b"abcdabcdabcd");
    }

    #[test]
    fn two_and_four_byte_commands() {
        // No header. 0xE0: 4 literals.
        let mut s = vec![0xE0, b'1', b'2', b'3', b'4'];
        // 2-byte command: 1 literal '5', then copy 3 from distance 5 -> "123".
        s.extend_from_slice(&[0x01, 4, b'5']);
        // 4-byte command: no literals, copy 5 from distance 8 -> "12345".
        s.extend_from_slice(&[0xC0, 0, 7, 0]);
        // Stop with 1 trailing literal.
        s.extend_from_slice(&[0xFD, b'!']);
        assert_eq!(decompress(&s, None).unwrap(), b"1234512312345!");
    }

    #[test]
    fn rejects_bad_streams() {
        assert!(decompress(&[0x00, 0x00], None).is_err()); // backref into empty output
        assert!(decompress(&[0x10, 0xFB, 0, 0, 9, 0xFC], None).is_err()); // short output
        assert!(decompress(&[0xE0, 1], None).is_err()); // truncated literal
    }
}
