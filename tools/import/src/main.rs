//! fg_import: reads a Skate 3 (Xbox 360) disc image the player owns and converts what flatground can
//! use into `.fgpak` packs. It reads only plain disc files (no executable decryption), ships no game
//! data, and writes only where it is told (default: `imported/`, git-ignored).
//!
//! Format knowledge comes from SK8-ENGINE/skate-3-rust-engine (GPL-3.0); this tool is GPL-3.0-only too.

mod anim;
mod assets;
mod attrib;
mod big;
mod cac;
mod camera;
mod gx;
mod mesh;
mod motion;
mod pack;
mod park;
mod parkmodel;
mod raster;
mod refpack;
mod rules;
mod rw4;
mod skater;
mod skin;
mod stategraph;
mod texture;
mod world;
mod xdvdfs;
mod xex;

use big::Big;
use std::fs::File;
use std::io::{BufWriter, Read, Seek};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use xdvdfs::{Disc, DiscFile};

const USAGE: &str = "usage:
  fg_import build    [options]                    convert what the game loads into the app folder (only
                                                  what is missing or out of date; `fg_import build -h`)
  fg_import ls       <disc.iso> [dir]             list a directory on the disc
  fg_import tree     <disc.iso>                   list every file on the disc with its size
  fg_import big      <disc.iso> <file.big>        list the entries of a BIG archive on the disc
  fg_import rx2      <disc.iso> <asset>           list the sections and textures of an RW4 arena
  fg_import cat      <disc.iso> <asset> <out>     copy one asset off the disc (for format research)
  fg_import textures <disc.iso> [options]         convert textures into a Wii pack
      --from <prefix>   disc path or BIG to scan (repeatable; default: data/content/global_locators)
      --out <dir>       output directory (default: imported)
      --max <n>         largest texture edge kept on the Wii (default 256, at most 1024)
      --png             also write PNG previews (for checking on the PC)
      --rgba8           keep textures with alpha at full 8-bit precision (twice the memory of RGB5A3)
  fg_import park     <disc.iso> [options]         a skate park district (collision, rails, spawn, textured
                                                  models) as park.fgpak
      --park <name>     StartPark, MegaPark, BlackBoxPark (default), IndustrialSkatePark,
                        DownTownSkatePark, SkateSchool or MaloofMoneyCup
      --max <n>         largest texture edge for the models (default 128; 0: no models)
      --out <dir>       output directory (default: imported)
  fg_import rules    <disc.iso> [--out <dir>]     Skate 3's riding tuning and flick shapes as rules.fgpak
  fg_import skater   <disc.iso> [options]         the default skater as skinned meshes + textures
      --lod <0|2>       model detail (default 0)
      --png             front/side previews; --clip <name> [--frame <n>] also renders that clip
      --anim <role>=<clip>  pick the clip the game plays for a role (docs/importing.md); <role>= drops it
      --max <n>, --out <dir>  as for textures
  fg_import motion   <disc.iso> [options]         the on-board motion library as motion.fgpak: clip timing,
                                                  attribute tracks and events, animation trees, and the
                                                  poses of the clips the on-board motion graph plays
      --budget <MiB>    pose memory budget (default 20); graph areas that don't fit are left out
      --list            also list every attribute name
      --out <dir>       output directory (default: imported)
  fg_import anims    <disc.iso> [options]         convert skeletons and animation clips (VBR banks)
      --from <asset>    a .abin, or a .big to take every .abin from (repeatable;
                        default: data/big/miscload.big::data/anim/OnBoard.abin)
      --clips <text>    only clips whose name contains <text> (repeatable, case-insensitive)
      --additive        keep clips relative to the bind pose (default: compose RIG_TPOSE in)
      --out <dir>       output directory (default: imported)
  fg_import info     <file.fgpak> [entry] [name]  list a pack, or show one entry in detail (motion/db: a
                                                  clip's or tree's data by name)
  fg_import xex-tables <image> [--base <hex>] [--out <dir>]
                        water (ocean PCA) and front-end menu tables from an executable image YOU have
                        already decrypted: an unencrypted XEX2, or the mapped PE image (MZ) loaded at
                        --base (default 82000000). fg_import never decrypts executables.

