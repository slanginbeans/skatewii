//! `fg_import cap`: Skate 3's Create-a-Park objects (quarter pipes, rails, ledges...) in
//! `data/content/parkassets.big`. The static recipe (`data/content/recipe/static/*.xml`) names each object
//! and the RW4 arena that holds it; the arena carries the object's collision (a ClusteredMesh in the
//! object's own frame, Y up, metres, with the game's edge codes) and its grind splines.
//! Layouts as in world.rs (after SK8-ENGINE/skate-3-rust-engine, GPL-3.0).
//!
//! For research and for testing the riding on Skate 3's own ramp shapes (the test below rides the half pipe
//! preset). Nothing read here is written anywhere.

use crate::big::Big;
use crate::park::RailPoints;
use crate::world::{self, Triangle};
use crate::xdvdfs::{Disc, DiscFile};
use crate::{Res, USAGE};
use std::io::{Read, Seek};

pub const ARCHIVE: &str = "data/content/parkassets.big";
const RECIPES: &str = "data/content/recipe/static/";

/// One Create-a-Park object: its recipe name and the id of the arena (`<id>.rx2`) that holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub name: String,
    pub arena: u64,
}

/// The objects of a recipe XML, in recipe order: each `<mod ... n="name">` and the `arenaid` of its first
/// level of detail.
pub fn objects(xml: &str) -> Vec<Object> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find("<mod ") {
        rest = &rest[at..];
        let end = rest.find("</mod>").unwrap_or(rest.len());
        let body = &rest[..end];
        let attr = |key: &str| {
            let k = format!("{key}=\"");
            body.find(&k).and_then(|i| {
                let v = &body[i + k.len()..];
                v.find('"').map(|j| v[..j].to_string())
            })
        };
        let arena =
            attr("arenaid").and_then(|a| u64::from_str_radix(a.trim_start_matches("0x"), 16).ok());
        if let (Some(name), Some(arena)) = (attr("n"), arena) {
            out.push(Object { name, arena });
        }
        rest = &rest[end.min(rest.len())..];
        if rest.len() <= 5 {
            break;
        }
        rest = &rest[5..];
    }
    out
}

/// Every object in the static recipes.
pub fn list<R: Read + Seek>(disc: &mut Disc<R>) -> Res<Vec<Object>> {
    let entry = disc.find(ARCHIVE)?;
    let mut big = Big::new(DiscFile { disc, entry })?;
    let recipes: Vec<_> = big
        .entries
        .iter()
        .filter(|e| {
            let p = e.path.to_ascii_lowercase();
            p.starts_with(RECIPES) && p.ends_with(".xml")
        })
        .cloned()
        .collect();
    let mut out = Vec::new();
    for e in recipes {
        out.extend(objects(&String::from_utf8_lossy(&big.read(&e)?)));
    }
    Ok(out)
}

/// An object's collision triangles and grind splines, in the object's frame.
pub fn load<R: Read + Seek>(
    disc: &mut Disc<R>,
    name: &str,
) -> Res<(Vec<Triangle>, Vec<RailPoints>)> {
    let object = list(disc)?
        .into_iter()
        .find(|o| o.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("no Create-a-Park object {name}"))?;
    let entry = disc.find(ARCHIVE)?;
    let mut big = Big::new(DiscFile { disc, entry })?;
    let file = format!("{:016x}.rx2", object.arena);
    let e = big
        .entries
        .iter()
        .find(|e| e.path.to_ascii_lowercase().ends_with(&file))
        .ok_or_else(|| format!("{name}: no arena {file} in {ARCHIVE}"))?
        .clone();
    let data = big.read(&e)?;
    let (mut tris, mut rails) = (Vec::new(), Vec::new());
    for (ty, bytes) in world::rx2_sections(&data)? {
        match ty {
            world::TYPE_CLUSTERED_MESH => tris.extend(world::decode_clustered_mesh(bytes)?),
            world::TYPE_SPLINE_DATA => rails.extend(world::decode_splines(bytes)?),
            _ => {}
        }
    }
    Ok((tris, rails))
}

