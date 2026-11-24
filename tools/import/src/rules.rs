//! `fg_import rules`: Skate 3's riding rules from the player's disc as `rules.fgpak`: the tuning values
//! flatground's skating and scoring read (`game_core::tuning::PATHS`, `game_core::scoring::rules::PATHS`,
//! from the `skatercollections` database), the trick catalogue (`rules/scoring`, [`crate::scoring`]) and
//! the flick-stick shapes (`data/joystick/*.pat`).

use crate::attrib::Database;
use crate::xdvdfs::Disc;
use crate::{Res, USAGE, pack, read_asset};
use game_core::pack::Kind;
use std::io::{Read, Seek};
use std::path::PathBuf;

const DB: &str = "data/big/db.big::data/db/";
const JOYSTICK: &str = "data/big/miscload.big::data/joystick/";
/// The gesture files, by the entry name the game reads (`rules/gestures/<name>`).
const SHAPES: &[&str] = &[
    "skater",
    "skater90",
    "skaterN90",
    "skater_air",
    "skater_fingerflip",
    "skaterls",
    "skaterstep",
];

/// A flick shape: name, tolerance radius, points.
pub type Shape = (String, f32, Vec<[f32; 2]>);

/// Parses a `.pat` file: `global_tolerance_dist`, then `pattern <name>` blocks of optional
/// `tolerance_dist` and `coord x y` lines. Other directives (time, speed, delay) are read and ignored, as
/// the game does.
pub fn parse_pat(text: &str) -> Res<Vec<Shape>> {
    let mut out: Vec<Shape> = Vec::new();
    let mut global = None;
    for (n, line) in text.lines().enumerate() {
        let words: Vec<&str> = line
            .split("//")
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        let Some(&first) = words.first() else {
            continue;
        };
        let num = |i: usize| -> Res<f32> {
            let v: f32 = words
                .get(i)
                .ok_or_else(|| format!("line {}: missing number", n + 1))?
                .parse()
                .map_err(|_| format!("line {}: bad number", n + 1))?;
            if v.is_finite() {
                Ok(v)
            } else {
                Err(format!("line {}: bad number", n + 1).into())
            }
        };
        match first {
            "global_tolerance_dist" => global = Some(num(1)?),
            "global_tolerance_time"
            | "global_tolerance_speed"
            | "global_anticipation_delay"
            | "tolerance_time" => {
                num(1)?;
            }
            "pattern" => out.push((
                words.get(1).ok_or("pattern without a name")?.to_string(),
                global.ok_or("pattern before global_tolerance_dist")?,
                Vec::new(),
            )),
            "tolerance_dist" => out.last_mut().ok_or("tolerance outside a pattern")?.1 = num(1)?,
            "coord" => {
                let p = [num(1)?, num(2)?];
                out.last_mut().ok_or("coord outside a pattern")?.2.push(p);
            }
            other => return Err(format!("line {}: unknown directive {other}", n + 1).into()),
        }
    }
    if let Some(p) = out.iter().find(|p| !(2..=15).contains(&p.2.len())) {
        return Err(format!("pattern {} has {} points", p.0, p.2.len()).into());
    }
    Ok(out)
}

/// The disc's tuning database (AttribSys `skaterschema` and `skatercollections`).
pub fn database<R: Read + Seek>(disc: &mut Disc<R>) -> Res<Database> {
    let file = |disc: &mut Disc<R>, name: &str| read_asset(disc, &format!("{DB}{name}"));
    Database::parse(
        (
            file(disc, "skaterschema.vlt")?,
            file(disc, "skaterschema.bin")?,
        ),
        (
            file(disc, "skatercollections.vlt")?,
            file(disc, "skatercollections.bin")?,
        ),
    )
}