<asset> is a disc path, or `<file.big>::<entry>` for a file inside a BIG archive.
Output is converted from your own disc and is for your own use. Do not share or commit it.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn run(args: &[String]) -> Res<()> {
    let (cmd, rest) = args.split_first().ok_or(USAGE)?;
    if cmd == "-h" || cmd == "--help" {
        println!("{USAGE}");
        return Ok(());
    }
    if cmd == "build" {
        return assets::run(rest);
    }
    if cmd == "info" {
        return info(rest);
    }
    if cmd == "xex-tables" {
        return xex_tables(rest);
    }
    let iso = PathBuf::from(rest.first().ok_or(USAGE)?);
    let mut disc = Disc::open(&iso)?;
    let arg = |i: usize| rest.get(i).map(String::as_str).ok_or(USAGE);
    match cmd.as_str() {
        "ls" => {
            eprintln!("XDVDFS partition at {:#x}", disc.partition_offset());
            let dir = match rest.get(1) {
                Some(p) => disc.find(p)?,
                None => disc.root().clone(),
            };
            for e in disc.read_dir(&dir)? {
                println!(
                    "{:>12} {}{}",
                    e.size,
                    e.name,
                    if e.is_dir { "/" } else { "" }
                );
            }
        }
        "tree" => {
            let root = disc.root().clone();
            disc.walk(&root, "", &mut |p, e| {
                if !e.is_dir {
                    println!("{:>12} {p}", e.size);
                }
            })?;
        }
        "big" => {
            let entry = disc.find(arg(1)?)?;
            let big = Big::new(DiscFile {
                disc: &mut disc,
                entry,
            })?;
            for e in &big.entries {
                println!(
                    "{:>10} {:>10} c{} {}",
                    e.unpacked, e.stored, e.compression, e.path
                );
            }
            println!("{} entries", big.entries.len());
        }
        "rx2" => {
            let bytes = read_asset(&mut disc, arg(1)?)?;
            let arena = rw4::Arena::parse(&bytes)?;
            println!(
                "arena type {:#x}, data base {:#x}, {} sections",
                arena.arena_type,
                arena.data_base,
                arena.sections.len()
            );
            for s in &arena.sections {
                println!(
                    "  #{:<4} {:#010x} {:<26} info {:#x} size {}",
                    s.index,
                    s.type_id,
                    rw4::type_name(s.type_id),
                    s.info,
                    s.size
                );
            }
            for t in texture::textures(&arena) {
                match t {
                    Ok(t) => println!(
                        "  texture #{}: {}x{} {:?}{}",
                        t.section,
                        t.info.width,
                        t.info.height,
                        t.info.format,
                        if t.info.tiled { "" } else { " (linear)" }
                    ),
                    Err(e) => println!("  texture: {e}"),
                }
            }
        }
        "cat" => {
            let bytes = read_asset(&mut disc, arg(1)?)?;
            std::fs::write(arg(2)?, bytes)?;
        }
        "textures" => textures(&mut disc, &rest[1..])?,
        "anims" => anims(&mut disc, &rest[1..])?,
        "skater" => skater::run(&mut disc, &rest[1..])?,
        "motion" => motion::run(&mut disc, &rest[1..])?,
        "park" => park::run(&mut disc, &rest[1..])?,
        "rules" => rules::run(&mut disc, &rest[1..])?,
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

/// Reads a disc file, or `archive.big::entry` from inside a BIG on the disc.
pub fn read_asset<R: Read + Seek>(disc: &mut Disc<R>, spec: &str) -> Res<Vec<u8>> {
    match spec.split_once("::") {
        Some((archive, inner)) => {
            let entry = disc.find(archive)?;
            let mut big = Big::new(DiscFile { disc, entry })?;
            let e = big
                .find(inner)
                .ok_or_else(|| format!("{archive}: no entry {inner}"))?
                .clone();
            Ok(big.read(&e)?)
        }
        None => {
            let e = disc.find(spec)?;
            Ok(disc.read_file(&e)?)
        }
    }
}

struct Options {
    from: Vec<String>,
    out: PathBuf,
    max: usize,
    png: bool,
    rgba8: bool,
}

fn parse_options(args: &[String]) -> Res<Options> {
    let mut o = Options {
        from: Vec::new(),
        out: PathBuf::from("imported"),
        max: 256,
        png: false,
        rgba8: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => o.from.push(it.next().ok_or(USAGE)?.clone()),
            "--out" => o.out = PathBuf::from(it.next().ok_or(USAGE)?),
            "--max" => o.max = it.next().ok_or(USAGE)?.parse::<usize>()?.clamp(4, 1024),
            "--png" => o.png = true,
            "--rgba8" => o.rgba8 = true,
            _ => return Err(format!("unknown option {a}\n{USAGE}").into()),
        }
    }
    if o.from.is_empty() {
        o.from.push("data/content/global_locators".into());
    }
    Ok(o)
}

/// Every RW4 arena under the given sources: (name, bytes). A source is a disc directory, a disc file, or a
/// BIG archive (all of its `.rx2` entries).
fn collect_arenas<R: Read + Seek>(
    disc: &mut Disc<R>,
    from: &[String],
    f: &mut dyn FnMut(&str, Vec<u8>),
) -> Res<()> {
    for src in from {
        let e = disc.find(src)?;
        if e.is_dir {
            let mut files = Vec::new();
            disc.walk(&e, src.trim_end_matches('/'), &mut |p, e| {
                if !e.is_dir && p.to_ascii_lowercase().ends_with(".rx2") {
                    files.push((p.to_string(), e.clone()));
                }
            })?;
            for (p, e) in files {
                f(&p, disc.read_file(&e)?);
            }
        } else if src.to_ascii_lowercase().ends_with(".big") {
            let mut big = Big::new(DiscFile { disc, entry: e })?;
            let entries: Vec<_> = big
                .entries
                .iter()
                .filter(|e| e.path.to_ascii_lowercase().ends_with(".rx2"))
                .cloned()
                .collect();
            for e in entries {
                match big.read(&e) {
                    Ok(b) => f(&format!("{src}::{}", e.path), b),
                    Err(err) => eprintln!("skip {src}::{}: {err}", e.path),
                }
            }
        } else {
            f(src, disc.read_file(&e)?);
        }
    }
    Ok(())
}

/// Picks the GX format and encodes. DXT1 sources that fit go to CMPR losslessly.
fn to_gx(
    t: &texture::Texture,
    max: usize,
    rgba8: bool,
) -> Result<(texture::Rgba, u8, Vec<u8>), String> {
    let rgba = texture::decode(t.data, &t.info)?;
    if t.info.format == texture::Format::Dxt1
        && t.info.width <= max
        && t.info.height <= max
        && !rgba.has_alpha()
    {
        let blocks = texture::dxt1_blocks(t.data, &t.info)?;
        let data = gx::cmpr_from_dxt1(&blocks, t.info.width.div_ceil(4), t.info.height.div_ceil(4));
        return Ok((rgba, gx::GX_TF_CMPR, data));
    }
    let small = gx::shrink_to(rgba, max);
    if small.has_alpha() && rgba8 {
        let data = gx::encode_rgba8(&small);
        Ok((small, gx::GX_TF_RGBA8, data))
    } else if small.has_alpha() {
        let data = gx::encode_rgb5a3(&small);
        Ok((small, gx::GX_TF_RGB5A3, data))
    } else {
        let data = gx::encode_cmpr(&small);
        Ok((small, gx::GX_TF_CMPR, data))
    }
}

fn safe_file_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn textures<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    let o = parse_options(args)?;
    std::fs::create_dir_all(&o.out)?;
    let png_dir = o.out.join("png");
    if o.png {
        std::fs::create_dir_all(&png_dir)?;
    }
    let mut items = Vec::new();
    let mut manifest = String::new();
    let (mut skipped, mut arenas) = (0usize, 0usize);
    collect_arenas(disc, &o.from, &mut |name, bytes| {
        arenas += 1;
        let arena = match rw4::Arena::parse(&bytes) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("skip {name}: {e}");
                return;
            }
        };
        for (i, t) in texture::textures(&arena).into_iter().enumerate() {
            let id = format!("{name}#{i}");
            let res = t.and_then(|t| to_gx(&t, o.max, o.rgba8).map(|r| (t.info, r)));
            match res {
                Ok((info, (img, fmt, data))) => {
                    manifest.push_str(&format!(
                        "{id}\t{:?} {}x{} -> {} {}x{}\n",
                        info.format,
                        info.width,
                        info.height,
                        gx::name(fmt),
                        img.width,
                        img.height
                    ));
                    if o.png
                        && let Err(e) =
                            write_png(&png_dir.join(format!("{}.png", safe_file_name(&id))), &img)
                    {
                        eprintln!("png {id}: {e}");
                    }
                    items.push(pack::Item {
                        name: id,
                        kind: game_core::pack::Kind::Texture,
                        data,
                        width: img.width as u16,
                        height: img.height as u16,
                        gx_format: fmt,
                    });
                }
                Err(e) => {
                    skipped += 1;
                    eprintln!("skip {id}: {e}");
                }
            }
        }
    })?;
    let bytes = pack::write(&items);
    let path = o.out.join("textures.fgpak");
    std::fs::write(&path, &bytes)?;
    std::fs::write(o.out.join("textures.txt"), manifest)?;
    println!(
        "{} textures from {arenas} arenas ({skipped} skipped) -> {} ({} KiB)",
        items.len(),
        path.display(),
        bytes.len() / 1024
    );
    Ok(())
}

