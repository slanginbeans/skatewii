//! Create-a-character recipes (`data/cacrecipes/*.bin` in `miscload.big`): which model, LOD and textures
//! each body slot uses. Layout as documented by SK8-ENGINE/skate-3-rust-engine
//! (tools/extract_default_skater.py `parse_fallback_recipe`, GPL-3.0). Only the asset lists are read.

/// Recipes start after a 16-byte wrapper.
pub const PAYLOAD_OFFSET: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    pub lod: u8,
    pub model_id: String,
    pub material_id: String,
    /// (channel, texture id), e.g. ("diffuse", "0000228c03e38818")
    pub textures: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub asset_id: String,
    pub models: Vec<ModelRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    pub name: String,
    /// (slot name, assets) in file order.
    pub slots: Vec<(String, Vec<Asset>)>,
}

struct Cursor<'a> {
    d: &'a [u8],
}

impl Cursor<'_> {
    fn byte(&self, at: usize) -> Result<u8, String> {
        self.d
            .get(at)
            .copied()
            .ok_or_else(|| format!("recipe truncated at {at:#x}"))
    }
    fn bytes(&self, at: usize, n: usize) -> Result<&[u8], String> {
        self.d
            .get(at..at + n)
            .ok_or_else(|| format!("recipe truncated at {at:#x}"))
    }
    fn text(&self, at: usize, n: usize) -> Result<String, String> {
        Ok(String::from_utf8_lossy(self.bytes(at, n)?).into_owned())
    }
    fn hex(&self, at: usize, n: usize) -> Result<String, String> {
        Ok(self
            .bytes(at, n)?
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    }
}

pub fn parse(wrapped: &[u8]) -> Result<Recipe, String> {
    let c = Cursor {
        d: wrapped
            .get(PAYLOAD_OFFSET..)
            .ok_or("recipe shorter than its wrapper")?,
    };
    let name_len = c.byte(7)? as usize;
    let name = c.text(8, name_len)?;
    let mut i = 0x18 + name_len;
    let list_count = c.byte(i - 1)?;
    let mut slots = Vec::new();
    for _ in 0..list_count {
        let len = c.byte(i + 3)? as usize;
        let slot = c.text(i + 4, len)?;
        i += 8 + len;
        let asset_count = c.byte(i - 1)?;
        let mut assets = Vec::new();
        for _ in 0..asset_count {
            let asset_id = c.hex(i, 8)?;
            let model_count = c.byte(i + 11)?;
            i += 12;
            let mut models = Vec::new();
            for _ in 0..model_count {
                let block = c.bytes(i, 0x25)?;
                let (lod, loops, tex_count) = (block[0], block[0x14], block[0x24]);
                let model_id = c.hex(i + 9, 8)?;
                let material_id = c.hex(i + 0x19, 8)?;
                i += 0x25;
                let mut textures = Vec::new();
                for _ in 0..tex_count {
                    let len = c.byte(i + 3)? as usize;
                    textures.push((c.text(i + 4, len)?, c.hex(i + 4 + len, 8)?));
                    i += 12 + len;
                }
                // Extra material loops (some female heads); skipped.
                for _ in 1..loops.max(1) {
                    i += 0x10;
                    for _ in 0..c.byte(i - 1)? {
                        i += 12 + c.byte(i + 3)? as usize;
                    }
                }
                models.push(ModelRef {
                    lod,
                    model_id,
                    material_id,
                    textures,
                });
            }
            assets.push(Asset { asset_id, models });
        }
        slots.push((slot, assets));
    }
    Ok(Recipe { name, slots })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lp(s: &str) -> Vec<u8> {
        let mut v = vec![0, 0, 0, s.len() as u8];
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// One slot, one asset, one model with one texture: the shape of the fallback recipes.
    #[test]
    fn parses_a_synthetic_recipe() {
        let mut d = vec![0u8; PAYLOAD_OFFSET];
        let mut p = vec![0u8; 7];
        p.push(3);
        p.extend_from_slice(b"abc");
        p.resize(0x18 + 3, 0);
        p[0x18 + 3 - 1] = 1; // one slot list
        p.extend(lp("Feet"));
        p.extend_from_slice(&[0, 0, 0, 1]); // asset count in the last byte
        p.extend_from_slice(&[0xAA; 8]); // asset id
        p.extend_from_slice(&[0, 0, 0, 1]); // model count
        let mut block = vec![0u8; 0x25];
        block[0] = 1; // lod
        block[9..17].copy_from_slice(&[0x11; 8]);
        block[0x14] = 1;
        block[0x19..0x21].copy_from_slice(&[0x22; 8]);
        block[0x24] = 1;
        p.extend(block);
        p.extend(lp("diffuse"));
        p.extend_from_slice(&[0x33; 8]);
        d.extend(p);
        let r = parse(&d).unwrap();
        assert_eq!(r.name, "abc");
        let (slot, assets) = &r.slots[0];
        assert_eq!(slot, "Feet");
        assert_eq!(assets[0].asset_id, "aa".repeat(8));
        let m = &assets[0].models[0];
        assert_eq!((m.lod, m.model_id.as_str()), (1, "1111111111111111"));
        assert_eq!(m.textures, [("diffuse".to_string(), "33".repeat(8))]);
    }
}
