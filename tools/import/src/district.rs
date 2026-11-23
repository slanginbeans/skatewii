//! `fg_import district`: a district cut into 100 m cells, one pack each, for streaming
//! (`game_core::city`). Everything the district has (its cells' and its `Global` stream files) is placed
//! by position: collision triangles by their centre, meshes by the middle of their bounds, rails by their
//! middle point. The city districts keep their own 100 m cells, so their cells come out as they are on
//! the disc; Skate School, which keeps everything in its `Global` files, is cut up the same way.
//!
//! Each cell is an ordinary park pack (`world::ImportedPark`), with its own copy of the textures its
//! meshes use. A cell bigger than `--cap` has its textures halved until it fits (down to 16 px).

use crate::big::Big;
use crate::city::{CELL, DISTRICTS, stream_files};
use crate::park::{self, RailPoints};
use crate::parkmodel::{
    ASSET_MODEL, ASSET_TEXTURE, Mesh, WiiTexture, convert_texture, mesh_item, read_meshes,
    texture_item, texture_name,
};
use crate::rw4::Arena;
use crate::world::{self, Triangle};
use crate::xdvdfs::{Disc, DiscFile};
use crate::{Res, USAGE, pack};
use game_core::city::{CellInfo, Index};
use game_core::math::v3;
use game_core::pack::Kind;
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

/// Most starts a district gets (each its own location in the menu).
const MAX_STARTS: usize = 4;

/// Smallest texture edge a cell's textures are halved to while it is over its cap.
const MIN_TEXTURE: usize = 16;

pub struct Options {
    pub district: String,
    /// The district's folder is `<out>/<district>/`.
    pub out: PathBuf,
    pub max: usize,
    /// Largest cell pack (bytes) before its textures are halved.
    pub cap: usize,
}

fn options(args: &[String]) -> Res<Options> {
    let mut o = Options {
        district: String::new(),
        out: PathBuf::from("imported/city"),
        max: 128,
        cap: 3 << 20,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--district" => o.district = it.next().ok_or(USAGE)?.clone(),
            "--out" => o.out = PathBuf::from(it.next().ok_or(USAGE)?),
            "--max" => {
                o.max = it
                    .next()
                    .ok_or(USAGE)?
                    .parse::<usize>()?
                    .clamp(MIN_TEXTURE, 1024)
            }
            "--cap" => o.cap = it.next().ok_or(USAGE)?.parse::<usize>()? << 10,
            _ => return Err(format!("unknown option {a}\n{USAGE}").into()),
        }
    }
    o.district = DISTRICTS
        .iter()
        .find(|d| d.eq_ignore_ascii_case(&o.district))
        .ok_or_else(|| format!("--district: one of {}", DISTRICTS.join(", ")))?
        .to_string();
    Ok(o)
}

/// The centre of the cell holding (x, z).
pub fn cell_of(x: f32, z: f32) -> (i32, i32) {
    let c = |v: f32| ((v / CELL).floor() * CELL + CELL * 0.5) as i32;
    (c(x), c(z))
}

/// Everything a district holds, in world space.
#[derive(Default)]
pub struct Content {
    pub tris: Vec<Triangle>,
    pub rails: Vec<RailPoints>,
    pub meshes: Vec<Mesh>,
    /// Texture assets by name (the first copy of each).
    pub textures: HashMap<String, Vec<u8>>,
}

