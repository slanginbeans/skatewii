//! `fg_import city`: measures a district's streaming cells (Skate 3's city districts and Skate School
//! keep their collision, models and textures in 100 m cells, `c<Stream>_<x>_<z>_high.xsf`) to plan
//! converting them cell by cell. Reports, per cell, what it holds and what a flatground pack of it
//! would weigh, and draws a top-down map of the collision. Nothing is written but the report.

use crate::big::{Big, BigEntry};
use crate::parkmodel::{ASSET_MODEL, ASSET_TEXTURE, read_meshes, texture_name};
use crate::rw4::Arena;
use crate::texture::{self, Format, Rgba};
use crate::world::{self, AssetRecord};
use crate::xdvdfs::{Disc, DiscFile};
use crate::{Res, USAGE};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::path::PathBuf;

/// The districts kept in streaming cells.
pub const DISTRICTS: &[&str] = &["DownTown", "University", "Industrial", "SkateSchool"];
/// A cell's edge (m); cells are named by their centre.
pub const CELL: f32 = 100.0;
/// Bytes of one collision triangle in a pack (`game_core::collision::TRIANGLE_BYTES`).
const TRI_BYTES: usize = 40;
/// Bytes of one rail point in a pack (position and class; `game_core::rails`), roughly.
const RAIL_POINT_BYTES: usize = 16;

/// Where a stream file sits: a cell's centre, or the district-wide `Global` file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    Global,
    Cell(i32, i32),
}

/// `c<stream>_<x>_<z>_high.xsf` or `c<stream>_Global.xsf` (any case), for the given stream.
pub fn parse_name(file: &str, stream: &str) -> Option<Place> {
    let f = file.rsplit('/').next()?.to_ascii_lowercase();
    let rest = f.strip_prefix(&format!("c{}_", stream.to_ascii_lowercase()))?;
    let rest = rest.strip_suffix(".xsf")?;
    if rest == "global" {
        return Some(Place::Global);
    }
    let rest = rest.strip_suffix("_high")?;
    let (x, z) = rest.split_once('_')?;
    Some(Place::Cell(x.parse().ok()?, z.parse().ok()?))
}

#[derive(Default, Debug, Clone)]
pub struct CellStats {
    pub sim_bytes: usize,
    pub pres_bytes: usize,
    pub tris: usize,
    /// Triangles whose centre lies outside the cell's square (more than 1 m out).
    pub tris_outside: usize,
    pub rails: usize,
    pub rail_points: usize,
    pub height: Option<(f32, f32)>,
    pub models: usize,
    pub meshes: usize,
    pub vertices: usize,
    pub indices: usize,
    /// Meshes whose centre lies outside the cell's square (more than 1 m out).
    pub meshes_outside: usize,
    /// Mesh bounds (x, z) of everything drawn.
    pub mesh_bounds: Option<([f32; 2], [f32; 2])>,
    pub textures: HashSet<String>,
    pub missing_textures: usize,
    pub asset_types: BTreeMap<u32, usize>,
}

impl CellStats {
    /// Pack bytes of the cell's meshes (`game_core::world` mesh entries).
    pub fn mesh_bytes(&self) -> usize {
        self.meshes * 12 + self.vertices * 24 + self.indices * 2
    }
    pub fn collision_bytes(&self) -> usize {
        self.tris * TRI_BYTES + self.rail_points * RAIL_POINT_BYTES
    }
}

/// A texture as the converter would store it: (bytes, from which Tex cell).
#[derive(Clone, Copy, Debug)]
struct TexSize {
    bytes: usize,
    cell: Place,
}

