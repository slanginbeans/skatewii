//! `fg_import park`: a skate park district from the disc (collision and grind rails for now) as a Wii
//! pack, `park.fgpak`.

use crate::big::Big;
use crate::world::{self, AssetRecord, Triangle};
use crate::xdvdfs::{Disc, DiscFile};
use crate::{Res, USAGE, pack};
use game_core::pack::Kind;
use std::io::{Read, Seek};
use std::path::PathBuf;

/// The districts small enough for the Wii (the city districts are hundreds of MB).
pub const PARKS: &[&str] = &[
    "StartPark",
    "MegaPark",
    "BlackBoxPark",
    "IndustrialSkatePark",
    "DownTownSkatePark",
    "SkateSchool",
    "MaloofMoneyCup",
];

pub struct Options {
    pub park: String,
    pub out: PathBuf,
}

pub fn options(args: &[String]) -> Res<Options> {
    let mut o = Options {
        park: crate::assets::DEFAULT_PARK.into(),
        out: PathBuf::from("imported"),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--park" => o.park = it.next().ok_or(USAGE)?.clone(),
            "--out" => o.out = PathBuf::from(it.next().ok_or(USAGE)?),
            _ => return Err(format!("unknown option {a}\n{USAGE}").into()),
        }
    }
    if !PARKS.iter().any(|p| p.eq_ignore_ascii_case(&o.park)) {
        return Err(format!("unknown park {}; parks: {}", o.park, PARKS.join(", ")).into());
    }
    Ok(o)
}

/// Every asset of one stream (`Sim`, `Pres`, `Tex`) of a district: its table and all its stream files.
pub fn load_stream<R: Read + Seek>(
    disc: &mut Disc<R>,
    park: &str,
    stream: &str,
) -> Res<Vec<(AssetRecord, Vec<u8>)>> {
    let archive = format!("data/content/worldDIST_{park}.big");
    let entry = disc.find(&archive)?;
    let mut big = Big::new(DiscFile { disc, entry })?;
    let dir = format!("data/content/world/stream/DIST_{park}/");
    let table = format!("{dir}DIST_{park}_{stream}.xst");
    let e = big
        .find(&table)
        .cloned()
        .ok_or_else(|| format!("{archive}: no {table}"))?;
    let records = world::read_atoc(&big.read(&e)?)?;
    let prefix = format!("{dir}c{stream}_").to_ascii_lowercase();
    let files: Vec<_> = big
        .entries
        .iter()
        .filter(|e| {
            let p = e.path.to_ascii_lowercase();
            p.starts_with(&prefix) && p.ends_with(".xsf")
        })
        .cloned()
        .collect();
    let mut assets: Vec<(AssetRecord, Vec<u8>)> = Vec::new();
    for f in files {
        for (r, bytes) in
            world::read_sfil(&big.read(&f)?, &records).map_err(|e| format!("{}: {e}", f.path))?
        {
            // Cells repeat shared assets; keep the first copy.
            if !assets.iter().any(|(a, _)| a.id == r.id) {
                assets.push((r, bytes));
            }
        }
    }
    let missing = records
        .iter()
        .filter(|r| !assets.iter().any(|(a, _)| a.id == r.id))
        .count();
    if missing > 0 {
        return Err(format!("{missing} {stream} assets missing from the stream files").into());
    }
    Ok(assets)
}

/// A grind rail as sampled points.
pub type RailPoints = Vec<[f32; 3]>;

/// A district's simulation data: every collision triangle, and its grind rails as sampled points.
pub fn simulation<R: Read + Seek>(
    disc: &mut Disc<R>,
    park: &str,
) -> Res<(Vec<Triangle>, Vec<RailPoints>)> {
    let (mut tris, mut rails) = (Vec::new(), Vec::new());
    for (r, data) in load_stream(disc, park, "Sim")? {
        let Ok(sections) = world::rx2_sections(&data) else {
            continue;
        };
        let at = |e| format!("asset {:016x}: {e}", r.id);
        for (ty, bytes) in sections {
            match ty {
                world::TYPE_CLUSTERED_MESH => {
                    tris.extend(world::decode_clustered_mesh(bytes).map_err(at)?)
                }
                // Parks keep everything in their global stream, in world space.
                world::TYPE_SPLINE_DATA => rails.extend(world::decode_splines(bytes).map_err(at)?),
                _ => {}
            }
        }
    }
    Ok((tris, rails))
}