pub fn run<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    let mut out = PathBuf::from("imported");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = PathBuf::from(it.next().ok_or(USAGE)?),
            _ => return Err(format!("unknown option {a}\n{USAGE}").into()),
        }
    }
    let db = database(disc)?;
    let mut values = Vec::new();
    let mut missing = Vec::new();
    let paths =
        || game_core::tuning::all_paths().chain(game_core::scoring::rules::PATHS.iter().copied());
    for path in paths() {
        match db.values(path) {
            Some(v) => values.push((path, v)),
            None => missing.push(path),
        }
    }
    for m in &missing {
        eprintln!("tuning: {m} not found (the built-in value stays)");
    }
    let mut items = vec![pack::Item {
        name: "rules/tuning".into(),
        kind: Kind::Data,
        data: game_core::tuning::encode(&values),
        width: 0,
        height: 0,
        gx_format: 0,
    }];
    // The trick catalogue: every trick's points and completion delay.
    let tricks =
        crate::scoring::catalogue(|name, field| db.raw(crate::scoring::CLASS, name, field));
    if tricks.is_empty() {
        eprintln!("scoring: no trick records found (the built-in catalogue stays)");
    } else {
        items.push(pack::Item {
            name: game_core::scoring::catalog::ENTRY.into(),
            kind: Kind::Data,
            data: game_core::scoring::catalog::encode(&tricks),
            width: 0,
            height: 0,
            gx_format: 0,
        });
    }
    // The camera: Skate 3's camera graph and every shot it can choose.
    let compiled = crate::camera::graph(&read_asset(disc, crate::camera::GRAPH)?)?;
    let roots = crate::camera::shots_named(&compiled)?;
    let field = |key: u64, name: &str| {
        db.raw_key(crate::camera::CLASS, key, name)
            .map(<[u8]>::to_vec)
    };
    let shots = crate::camera::shots(&roots, &field)?;
    let missing_shots: Vec<&String> = roots.iter().filter(|n| shots.find(n).is_none()).collect();
    println!(
        "camera: graph {} KiB, {} shots ({} named by the graph{})",
        compiled.len() / 1024,
        shots.defs.len(),
        roots.len(),
        if missing_shots.is_empty() {
            String::new()
        } else {
            format!(", not in the database: {missing_shots:?}")
        }
    );
    items.push(pack::Item {
        name: game_core::camera::select::ENTRY.into(),
        kind: Kind::StateGraph,
        data: compiled,
        width: 0,
        height: 0,
        gx_format: 0,
    });
    items.push(pack::Item {
        name: game_core::camera::shot::ENTRY.into(),
        kind: Kind::Data,
        data: shots.encode(),
        width: 0,
        height: 0,
        gx_format: 0,
    });
    // The camera shake's sample loop (without it the game uses its own).
    match read_asset(disc, crate::camera::SHAKE)
        .and_then(|b| crate::camera::shake(&String::from_utf8_lossy(&b)))
    {
        Ok(s) => {
            println!("camera: shake loop of {} samples", s.rotation.len());
            items.push(pack::Item {
                name: game_core::camera::shake::ENTRY.into(),
                kind: Kind::Data,
                data: s.encode(),
                width: 0,
                height: 0,
                gx_format: 0,
            });
        }
        Err(e) => eprintln!("camera: no shake samples ({e}; the built-in loop stays)"),
    }
    let mut shape_count = 0;
    for name in SHAPES {
        let text = read_asset(disc, &format!("{JOYSTICK}{name}.pat"))?;
        let text = String::from_utf8_lossy(&text);
        let shapes = parse_pat(&text).map_err(|e| format!("{name}.pat: {e}"))?;
        shape_count += shapes.len();
        items.push(pack::Item {
            name: format!("rules/gestures/{name}"),
            kind: Kind::Data,
            data: game_core::gesture::encode(&shapes),
            width: 0,
            height: 0,
            gx_format: 0,
        });
    }
    std::fs::create_dir_all(&out)?;
    let path = out.join("rules.fgpak");
    let bytes = pack::write(&items);
    std::fs::write(&path, &bytes)?;
    println!(
        "rules -> {} ({} bytes): {} of {} tuning values, {} tricks, {} flick shapes in {} files",
        path.display(),
        bytes.len(),
        values.len(),
        paths().count(),
        tricks.len(),
        shape_count,
        SHAPES.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pattern_files() {
        let text = "global_tolerance_dist 0.5\nglobal_tolerance_time 10.0 // ignored\n\
                    pattern Up\n\ttolerance_dist 0.25\n\tcoord 0 1\n\tcoord 0 -1\n\
                    pattern Across\n\tcoord -1 0\n\tcoord 1 0\n";
        let p = parse_pat(text).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], ("Up".into(), 0.25, vec![[0.0, 1.0], [0.0, -1.0]]));
        assert_eq!(p[1].1, 0.5, "global tolerance");
        assert!(
            parse_pat("pattern X\ncoord 0 1\ncoord 1 0").is_err(),
            "no global tolerance"
        );
        assert!(
            parse_pat("global_tolerance_dist 1\npattern X\ncoord 0 1").is_err(),
            "one point"
        );
        assert!(parse_pat("global_tolerance_dist 1\npolar 1 2").is_err());
    }
}