/// The size of a texture shrunk to `max` and stored as CMPR (4 bits a pixel), or RGB5A3 (16 bits) when
/// its format can carry alpha (an upper bound: the converter only keeps alpha for real cut-outs).
pub fn stored_size(width: usize, height: usize, format: Format, max: usize) -> usize {
    let (mut w, mut h) = (width.max(1), height.max(1));
    while w > max || h > max {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    let alpha = matches!(
        format,
        Format::Dxt3 | Format::Dxt5 | Format::A8R8G8B8 | Format::A8
    );
    if alpha {
        w.div_ceil(4) * 4 * h.div_ceil(4) * 4 * 2
    } else {
        w.div_ceil(8) * 8 * h.div_ceil(8) * 8 / 2
    }
}

struct Options {
    districts: Vec<String>,
    out: PathBuf,
    max: usize,
    map: bool,
}

fn options(args: &[String]) -> Res<Options> {
    let mut o = Options {
        districts: DISTRICTS.iter().map(|s| s.to_string()).collect(),
        out: PathBuf::from("imported/city"),
        max: 128,
        map: true,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--district" => {
                let d = it.next().ok_or(USAGE)?;
                let d = DISTRICTS
                    .iter()
                    .find(|x| x.eq_ignore_ascii_case(d))
                    .ok_or_else(|| {
                        format!("unknown district {d}; districts: {}", DISTRICTS.join(", "))
                    })?;
                o.districts = vec![d.to_string()];
            }
            "--out" => o.out = PathBuf::from(it.next().ok_or(USAGE)?),
            "--max" => o.max = it.next().ok_or(USAGE)?.parse::<usize>()?.clamp(8, 1024),
            "--no-map" => o.map = false,
            _ => return Err(format!("unknown option {a}\n{USAGE}").into()),
        }
    }
    Ok(o)
}

pub fn run<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    let o = options(args)?;
    std::fs::create_dir_all(&o.out)?;
    for d in &o.districts {
        measure(disc, d, &o)?;
    }
    Ok(())
}

fn inside(place: Place, x: f32, z: f32) -> bool {
    match place {
        Place::Global => true,
        Place::Cell(cx, cz) => {
            let h = CELL * 0.5 + 1.0;
            (x - cx as f32).abs() <= h && (z - cz as f32).abs() <= h
        }
    }
}

/// A stream file and where it sits.
pub(crate) type StreamFile = (Place, BigEntry);

/// One stream's table and its files by place.
pub(crate) fn stream_files<S: crate::big::ReadAt>(
    big: &mut Big<S>,
    district: &str,
    stream: &str,
) -> Res<(Vec<AssetRecord>, Vec<StreamFile>)> {
    let dir = format!("data/content/world/stream/DIST_{district}/");
    let table = format!("{dir}DIST_{district}_{stream}.xst");
    let e = big
        .find(&table)
        .cloned()
        .ok_or_else(|| format!("no {table}"))?;
    let records = world::read_atoc(&big.read(&e)?)?;
    let mut files: Vec<StreamFile> = big
        .entries
        .iter()
        .filter_map(|e| Some((parse_name(&e.path, stream)?, e.clone())))
        .collect();
    files.sort_by_key(|f| f.0);
    Ok((records, files))
}