fn write_png(path: &Path, img: &texture::Rgba) -> Res<()> {
    let mut enc = png::Encoder::new(
        BufWriter::new(File::create(path)?),
        img.width as u32,
        img.height as u32,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&img.pixels)?;
    Ok(())
}

fn xex_tables(args: &[String]) -> Res<()> {
    let path = args.first().ok_or(USAGE)?;
    let (mut base, mut out) = (xex::DEFAULT_BASE, PathBuf::from("imported"));
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base" => {
                base = u32::from_str_radix(it.next().ok_or(USAGE)?.trim_start_matches("0x"), 16)?
            }
            "--out" => out = PathBuf::from(it.next().ok_or(USAGE)?),
            _ => return Err(format!("unknown option {a}\n{USAGE}").into()),
        }
    }
    let image = xex::XexImage::load(&std::fs::read(path)?, base)?;
    println!(
        "image: {} bytes at {:#010x}",
        image.image.len(),
        image.base_address
    );
    let mut items = Vec::new();
    // Each table is optional: report what was not found and keep the rest.
    match xex::ocean_pca::extract(&image) {
        Ok(pca) => {
            println!(
                "ocean PCA: {} frames at {:#010x}",
                pca.frames.len(),
                pca.means_address
            );
            items.push(data_item("xex/ocean_pca", pca.to_bytes()));
        }
        Err(e) => eprintln!("ocean PCA: {e}"),
    }
    match xex::menu_tables::extract(&image) {
        Ok(menus) => {
            println!(
                "menu tables: {} settings, {} screens, {} categories, {} items, {} keys, {} modes",
                menus.settings_rows.len(),
                menus.settings_screens.len(),
                menus.categories.len(),
                menus.items.len(),
                menus.keys.len(),
                menus.modes.len()
            );
            for (name, at) in &menus.addresses {
                println!("  {name} at {at}");
            }
            items.push(data_item("xex/menu_tables", menus.to_text().into_bytes()));
        }
        Err(e) => eprintln!("menu tables: {e}"),
    }
    if items.is_empty() {
        return Err("no tables found in this image".into());
    }
    std::fs::create_dir_all(&out)?;
    let path = out.join("xex.fgpak");
    std::fs::write(&path, pack::write(&items))?;
    println!("-> {}", path.display());
    Ok(())
}