/// `fg_import cap <disc.iso> [name]`: lists the objects, or shows one object's collision and splines.
pub fn run<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    match args.first() {
        None => {
            for o in list(disc)? {
                println!("{:016x} {}", o.arena, o.name);
            }
        }
        Some(name) if !name.starts_with('-') => {
            let (tris, rails) = load(disc, name)?;
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in tris.iter().flat_map(|t| t.v) {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            let edged = tris.iter().filter(|t| t.flags & 0x20 != 0).count();
            println!(
                "{name}: {} triangles ({edged} with edge codes), bounds {lo:?} .. {hi:?}, {} grind splines",
                tris.len(),
                rails.len()
            );
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_object_names_and_arenas_from_a_recipe() {
        let xml = r#"<compositeasset n="db">
            <mat id="0x1" type="t"><ud n="cap" /></mat>
            <mod id="0x10" n="box_a"><ud n="cap"><cap.Unlock value="" /></ud>
                <lod idx="0" id="0x11" arenaid="0x00000000000000aa"></lod>
                <lod idx="1" id="0x12" arenaid="0x00000000000000ab"></lod></mod>
            <mod id="0x20" n="no_arena"></mod>
            <mod id="0x30" n="ramp_b"><lod idx="0" id="0x31" arenaid="0x2c"></lod></mod>
        </compositeasset>"#;
        assert_eq!(
            objects(xml),
            vec![
                Object {
                    name: "box_a".into(),
                    arena: 0xaa
                },
                Object {
                    name: "ramp_b".into(),
                    arena: 0x2c
                },
            ]
        );
    }

    /// Rides Skate 3's own Create-a-Park half pipe preset (two 3 m quarter pipes 6 m apart), read from the
    /// disc at `FG_ISO`; skipped without one. The rider rolls off the near deck and must reach the bottom
    /// rolling and ride up the far wall. (For scale: SK8, on the same mesh with the game's edge codes, takes
    /// the bottom at 9.2 m/s; flatground at c7a0475 at 7.2 with the imported rules, 6.4 with the built-in.)
    #[test]
    fn rides_the_cap_half_pipe_preset_from_the_disc() {
        let Some(iso) = std::env::var_os("FG_ISO").filter(|v| !v.is_empty()) else {
            eprintln!("FG_ISO not set: skipped");
            return;
        };
        let mut disc = Disc::open(std::path::Path::new(&iso)).unwrap();
        let (tris, _) = load(&mut disc, "aa_quarterpipePreset_Industrial").unwrap();
        // Floor at y 0; the preset turned so it runs along z with its near lip at x 0 (object z -> x).
        let place = |p: [f32; 3]| [p[2] + 3.0, p[1], -p[0]];
        let f = [
            [-60.0, 0.0, -60.0],
            [60.0, 0.0, -60.0],
            [60.0, 0.0, 60.0],
            [-60.0, 0.0, 60.0],
        ];
        let mut encoded = vec![([f[0], f[2], f[1]], 0, 0), ([f[0], f[3], f[2]], 0, 0)];
        encoded.extend(
            tris.iter()
                .map(|t| (t.v.map(place), t.surface, t.flags as u16)),
        );
        let collision = game_core::collision::encode(&encoded);
        let item = |name: &str, kind, data| crate::pack::Item {
            name: name.into(),
            kind,
            data,
            width: 0,
            height: 0,
            gx_format: 0,
        };
        use game_core::pack::Kind;
        let spawn = [0.0f32, 0.0, -40.0, 0.0]
            .iter()
            .flat_map(|f| f.to_be_bytes())
            .collect();
        let pack: &'static [u8] = Vec::leak(crate::pack::write(&[
            item("park/collision", Kind::Collision, collision),
            item("park/spawn", Kind::Data, spawn),
        ]));
        let mut g = game_core::Game::new();
        g.load_park(pack).unwrap();
        let deck_top = 2.999;
        g.place_rider(
            game_core::math::v3(-0.7, deck_top, 0.0),
            core::f32::consts::FRAC_PI_2,
            1.4,
        );
        let pad = game_core::input::PadRaw {
            flags: game_core::input::flag::CONNECTED,
            ..Default::default()
        };
        let (mut bottom, mut far) = (0.0f32, 0.0f32);
        for _ in 0..240 {
            g.update(&pad, 1.0 / 60.0);
            let sk = &g.skater;
            assert!(
                !matches!(sk.mode, game_core::skater::Mode::Bailed { .. }),
                "bailed at {:?}",
                sk.pos
            );
            let x = sk.pos.x;
            if (3.0..9.0).contains(&x) && matches!(sk.mode, game_core::skater::Mode::Rolling) {
                bottom = bottom.max(sk.ground_velocity().length());
            }
            if x > 9.0 {
                far = far.max(sk.pos.y);
            }
        }
        eprintln!("bottom speed {bottom:.2} m/s, far wall {far:.2} m");
        assert!(bottom > 6.0, "bottom speed {bottom}");
        assert!(far > 1.0, "far wall reached {far} m");
    }
}
