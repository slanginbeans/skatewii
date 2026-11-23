//! Writes `.fgpak` files (format and reader: `game_core::pack`).

use game_core::pack::{DATA_ALIGN, ENTRY_SIZE, HEADER_SIZE, Kind, MAGIC, VERSION, name_hash};

#[derive(Clone)]
pub struct Item {
    pub name: String,
    pub kind: Kind,
    pub data: Vec<u8>,
    pub width: u16,
    pub height: u16,
    pub gx_format: u8,
}

pub fn write(items: &[Item]) -> Vec<u8> {
    let mut strings = Vec::new();
    let mut name_offsets = Vec::new();
    for it in items {
        name_offsets.push(strings.len() as u32);
        strings.extend_from_slice(it.name.as_bytes());
        strings.push(0);
    }
    let strings_at = HEADER_SIZE + items.len() * ENTRY_SIZE;
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_be_bytes());
    out.extend_from_slice(&(items.len() as u32).to_be_bytes());
    out.extend_from_slice(&(strings_at as u32).to_be_bytes());
    out.resize(strings_at, 0);
    out.extend_from_slice(&strings);
    for (i, it) in items.iter().enumerate() {
        out.resize(out.len().next_multiple_of(DATA_ALIGN), 0);
        let off = out.len();
        out.extend_from_slice(&it.data);
        let e = HEADER_SIZE + i * ENTRY_SIZE;
        out[e..e + 4].copy_from_slice(&name_hash(&it.name).to_be_bytes());
        out[e + 4..e + 8].copy_from_slice(&name_offsets[i].to_be_bytes());
        out[e + 8..e + 12].copy_from_slice(&(it.kind as u32).to_be_bytes());
        out[e + 12..e + 16].copy_from_slice(&(off as u32).to_be_bytes());
        out[e + 16..e + 20].copy_from_slice(&(it.data.len() as u32).to_be_bytes());
        out[e + 20..e + 22].copy_from_slice(&it.width.to_be_bytes());
        out[e + 22..e + 24].copy_from_slice(&it.height.to_be_bytes());
        out[e + 24] = it.gx_format;
    }
    out.resize(out.len().next_multiple_of(DATA_ALIGN), 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_core::pack::Pack;

    #[test]
    fn round_trips_through_the_game_reader() {
        let items = [
            Item {
                name: "tex/a".into(),
                kind: Kind::Texture,
                data: vec![1; 32],
                width: 8,
                height: 8,
                gx_format: 0xE,
            },
            Item {
                name: "tex/B".into(),
                kind: Kind::Texture,
                data: vec![2; 7],
                width: 4,
                height: 4,
                gx_format: 5,
            },
        ];
        let bytes = write(&items);
        let pack = Pack::parse(&bytes).unwrap();
        assert_eq!(pack.len(), 2);
        let b = pack.find("TEX/b").unwrap();
        assert_eq!(
            (b.data, b.width, b.height, b.gx_format, b.kind),
            (&[2u8; 7][..], 4, 4, 5, 1)
        );
        let a = pack.find("tex/a").unwrap();
        assert_eq!(
            a.data.as_ptr() as usize % DATA_ALIGN,
            bytes.as_ptr() as usize % DATA_ALIGN
        );
        assert!(pack.find("tex/c").is_none());
    }
}