fn data_item(name: &str, data: Vec<u8>) -> pack::Item {
    pack::Item {
        name: name.into(),
        kind: game_core::pack::Kind::Data,
        data,
        width: 0,
        height: 0,
        gx_format: 0,
    }
}

fn anims<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    let (mut from, mut filters, mut out, mut additive) =
        (Vec::new(), Vec::new(), PathBuf::from("imported"), false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => from.push(it.next().ok_or(USAGE)?.clone()),
            "--clips" => filters.push(it.next().ok_or(USAGE)?.to_ascii_uppercase()),
            "--out" => out = PathBuf::from(it.next().ok_or(USAGE)?),
            "--additive" => additive = true,
            _ => return Err(format!("unknown option {a}\n{USAGE}").into()),
        }
    }
    if from.is_empty() {
        from.push("data/big/miscload.big::data/anim/OnBoard.abin".into());
    }
    // Expand a bare .big into all of its .abin entries.
    let mut banks = Vec::new();
    for src in from {
        if src.to_ascii_lowercase().ends_with(".big") {
            let entry = disc.find(&src)?;
            let big = Big::new(DiscFile {
                disc: &mut *disc,
                entry,
            })?;
            banks.extend(
                big.entries
                    .iter()
                    .filter(|e| e.path.to_ascii_lowercase().ends_with(".abin"))
                    .map(|e| format!("{src}::{}", e.path)),
            );
        } else {
            banks.push(src);
        }
    }
    let mut items = Vec::new();
    let mut manifest = String::new();
    let (mut clips, mut skipped) = (0usize, 0usize);
    for spec in &banks {
        let stem = spec
            .rsplit(['/', ':'])
            .next()
            .unwrap_or(spec)
            .trim_end_matches(".abin")
            .to_ascii_lowercase();
        let bank = match anim::abin::Bank::parse(read_asset(disc, spec)?) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("skip {spec}: {e}");
                continue;
            }
        };
        let Some(hierarchy) = bank.hierarchy() else {
            eprintln!("skip {spec}: no skeleton hierarchy");
            continue;
        };
        items.push(pack::Item {
            name: format!("anim/{stem}/skeleton"),
            kind: game_core::pack::Kind::Skeleton,
            data: anim::encode_skeleton(hierarchy),
            width: 0,
            height: 0,
            gx_format: 0,
        });
        manifest.push_str(&format!(
            "anim/{stem}/skeleton\t{} bones\n",
            hierarchy.bone_count
        ));
        // Clips are stored relative to the bank's bind pose; compose it in so the game plays plain local
        // transforms (unless --additive asks for the raw deltas).
        let bind = if additive {
            None
        } else {
            anim::bind_pose(&bank)
        };
        if !additive && bind.is_none() {
            eprintln!("{spec}: no {} pose, clips stay relative", anim::BIND_POSE);
        }
        for record in bank.records() {
            let lower = record.header.name.to_ascii_lowercase();
            // Poses are one frame each and referenced by other animations, so they always come along.
            let (name, decoded) = match record.data {
                anim::abin::RecordData::Pose(_) => (
                    format!("anim/{stem}/pose/{lower}"),
                    anim::decode_pose(&bank, record),
                ),
                anim::abin::RecordData::Clip(_)
                    if filters.is_empty()
                        || filters
                            .iter()
                            .any(|f| record.header.name.contains(f.as_str())) =>
                {
                    (
                        format!("anim/{stem}/{lower}"),
                        anim::decode_clip(&bank, record),
                    )
                }
                _ => continue,
            };
            match decoded {
                Ok(mut c) => {
                    if let (Some(b), anim::abin::RecordData::Clip(_)) = (&bind, &record.data) {
                        anim::add_bind_pose(&mut c, b);
                    }
                    let data = anim::encode_clip(&c);
                    manifest.push_str(&format!(
                        "{name}\t{} frames @ {} fps{}\t{} bytes\n",
                        c.frames.len(),
                        c.fps,
                        if c.looping { ", loop" } else { "" },
                        data.len()
                    ));
                    clips += 1;
                    items.push(pack::Item {
                        name,
                        kind: game_core::pack::Kind::Animation,
                        data,
                        width: 0,
                        height: 0,
                        gx_format: 0,
                    });
                }
                Err(e) => {
                    skipped += 1;
                    eprintln!("skip {name}: {e}");
                }
            }
        }
    }
    std::fs::create_dir_all(&out)?;
    let bytes = pack::write(&items);
    let path = out.join("anims.fgpak");
    std::fs::write(&path, &bytes)?;
    std::fs::write(out.join("anims.txt"), manifest)?;
    println!(
        "{clips} clips from {} banks ({skipped} skipped) -> {} ({} KiB)",
        banks.len(),
        path.display(),
        bytes.len() / 1024
    );
    Ok(())
}