/// Where to start: the centre of the broad, flat, upward face nearest the district origin (SK8's
/// spawn rule for parks), facing the middle of the park. (x, y, z, yaw)
pub fn spawn(tris: &[Triangle]) -> Option<[f32; 4]> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    // Broad flat upward faces: (centre, area).
    let mut flat = Vec::new();
    for t in tris {
        for v in &t.v {
            lo = [lo[0].min(v[0]), lo[1].min(v[2])];
            hi = [hi[0].max(v[0]), hi[1].max(v[2])];
        }
        let n = cross(sub(t.v[1], t.v[0]), sub(t.v[2], t.v[0]));
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let area = len / 2.0;
        if len == 0.0 || (n[1] / len).abs() < 0.9 || area < 2.0 {
            continue;
        }
        let c: [f32; 3] = std::array::from_fn(|k| (t.v[0][k] + t.v[1][k] + t.v[2][k]) / 3.0);
        flat.push((c, area));
    }
    // The main floor: the lowest half-metre height band holding a good share of the flat area. Indoor
    // parks have a roof that can out-measure a floor cluttered with ramps, and a few parks have small
    // pits below the floor.
    let band = |y: f32| (y * 2.0).round() as i32;
    let mut by_band = std::collections::BTreeMap::<i32, f32>::new();
    for (c, a) in &flat {
        *by_band.entry(band(c[1])).or_default() += a;
    }
    let most = by_band.values().copied().fold(0.0, f32::max);
    let floor = by_band
        .iter()
        .find(|(_, a)| **a >= 0.3 * most)
        .map(|(b, _)| *b)?;
    let (c, _) = flat
        .iter()
        .filter(|(c, _)| band(c[1]) == floor)
        .min_by(|(a, _), (b, _)| {
            (a[0] * a[0] + a[2] * a[2]).total_cmp(&(b[0] * b[0] + b[2] * b[2]))
        })?;
    let c = *c;
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    // Yaw 0 faces +Z, positive towards +X.
    let yaw = (mid[0] - c[0]).atan2(mid[1] - c[2]);
    Some([c[0], c[1], c[2], yaw])
}

pub fn run<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    let o = options(args)?;
    let (tris, rails) = simulation(disc, &o.park)?;
    let spawn = spawn(&tris).ok_or("no flat ground to start on")?;
    let rail_m: f32 = rails
        .iter()
        .flat_map(|r| r.windows(2))
        .map(|w| {
            (0..3)
                .map(|k| (w[1][k] - w[0][k]).powi(2))
                .sum::<f32>()
                .sqrt()
        })
        .sum();
    println!("{} grind rails, {:.0} m in all", rails.len(), rail_m);
    let encoded: Vec<([[f32; 3]; 3], u16, u16)> = tris
        .iter()
        .map(|t| (t.v, t.surface, t.flags as u16))
        .collect();
    let items = vec![
        pack::Item {
            name: "park/collision".into(),
            kind: Kind::Collision,
            data: game_core::collision::encode(&encoded),
            width: 0,
            height: 0,
            gx_format: 0,
        },
        pack::Item {
            name: "park/spawn".into(),
            kind: Kind::Data,
            data: spawn.iter().flat_map(|f| f.to_be_bytes()).collect(),
            width: 0,
            height: 0,
            gx_format: 0,
        },
        pack::Item {
            name: "park/rails".into(),
            kind: Kind::Data,
            data: game_core::rails::encode(&rails),
            width: 0,
            height: 0,
            gx_format: 0,
        },
    ];
    std::fs::create_dir_all(&o.out)?;
    let bytes = pack::write(&items);
    let path = o.out.join("park.fgpak");
    std::fs::write(&path, &bytes)?;
    println!(
        "{} -> {} ({} KiB), spawn at {:.1?}, yaw {:.0} degrees",
        o.park,
        path.display(),
        bytes.len() / 1024,
        &spawn[..3],
        spawn[3].to_degrees()
    );
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for t in &tris {
        for v in &t.v {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
    }
    let mut surfaces = std::collections::BTreeMap::<u16, usize>::new();
    for t in &tris {
        *surfaces.entry(t.surface).or_default() += 1;
    }
    println!(
        "{}: {} collision triangles, bounds {:.1?} .. {:.1?}",
        o.park,
        tris.len(),
        lo,
        hi
    );
    println!("surfaces (id: triangles): {surfaces:?}");
    // Ramps and banks at ground level (not flat, not walls, below 8 m).
    let ramps = tris
        .iter()
        .filter(|t| {
            let e1 = [
                t.v[1][0] - t.v[0][0],
                t.v[1][1] - t.v[0][1],
                t.v[1][2] - t.v[0][2],
            ];
            let e2 = [
                t.v[2][0] - t.v[0][0],
                t.v[2][1] - t.v[0][1],
                t.v[2][2] - t.v[0][2],
            ];
            let n = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let ny = (n[1] / (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()).abs();
            (0.35..0.98).contains(&ny) && t.v.iter().all(|v| v[1] < 8.0)
        })
        .count();
    println!("sloped (rideable) triangles below 8 m: {ramps}");
    Ok(())
}
