//! Skate 3's trick catalogue for `rules.fgpak` (`rules/scoring`, `game_core::scoring::catalog`): per
//! trick its points and completion delay, from the disc's scoring records (class
//! `Hash_6918469984A8C596` in `skatercollections`, one collection per trick named by its identifier).
//! Layout and field names from SK8-ENGINE/skate-3-rust-engine's `skate-data/src/scoring.rs` (GPL-3.0),
//! which checks each record's id against the executable's enum (`game_core::scoring::catalog::IDENTIFIERS`)
//! as this does.

use game_core::scoring::catalog::IDENTIFIERS;

/// The scoring records' class and the fields read: the trick's enum id, its points, its completion
/// delay (s).
pub const CLASS: &str = "Hash_6918469984A8C596";
const ID: &str = "Hash_B2383F16252DFE8E";
const POINTS: &str = "Hash_937F62AE5C1ED284";
const DELAY: &str = "Hash_DC7F402E5680BA02";

fn word(raw: Option<&[u8]>) -> Option<[u8; 4]> {
    raw?.get(..4)?.try_into().ok()
}

/// Every trick the disc has a record for: (id, points, completion delay). `raw(identifier, field)` is
/// the record's raw big-endian field. A record whose id disagrees with the enum is skipped (and
/// reported), as SK8 rejects it.
pub fn catalogue<'a>(raw: impl Fn(&str, &str) -> Option<&'a [u8]>) -> Vec<(u16, i32, f32)> {
    let mut out = Vec::new();
    for (i, (name, _, _)) in IDENTIFIERS.iter().enumerate() {
        let Some(points) = word(raw(name, POINTS)) else {
            continue;
        };
        if let Some(id) = word(raw(name, ID)).map(u32::from_be_bytes)
            && id != i as u32
        {
            eprintln!("scoring: {name} is trick {id} on the disc, {i} in the enum (skipped)");
            continue;
        }
        let delay = word(raw(name, DELAY))
            .map(f32::from_be_bytes)
            .filter(|d| d.is_finite())
            .unwrap_or(0.0);
        out.push((i as u16, i32::from_be_bytes(points), delay));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_core::scoring::Catalog;
    use game_core::scoring::catalog::{encode, id};
    use std::collections::HashMap;

    #[test]
    fn the_catalogue_reads_each_tricks_record_into_the_rules_entry() {
        // Synthetic records: two tricks with points, ids and delays, one with no id field, one whose
        // id disagrees with the enum, and a field the importer doesn't read.
        let kickflip = id("kickflip").unwrap() as u32;
        let grind = id("fs_50_50").unwrap() as u32;
        let mut db: HashMap<(&str, &str), Vec<u8>> = HashMap::new();
        db.insert(("kickflip", POINTS), 123i32.to_be_bytes().to_vec());
        db.insert(("kickflip", ID), kickflip.to_be_bytes().to_vec());
        db.insert(("kickflip", DELAY), 0.0f32.to_be_bytes().to_vec());
        db.insert(("fs_50_50", POINTS), 7i32.to_be_bytes().to_vec());
        db.insert(("fs_50_50", ID), grind.to_be_bytes().to_vec());
        db.insert(("fs_50_50", DELAY), 0.3f32.to_be_bytes().to_vec());
        db.insert(("ollie", POINTS), 5i32.to_be_bytes().to_vec());
        db.insert(("heelflip", POINTS), 9i32.to_be_bytes().to_vec());
        db.insert(("heelflip", ID), 3u32.to_be_bytes().to_vec());
        db.insert(("heelflip", "Hash_0000000000000001"), vec![1]);
        let c = catalogue(|name, field| {
            db.iter()
                .find(|((n, f), _)| *n == name && *f == field)
                .map(|(_, v)| v.as_slice())
        });
        let ollie = id("ollie").unwrap();
        assert_eq!(
            c,
            vec![
                (grind as u16, 7, 0.3),
                (kickflip as u16, 123, 0.0),
                (ollie, 5, 0.0)
            ]
        );
        // Through the rules entry into the game's catalogue.
        let g = Catalog::parse(&encode(&c)).unwrap();
        assert_eq!(g.get(kickflip as u16), Some((123, 0.0)));
        assert_eq!(g.get(grind as u16), Some((7, 0.3)));
        assert_eq!(g.get(id("heelflip").unwrap()), None, "id disagreed");
        assert_eq!(g.len(), 3);
    }
}