fn measure<R: Read + Seek>(disc: &mut Disc<R>, district: &str, o: &Options) -> Res<()> {
    let archive = format!("data/content/worldDIST_{district}.big");
    let entry = disc.find(&archive)?;
    let mut big = Big::new(DiscFile { disc, entry })?;
    let t0 = std::time::Instant::now();

    // Every texture: its stored size and the Tex cell it comes from.
    let (tex_records, tex_files) = stream_files(&mut big, district, "Tex")?;
    let mut tex: HashMap<String, TexSize> = HashMap::new();
    let mut tex_cells: BTreeMap<Place, (usize, usize)> = BTreeMap::new();
    let mut tex_dupes = 0usize;
    for (place, f) in &tex_files {
        let assets = world::read_sfil(&big.read(f)?, &tex_records)
            .map_err(|e| format!("{}: {e}", f.path))?;
        for (r, data) in assets {
            if r.asset_type != ASSET_TEXTURE {
                continue;
            }
            let Some(name) = texture_name(&data) else {
                continue;
            };
            let Ok(a) = Arena::parse(&data) else { continue };
            let Some(Ok(t)) = texture::textures(&a).into_iter().next() else {
                continue;
            };
            let bytes = stored_size(t.info.width, t.info.height, t.info.format, o.max);
            let c = tex_cells.entry(*place).or_default();
            c.0 += 1;
            c.1 += bytes;
            if tex
                .insert(
                    name,
                    TexSize {
                        bytes,
                        cell: *place,
                    },
                )
                .is_some()
            {
                tex_dupes += 1;
            }
        }
    }

    // Collision and rails.
    let mut cells: BTreeMap<Place, CellStats> = BTreeMap::new();
    let mut all_tris: Vec<world::Triangle> = Vec::new();
    let (sim_records, sim_files) = stream_files(&mut big, district, "Sim")?;
    let mut seen: HashMap<u64, usize> = HashMap::new();
    for (place, f) in &sim_files {
        let assets = world::read_sfil(&big.read(f)?, &sim_records)
            .map_err(|e| format!("{}: {e}", f.path))?;
        let c = cells.entry(*place).or_default();
        c.sim_bytes += f.unpacked as usize;
        for (r, data) in assets {
            *seen.entry(r.id).or_default() += 1;
            *c.asset_types.entry(r.asset_type).or_default() += 1;
            let Ok(sections) = world::rx2_sections(&data) else {
                continue;
            };
            for (ty, bytes) in sections {
                match ty {
                    world::TYPE_CLUSTERED_MESH => {
                        let tris = world::decode_clustered_mesh(bytes)
                            .map_err(|e| format!("{}: asset {:016x}: {e}", f.path, r.id))?;
                        for t in &tris {
                            let cx = (t.v[0][0] + t.v[1][0] + t.v[2][0]) / 3.0;
                            let cz = (t.v[0][2] + t.v[1][2] + t.v[2][2]) / 3.0;
                            if !inside(*place, cx, cz) {
                                c.tris_outside += 1;
                            }
                            for v in &t.v {
                                let (lo, hi) = c.height.unwrap_or((v[1], v[1]));
                                c.height = Some((lo.min(v[1]), hi.max(v[1])));
                            }
                        }
                        c.tris += tris.len();
                        all_tris.extend(tris);
                    }
                    world::TYPE_SPLINE_DATA => {
                        let rails = world::decode_splines(bytes)
                            .map_err(|e| format!("{}: asset {:016x}: {e}", f.path, r.id))?;
                        c.rails += rails.len();
                        c.rail_points += rails.iter().map(Vec::len).sum::<usize>();
                    }
                    _ => {}
                }
            }
        }
    }
    let sim_shared = seen.values().filter(|&&n| n > 1).count();

    // Models.
    let (pres_records, pres_files) = stream_files(&mut big, district, "Pres")?;
    let mut seen: HashMap<u64, usize> = HashMap::new();
    for (place, f) in &pres_files {
        let assets = world::read_sfil(&big.read(f)?, &pres_records)
            .map_err(|e| format!("{}: {e}", f.path))?;
        let c = cells.entry(*place).or_default();
        c.pres_bytes += f.unpacked as usize;
        for (r, data) in assets {
            *c.asset_types.entry(r.asset_type).or_default() += 1;
            if r.asset_type == ASSET_TEXTURE {
                if let Some(name) = texture_name(&data)
                    && let Ok(a) = Arena::parse(&data)
                    && let Some(Ok(t)) = texture::textures(&a).into_iter().next()
                {
                    let bytes = stored_size(t.info.width, t.info.height, t.info.format, o.max);
                    tex.entry(name).or_insert(TexSize {
                        bytes,
                        cell: *place,
                    });
                }
                continue;
            }
            if r.asset_type != ASSET_MODEL {
                continue;
            }
            *seen.entry(r.id).or_default() += 1;
            let Ok(meshes) = Arena::parse(&data)
                .map_err(|e| e.to_string())
                .and_then(|a| read_meshes(&a).map_err(|e| e.to_string()))
            else {
                continue;
            };
            c.models += 1;
            for m in meshes {
                let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
                for p in &m.positions {
                    lo = [lo[0].min(p[0]), lo[1].min(p[2])];
                    hi = [hi[0].max(p[0]), hi[1].max(p[2])];
                }
                if m.positions.is_empty() {
                    continue;
                }
                if !inside(*place, (lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5) {
                    c.meshes_outside += 1;
                }
                let b = c.mesh_bounds.get_or_insert((lo, hi));
                *b = (
                    [b.0[0].min(lo[0]), b.0[1].min(lo[1])],
                    [b.1[0].max(hi[0]), b.1[1].max(hi[1])],
                );
                c.meshes += 1;
                c.vertices += m.positions.len();
                c.indices += m.indices.len();
                if let Some(t) = m.texture {
                    c.textures.insert(t);
                }
            }
        }
    }
    let pres_shared = seen.values().filter(|&&n| n > 1).count();
    // Textures the converter can't find are left untextured.
    for c in cells.values_mut() {
        let before = c.textures.len();
        c.textures.retain(|t| tex.contains_key(t));
        c.missing_textures = before - c.textures.len();
    }

    // Report.
    let tex_bytes = |c: &CellStats| c.textures.iter().map(|t| tex[t].bytes).sum::<usize>();
    let total = |c: &CellStats| c.collision_bytes() + c.mesh_bytes() + tex_bytes(c);
    let mut csv = String::from(
        "x,z,sim_kb,pres_kb,tris,tris_outside,rails,rail_points,y_min,y_max,models,meshes,vertices,indices,meshes_outside,textures,missing_textures,collision_kb,mesh_kb,texture_kb,pack_kb\n",
    );
    for (p, c) in &cells {
        let (x, z) = match p {
            Place::Global => ("global".to_string(), String::new()),
            Place::Cell(x, z) => (x.to_string(), z.to_string()),
        };
        let (ylo, yhi) = c.height.unwrap_or((0.0, 0.0));
        writeln!(
            csv,
            "{x},{z},{},{},{},{},{},{},{ylo:.1},{yhi:.1},{},{},{},{},{},{},{},{},{},{},{}",
            c.sim_bytes / 1024,
            c.pres_bytes / 1024,
            c.tris,
            c.tris_outside,
            c.rails,
            c.rail_points,
            c.models,
            c.meshes,
            c.vertices,
            c.indices,
            c.meshes_outside,
            c.textures.len(),
            c.missing_textures,
            c.collision_bytes() / 1024,
            c.mesh_bytes() / 1024,
            tex_bytes(c) / 1024,
            total(c) / 1024,
        )?;
    }
    let csv_path = o.out.join(format!("{district}.csv"));
    std::fs::write(&csv_path, csv)?;

    let grid: Vec<&CellStats> = cells
        .iter()
        .filter(|(p, _)| **p != Place::Global)
        .map(|(_, c)| c)
        .collect();
    let mut sizes: Vec<usize> = grid.iter().map(|c| total(c)).collect();
    sizes.sort_unstable();
    let pct = |q: f32| {
        sizes
            .get(((sizes.len() as f32 - 1.0) * q) as usize)
            .copied()
            .unwrap_or(0)
            / 1024
    };
    let sum = |f: &dyn Fn(&CellStats) -> usize| grid.iter().map(|c| f(c)).sum::<usize>();
    let busy = grid.iter().filter(|c| c.tris > 0 || c.meshes > 0).count();
    println!(
        "== {district}: {} cells ({busy} with collision or models), read in {:.1} s",
        grid.len(),
        t0.elapsed().as_secs_f32()
    );
    let (xs, zs): (Vec<i32>, Vec<i32>) = cells
        .keys()
        .filter_map(|p| match p {
            Place::Cell(x, z) => Some((*x, *z)),
            Place::Global => None,
        })
        .unzip();
    if let (Some(x0), Some(x1), Some(z0), Some(z1)) = (
        xs.iter().min(),
        xs.iter().max(),
        zs.iter().min(),
        zs.iter().max(),
    ) {
        println!(
            "   cell centres x {x0}..{x1}, z {z0}..{z1} ({:.1} x {:.1} km)",
            (x1 - x0) as f32 / 1000.0 + 0.1,
            (z1 - z0) as f32 / 1000.0 + 0.1,
        );
    }
    println!(
        "   cells: {} collision triangles ({} outside their cell), {} rails, {} meshes ({} outside their cell), {} vertices, {} textures used ({} missing)",
        sum(&|c| c.tris),
        sum(&|c| c.tris_outside),
        sum(&|c| c.rails),
        sum(&|c| c.meshes),
        sum(&|c| c.meshes_outside),
        sum(&|c| c.vertices),
        sum(&|c| c.textures.len()),
        sum(&|c| c.missing_textures),
    );
    println!(
        "   cell packs at {} px: median {} KB, p90 {} KB, p99 {} KB, max {} KB; all cells {:.1} MB (collision {:.1}, meshes {:.1}, textures {:.1})",
        o.max,
        pct(0.5),
        pct(0.9),
        pct(0.99),
        pct(1.0),
        sizes.iter().sum::<usize>() as f32 / 1048576.0,
        sum(&|c| c.collision_bytes()) as f32 / 1048576.0,
        sum(&|c| c.mesh_bytes()) as f32 / 1048576.0,
        sum(&|c| tex_bytes(c)) as f32 / 1048576.0,
    );
    if let Some(g) = cells.get(&Place::Global) {
        println!(
            "   global: {} KB sim, {} KB pres: {} triangles, {} rails ({} points), {} models, {} meshes, {} vertices, {} textures ({} missing), mesh bounds {:?}, asset types {:?}",
            g.sim_bytes / 1024,
            g.pres_bytes / 1024,
            g.tris,
            g.rails,
            g.rail_points,
            g.models,
            g.meshes,
            g.vertices,
            g.textures.len(),
            g.missing_textures,
            g.mesh_bounds,
            g.asset_types,
        );
    }
    let shared_tex = {
        let mut n: HashMap<&String, usize> = HashMap::new();
        for c in cells.values() {
            for t in &c.textures {
                *n.entry(t).or_default() += 1;
            }
        }
        n.values().filter(|&&k| k > 1).count()
    };
    println!(
        "   textures: {} different ({} in the Tex stream's {} cells, which repeat them {} times; the rest in Pres), {} used by more than one cell",
        tex.len(),
        tex.values()
            .filter(|t| tex_cells.contains_key(&t.cell))
            .count(),
        tex_cells.len(),
        tex_dupes,
        shared_tex,
    );
    println!(
        "   assets in more than one cell file: sim {sim_shared}, models {pres_shared} (counted in each cell)"
    );
    // The busiest cells.
    let mut by_size: Vec<(&Place, &CellStats)> =
        cells.iter().filter(|(p, _)| **p != Place::Global).collect();
    by_size.sort_by_key(|(_, c)| std::cmp::Reverse(total(c)));
    for (p, c) in by_size.iter().take(5) {
        println!(
            "   big cell {p:?}: {} KB ({} tris, {} meshes, {} vertices, {} textures {} KB)",
            total(c) / 1024,
            c.tris,
            c.meshes,
            c.vertices,
            c.textures.len(),
            tex_bytes(c) / 1024
        );
    }
    println!("   -> {}", csv_path.display());
    if o.map {
        let path = o.out.join(format!("{district}_map.png"));
        draw_map(&all_tris, &path)?;
        println!("   -> {}", path.display());
    }
    Ok(())
}

/// A top-down map of the collision, a pixel a metre: shaded by height and slope, walls dark, the 100 m
/// cell grid (cells are centred on odd multiples of 50 m) in faint lines, the origin's axes red.
fn draw_map(tris: &[world::Triangle], path: &std::path::Path) -> Res<()> {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for t in tris {
        for v in &t.v {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
    }
    if tris.is_empty() {
        return Ok(());
    }
    let (x0, z0) = (lo[0].floor(), lo[2].floor());
    let (w, h) = (
        (hi[0] - x0).ceil() as usize + 1,
        (hi[2] - z0).ceil() as usize + 1,
    );
    let mut top = vec![f32::MIN; w * h];
    let mut shade = vec![0u8; w * h];
    for t in tris {
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
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
        let ny = (n[1] / len).abs();
        let light = (((n[0] / len) * -0.4 + (n[2] / len) * -0.3 + ny * 0.85).abs() * 255.0) as u8;
        let (mut bx0, mut bx1, mut bz0, mut bz1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for v in &t.v {
            bx0 = bx0.min(v[0]);
            bx1 = bx1.max(v[0]);
            bz0 = bz0.min(v[2]);
            bz1 = bz1.max(v[2]);
        }
        let ymax = t.v.iter().map(|v| v[1]).fold(f32::MIN, f32::max);
        let (ix0, ix1) = ((bx0 - x0) as usize, ((bx1 - x0) as usize).min(w - 1));
        let (iz0, iz1) = ((bz0 - z0) as usize, ((bz1 - z0) as usize).min(h - 1));
        for iz in iz0..=iz1 {
            for ix in ix0..=ix1 {
                let (px, pz) = (x0 + ix as f32 + 0.5, z0 + iz as f32 + 0.5);
                // Inside the triangle's footprint (or a thin wall's box).
                let a = t.v[0];
                let (b, c) = (t.v[1], t.v[2]);
                let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
                let inside = if d.abs() < 1e-6 {
                    true
                } else {
                    let l1 = ((b[2] - c[2]) * (px - c[0]) + (c[0] - b[0]) * (pz - c[2])) / d;
                    let l2 = ((c[2] - a[2]) * (px - c[0]) + (a[0] - c[0]) * (pz - c[2])) / d;
                    let slack = 0.7 / (bx1 - bx0).min(bz1 - bz0).max(0.7);
                    l1 >= -slack && l2 >= -slack && 1.0 - l1 - l2 >= -slack
                };
                if inside {
                    let i = iz * w + ix;
                    if ymax > top[i] {
                        top[i] = ymax;
                        shade[i] = if ny < 0.35 { 40 } else { light };
                    }
                }
            }
        }
    }
    let span = (hi[1] - lo[1]).max(1.0);
    let mut img = Rgba {
        width: w,
        height: h,
        pixels: vec![0; w * h * 4],
    };
    for i in 0..w * h {
        let p = &mut img.pixels[i * 4..i * 4 + 4];
        if top[i] == f32::MIN {
            p.copy_from_slice(&[18, 22, 34, 255]);
            continue;
        }
        // Low ground blue-green, high ground warm.
        let k = ((top[i] - lo[1]) / span).clamp(0.0, 1.0);
        let base = [
            60.0 + 160.0 * k,
            120.0 + 60.0 * (1.0 - (2.0 * k - 1.0).abs()),
            150.0 - 110.0 * k,
        ];
        let s = shade[i] as f32 / 255.0;
        for c in 0..3 {
            p[c] = (base[c] * (0.35 + 0.65 * s)).clamp(0.0, 255.0) as u8;
        }
        p[3] = 255;
    }
    // The cell grid and the axes.
    for iz in 0..h {
        for ix in 0..w {
            let (x, z) = (x0 as i32 + ix as i32, z0 as i32 + iz as i32);
            let p = &mut img.pixels[(iz * w + ix) * 4..(iz * w + ix) * 4 + 4];
            if x == 0 || z == 0 {
                p[..3].copy_from_slice(&[220, 40, 40]);
            } else if x.rem_euclid(100) == 0 || z.rem_euclid(100) == 0 {
                for c in p[..3].iter_mut() {
                    *c = c.saturating_add(45);
                }
            }
        }
    }
    crate::write_png(path, &img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_names() {
        let d = "data/content/world/stream/DIST_DownTown/";
        assert_eq!(
            parse_name(&format!("{d}cSim_-150_250_high.xsf"), "Sim"),
            Some(Place::Cell(-150, 250))
        );
        assert_eq!(
            parse_name(&format!("{d}cPres_Global.xsf"), "Pres"),
            Some(Place::Global)
        );
        assert_eq!(
            parse_name(&format!("{d}cPres_50_1450_high.xsf"), "Sim"),
            None
        );
        assert_eq!(
            parse_name(&format!("{d}DIST_DownTown_Sim.xst"), "Sim"),
            None
        );
        assert_eq!(
            parse_name("CTEX_-400_-300_HIGH.XSF", "Tex"),
            Some(Place::Cell(-400, -300))
        );
    }

    #[test]
    fn stored_texture_sizes() {
        // 512 x 256 DXT1 shrinks to 128 x 64 CMPR: 4 bits a pixel.
        assert_eq!(stored_size(512, 256, Format::Dxt1, 128), 128 * 64 / 2);
        // With alpha: RGB5A3, 16 bits a pixel.
        assert_eq!(stored_size(64, 64, Format::Dxt5, 128), 64 * 64 * 2);
        // Tiny textures still fill a CMPR tile.
        assert_eq!(stored_size(4, 4, Format::Dxt1, 128), 8 * 8 / 2);
    }

    #[test]
    fn inside_a_cell() {
        assert!(inside(Place::Cell(50, 150), 99.0, 101.0));
        assert!(!inside(Place::Cell(50, 150), 103.0, 150.0));
        assert!(inside(Place::Global, 1e6, -1e6));
    }
}