fn gather<R: Read + Seek>(disc: &mut Disc<R>, district: &str) -> Res<Content> {
    let archive = format!("data/content/worldDIST_{district}.big");
    let entry = disc.find(&archive)?;
    let mut big = Big::new(DiscFile { disc, entry })?;
    let mut c = Content::default();
    let (records, files) = stream_files(&mut big, district, "Sim")?;
    for (_, f) in &files {
        for (r, data) in
            world::read_sfil(&big.read(f)?, &records).map_err(|e| format!("{}: {e}", f.path))?
        {
            let Ok(sections) = world::rx2_sections(&data) else {
                continue;
            };
            let at = |e| format!("{}: asset {:016x}: {e}", f.path, r.id);
            for (ty, bytes) in sections {
                match ty {
                    world::TYPE_CLUSTERED_MESH => c
                        .tris
                        .extend(world::decode_clustered_mesh(bytes).map_err(at)?),
                    world::TYPE_SPLINE_DATA => {
                        c.rails.extend(world::decode_splines(bytes).map_err(at)?)
                    }
                    _ => {}
                }
            }
        }
    }
    for stream in ["Pres", "Tex"] {
        // Skate School has no Tex stream.
        let Ok((records, files)) = stream_files(&mut big, district, stream) else {
            continue;
        };
        for (_, f) in &files {
            for (r, data) in
                world::read_sfil(&big.read(f)?, &records).map_err(|e| format!("{}: {e}", f.path))?
            {
                match r.asset_type {
                    ASSET_TEXTURE => {
                        if let Some(name) = texture_name(&data) {
                            c.textures.entry(name).or_insert(data);
                        }
                    }
                    ASSET_MODEL => {
                        let meshes = Arena::parse(&data)
                            .map_err(|e| e.to_string())
                            .and_then(|a| read_meshes(&a).map_err(|e| e.to_string()));
                        match meshes {
                            Ok(m) => c
                                .meshes
                                .extend(m.into_iter().filter(|m| !m.positions.is_empty())),
                            Err(e) => eprintln!("{}: model {:016x}: {e}", f.path, r.id),
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(c)
}

/// What goes into one cell (indexes into [`Content`]).
#[derive(Default, Debug, PartialEq)]
pub struct Cell {
    pub tris: Vec<usize>,
    pub rails: Vec<usize>,
    pub meshes: Vec<usize>,
}

/// Every cell with something in it, by centre.
pub fn assign(c: &Content) -> BTreeMap<(i32, i32), Cell> {
    let mut cells: BTreeMap<(i32, i32), Cell> = BTreeMap::new();
    for (i, t) in c.tris.iter().enumerate() {
        let x = (t.v[0][0] + t.v[1][0] + t.v[2][0]) / 3.0;
        let z = (t.v[0][2] + t.v[1][2] + t.v[2][2]) / 3.0;
        cells.entry(cell_of(x, z)).or_default().tris.push(i);
    }
    for (i, r) in c.rails.iter().enumerate() {
        if let Some(p) = r.get(r.len() / 2) {
            cells.entry(cell_of(p[0], p[2])).or_default().rails.push(i);
        }
    }
    for (i, m) in c.meshes.iter().enumerate() {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in &m.positions {
            lo = [lo[0].min(p[0]), lo[1].min(p[2])];
            hi = [hi[0].max(p[0]), hi[1].max(p[2])];
        }
        cells
            .entry(cell_of((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5))
            .or_default()
            .meshes
            .push(i);
    }
    cells
}

/// Where a rider put into a cell stands: the middle of the flat face (either winding, as the park spawn
/// rule) nearest the cell's centre
/// (or the centre at height 0 when it has none), facing +z.
fn cell_spawn(tris: &[&Triangle], centre: (i32, i32)) -> [f32; 4] {
    let (cx, cz) = (centre.0 as f32, centre.1 as f32);
    let mut best: Option<([f32; 3], f32)> = None;
    for t in tris {
        let e1: [f32; 3] = std::array::from_fn(|k| t.v[1][k] - t.v[0][k]);
        let e2: [f32; 3] = std::array::from_fn(|k| t.v[2][k] - t.v[0][k]);
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len == 0.0 || (n[1] / len).abs() < 0.9 {
            continue;
        }
        let m: [f32; 3] = std::array::from_fn(|k| (t.v[0][k] + t.v[1][k] + t.v[2][k]) / 3.0);
        let d = (m[0] - cx).powi(2) + (m[2] - cz).powi(2);
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((m, d));
        }
    }
    match best {
        Some((m, _)) => [m[0], m[1], m[2], 0.0],
        None => [cx, 0.0, cz, 0.0],
    }
}

fn data(name: &str, kind: Kind, data: Vec<u8>) -> pack::Item {
    pack::Item {
        name: name.into(),
        kind,
        data,
        width: 0,
        height: 0,
        gx_format: 0,
    }
}

/// Converted textures, by (name, edge), shared between cells.
type TextureCache = HashMap<(String, usize), Option<WiiTexture>>;

/// The cell's meshes and their textures at `max` px: (items, texture bytes, vertices).
fn models(
    c: &Content,
    cell: &Cell,
    max: usize,
    cache: &mut TextureCache,
) -> (Vec<pack::Item>, usize, usize) {
    let mut items = Vec::new();
    let mut local: HashMap<&str, Option<(u16, bool)>> = HashMap::new();
    let (mut tex_bytes, mut verts) = (0, 0);
    for (n, &i) in cell.meshes.iter().enumerate() {
        let m = &c.meshes[i];
        let tex = m.texture.as_deref().and_then(|name| {
            *local.entry(name).or_insert_with(|| {
                let t = cache
                    .entry((name.to_string(), max))
                    .or_insert_with(|| convert_texture(c.textures.get(name)?, max))
                    .as_ref()?;
                let k = items
                    .iter()
                    .filter(|it: &&pack::Item| it.kind == Kind::Texture)
                    .count() as u16;
                tex_bytes += t.data.len();
                items.push(texture_item(format!("park/tex/{k}"), t));
                Some((k, t.alpha))
            })
        });
        verts += m.positions.len();
        items.push(mesh_item(format!("park/mesh/{n}"), m, tex));
    }
    (items, tex_bytes, verts)
}

/// Converts one district into `<out>/<district>/`: its cell packs and `index.fgpak`.
pub fn run<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    let o = options(args)?;
    let t0 = std::time::Instant::now();
    let far = park::deck_to_truck(disc).unwrap_or(game_core::rails::FAR_SIDE);
    let content = gather(disc, &o.district)?;
    println!(
        "{}: {} collision triangles, {} rails, {} meshes, {} textures (read in {:.1} s)",
        o.district,
        content.tris.len(),
        content.rails.len(),
        content.meshes.len(),
        content.textures.len(),
        t0.elapsed().as_secs_f32()
    );
    let cells = assign(&content);
    let dir = o.out.join(&o.district);
    std::fs::create_dir_all(&dir)?;
    // Cells from an earlier conversion that this one may not have.
    for e in std::fs::read_dir(&dir)?.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if (n.starts_with("c_") && n.ends_with(".fgpak")) || n == "index.fgpak" {
            std::fs::remove_file(e.path())?;
        }
    }
    let encode = |ids: &mut dyn Iterator<Item = usize>| {
        let t: Vec<([[f32; 3]; 3], u16, u16)> = ids
            .map(|i| {
                let t = &content.tris[i];
                (t.v, t.surface, t.flags as u16)
            })
            .collect();
        // Packed (read in place by the game), or plain when it is too big for that.
        game_core::collision::encode_packed(&t).unwrap_or_else(|| game_core::collision::encode(&t))
    };
    let mut cache = TextureCache::new();
    let mut infos = Vec::new();
    let (mut shrunk, mut total, mut rail_counts) = (0usize, 0usize, [0usize; 5]);
    for (&key, cell) in &cells {
        let collision = encode(&mut cell.tris.iter().copied());
        let rails: Vec<RailPoints> = cell
            .rails
            .iter()
            .map(|&i| content.rails[i].clone())
            .collect();
        // The rails' probes see the neighbouring cells' collision too (a ledge on a cell's edge).
        let rails = if rails.is_empty() {
            Vec::new()
        } else {
            let step = CELL as i32;
            let mut around = (-1..=1)
                .flat_map(|i| (-1..=1).map(move |j| (key.0 + i * step, key.1 + j * step)))
                .filter_map(|k| cells.get(&k))
                .flat_map(|c| c.tris.iter().copied());
            let (r, counts) = park::classify_rails_quietly(&encode(&mut around), &rails, far)?;
            for k in 0..5 {
                rail_counts[k] += counts[k];
            }
            r
        };
        let tris: Vec<&Triangle> = cell.tris.iter().map(|&i| &content.tris[i]).collect();
        let spawn = cell_spawn(&tris, key);
        let base = [
            data("park/collision", Kind::Collision, collision),
            data(
                "park/spawn",
                Kind::Data,
                spawn.iter().flat_map(|f| f.to_be_bytes()).collect(),
            ),
            data("park/rails", Kind::Data, game_core::rails::encode(&rails)),
        ];
        let mut max = o.max;
        let (bytes, tex_bytes, verts) = loop {
            let (m, tex_bytes, verts) = models(&content, cell, max, &mut cache);
            let mut items = base.to_vec();
            items.extend(m);
            let bytes = pack::write(&items);
            if bytes.len() <= o.cap || max <= MIN_TEXTURE || tex_bytes == 0 {
                if max < o.max {
                    shrunk += 1;
                }
                break (bytes, tex_bytes, verts);
            }
            max /= 2;
        };
        let info = CellInfo {
            x: key.0,
            z: key.1,
            bytes: bytes.len() as u32,
            tris: cell.tris.len() as u32,
            vertices: verts as u32,
            texture_bytes: tex_bytes as u32,
        };
        std::fs::write(dir.join(info.file()), &bytes)?;
        total += bytes.len();
        infos.push(info);
    }
    let t1 = std::time::Instant::now();
    let mut starts = crate::starts::pick(&content.tris, &content.rails, MAX_STARTS);
    if starts.is_empty() {
        // Nowhere passes: the park spawn rule.
        let s = park::spawn(&content.tris).unwrap_or([0.0; 4]);
        starts.push((v3(s[0], s[1], s[2]), s[3]));
    }
    let floor = content
        .tris
        .iter()
        .flat_map(|t| t.v.iter().map(|v| v[1]))
        .fold(f32::MAX, f32::min);
    let index = Index {
        cell: CELL,
        floor: if floor == f32::MAX { 0.0 } else { floor },
        starts,
        cells: infos,
    };
    for (i, (p, yaw)) in index.starts.iter().enumerate() {
        println!(
            "start {}: {:.1} {:.1} {:.1} yaw {:.0}",
            i + 1,
            p.x,
            p.y,
            p.z,
            yaw.to_degrees()
        );
    }
    println!("starts picked in {:.1} s", t1.elapsed().as_secs_f32());
    write_index(&dir, &index)?;
    let mut sizes: Vec<u32> = index.cells.iter().map(|c| c.bytes).collect();
    sizes.sort_unstable();
    let p = |q: f32| {
        sizes
            .get(((sizes.len() as f32 - 1.0) * q) as usize)
            .copied()
            .unwrap_or(0)
            / 1024
    };
    println!(
        "{} -> {}: {} cells, {:.1} MB (median {} KB, p90 {} KB, max {} KB); {} cells' textures shrunk to fit {} KB; rail segments: {} thin, {} fat, {} ledge, {} curb, {} blocked; {} starts; {:.1} s",
        o.district,
        dir.display(),
        index.cells.len(),
        total as f32 / 1048576.0,
        p(0.5),
        p(0.9),
        p(1.0),
        shrunk,
        o.cap / 1024,
        rail_counts[0],
        rail_counts[1],
        rail_counts[2],
        rail_counts[3],
        rail_counts[4],
        index.starts.len(),
        t0.elapsed().as_secs_f32()
    );
    Ok(())
}

/// `index.fgpak`: the district's [`Index`] as `city/index`.
pub fn write_index(dir: &Path, index: &Index) -> Res<()> {
    let bytes = pack::write(&[data("city/index", Kind::Data, index.encode())]);
    std::fs::write(dir.join("index.fgpak"), bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(x: f32, z: f32) -> Triangle {
        Triangle {
            v: [[x, 0.0, z], [x + 1.0, 0.0, z], [x, 0.0, z + 1.0]],
            surface: 0,
            flags: 0,
        }
    }

    #[test]
    fn cells_by_position() {
        assert_eq!(cell_of(0.0, 0.0), (50, 50));
        assert_eq!(cell_of(-0.5, 199.0), (-50, 150));
        assert_eq!(cell_of(-100.0, -100.5), (-50, -150));
        let c = Content {
            tris: vec![tri(10.0, 10.0), tri(-10.0, 10.0), tri(20.0, 30.0)],
            rails: vec![vec![[-5.0, 0.0, 0.0], [5.0, 0.0, 0.0], [150.0, 0.0, 0.0]]],
            meshes: vec![Mesh {
                positions: vec![[-30.0, 0.0, 120.0], [-10.0, 5.0, 140.0]],
                uvs: vec![[0.0; 2]; 2],
                indices: vec![0, 1, 0],
                texture: None,
            }],
            textures: HashMap::new(),
        };
        let cells = assign(&c);
        assert_eq!(
            cells.keys().copied().collect::<Vec<_>>(),
            vec![(-50, 50), (-50, 150), (50, 50)]
        );
        assert_eq!(cells[&(50, 50)].tris, vec![0, 2]);
        assert_eq!(cells[&(-50, 50)].tris, vec![1]);
        // The rail by its middle point, the mesh by the middle of its bounds.
        assert_eq!(cells[&(50, 50)].rails, vec![0]);
        assert_eq!(cells[&(-50, 150)].meshes, vec![0]);
    }

    #[test]
    fn a_cell_pack_loads_as_a_park_and_the_index_lists_it() {
        let t = [tri(10.0, 10.0), tri(60.0, 60.0)];
        let refs: Vec<&Triangle> = t.iter().collect();
        let spawn = cell_spawn(&refs, (50, 50));
        assert_eq!(spawn[..3], [60.0 + 1.0 / 3.0, 0.0, 60.0 + 1.0 / 3.0]);
        let enc: Vec<_> = t.iter().map(|t| (t.v, 0u16, 0u16)).collect();
        let bytes = pack::write(&[
            data(
                "park/collision",
                Kind::Collision,
                game_core::collision::encode(&enc),
            ),
            data(
                "park/spawn",
                Kind::Data,
                spawn.iter().flat_map(|f| f.to_be_bytes()).collect(),
            ),
            data("park/rails", Kind::Data, game_core::rails::encode(&[])),
        ]);
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let park = game_core::world::ImportedPark::parse(bytes).unwrap();
        assert_eq!(park.mesh.len(), 2);

        let dir = std::env::temp_dir().join(format!("fg_district_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index {
            cell: CELL,
            floor: -1.0,
            starts: vec![(v3(1.0, 2.0, 3.0), 0.5)],
            cells: vec![CellInfo {
                x: 50,
                z: 50,
                bytes: bytes.len() as u32,
                tris: 2,
                vertices: 0,
                texture_bytes: 0,
            }],
        };
        write_index(&dir, &index).unwrap();
        let file = std::fs::read(dir.join("index.fgpak")).unwrap();
        let pack = game_core::pack::Pack::parse(&file).unwrap();
        assert_eq!(
            Index::parse(pack.find("city/index").unwrap().data),
            Some(index)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