fn info(args: &[String]) -> Res<()> {
    use game_core::{anim, pack::Pack};
    let bytes = std::fs::read(args.first().ok_or(USAGE)?)?;
    let pack = Pack::parse(&bytes).map_err(|e| format!("{e:?}"))?;
    let Some(name) = args.get(1) else {
        for e in pack.iter() {
            println!("{:>9} kind {} {}", e.data.len(), e.kind, e.name);
        }
        println!("{} entries", pack.len());
        return Ok(());
    };
    let e = pack.find(name).ok_or("no such entry")?;
    match e.kind {
        3 => {
            let s = anim::Skeleton::parse(e.data).map_err(|_| "truncated skeleton")?;
            println!("{} bones, trajectory {}", s.bones(), s.has_trajectory());
            for b in 0..s.bones() {
                println!(
                    "  {b:>3} {:<24} parent {:>3} mirror {:>3}",
                    s.name(b),
                    s.parent(b),
                    s.mirror(b)
                );
            }
        }
        4 => {
            let c = anim::Clip::parse(e.data).map_err(|_| "truncated clip")?;
            println!(
                "{} bones, {} frames @ {} fps, loop {}",
                c.bones(),
                c.frames(),
                c.fps(),
                c.looping()
            );
            for f in [0, c.frames() / 2, c.frames() - 1] {
                println!("frame {f}:");
                for b in 0..c.bones().min(6) {
                    let s = c.sample(f, b);
                    println!(
                        "  bone {b}: rot {:.3?} pos {:.3?} scale {:.3?}",
                        s.rotation, s.translation, s.scale
                    );
                }
            }
        }
        7 => {
            let db = game_core::motion::MotionDb::parse(e.data).map_err(|e| format!("{e:?}"))?;
            println!(
                "{} clips, {} trees, {} attribute records, {} names",
                db.clips().len(),
                db.trees().len(),
                db.attribute_count(),
                db.names()
            );
            // Optional third argument: one clip or tree in detail.
            if let Some(n) = args.get(2) {
                match db.find(n).ok_or("no such clip or tree")? {
                    game_core::motion::Node::Clip(i) => {
                        let c = db.clip(i);
                        println!(
                            "clip {}: {} frames @ {} fps, base speed {}, {:.3} s, loop {}, pose {}",
                            db.name(c.name),
                            c.frames,
                            c.fps,
                            c.base_speed,
                            c.length(),
                            c.looping(),
                            c.has_pose
                        );
                        for a in db.attributes(i) {
                            println!(
                                "  {:<30} kind {} {:>7.3}..{:<7.3} {:?}",
                                db.name(a.name),
                                a.kind,
                                a.begin,
                                a.end,
                                db.words(a)
                                    .iter()
                                    .take(8)
                                    .map(|w| f32::from_bits(*w))
                                    .collect::<Vec<_>>()
                            );
                        }
                    }
                    game_core::motion::Node::Tree(i) => print_tree(&db, i),
                    game_core::motion::Node::Missing(_) => {}
                }
            }
        }
        _ => println!(
            "{} bytes, kind {}, {}x{}, GX format {:#x}",
            e.data.len(),
            e.kind,
            e.width,
            e.height,
            e.gx_format
        ),
    }
    Ok(())
}

/// One animation tree of a motion database, with names.
fn print_tree(db: &game_core::motion::MotionDb, i: u16) {
    use game_core::motion::{Node, TreeDef};
    let node = |n: &Node| match *n {
        Node::Clip(c) => db.name(db.clip(c).name).to_string(),
        Node::Tree(t) => format!("{} (tree)", db.name(db.tree(t).name)),
        Node::Missing(n) => format!("{} (missing)", db.name(n)),
    };
    match &db.tree(i).def {
        TreeDef::PhaseBlend {
            parameter,
            children,
        } => {
            println!("phase blend by {}:", db.name(*parameter));
            children.iter().for_each(|c| println!("  {}", node(c)));
        }
        TreeDef::BlendSpace {
            parameters,
            children,
            simplexes,
        } => {
            let p: Vec<&str> = parameters.iter().map(|p| db.name(*p)).collect();
            println!("blend space over {p:?}, {} simplexes:", simplexes.len());
            children.iter().for_each(|c| println!("  {}", node(c)));
        }
        TreeDef::Selector {
            parameter,
            default,
            options,
        } => {
            println!(
                "selector by {} (default {}):",
                db.name(*parameter),
                node(default)
            );
            for (v, c) in options {
                println!("  {} -> {}", db.name(*v), node(c));
            }
        }
        TreeDef::SelectionSpace {
            parameters,
            candidates,
        } => {
            let p: Vec<&str> = parameters.iter().map(|p| db.name(p.name)).collect();
            println!("selection space over {p:?}:");
            for (c, v) in candidates {
                println!("  {} at {v:?}", node(c));
            }
        }
    }
}
