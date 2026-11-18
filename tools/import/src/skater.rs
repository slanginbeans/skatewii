//! `fg_import skater`: the default create-a-character skater (the disc's fallback recipe) as Wii skinned
//! meshes with their diffuse textures, plus PC preview renders to check the conversion.

use crate::anim::{self, abin::Bank};
use crate::big::Big;
use crate::xdvdfs::{Disc, DiscFile};
use crate::{Res, USAGE, cac, gx, mesh, pack, raster, read_asset, rw4, skin, texture, write_png};
use game_core::anim::{Clip, Skeleton};
use game_core::pack::Kind;
use std::io::{Read, Seek};
use std::path::PathBuf;

const RECIPE: &str = "data/big/miscload.big::data/cacrecipes/SavedRecipeFallbackMale.bin";
const ARCHIVE: &str = "data/content/createacharacter.big";
const BANK: &str = "data/big/miscload.big::data/anim/OnBoard.abin";

/// Clips the game plays, by role (`skater/clip/<role>` in the pack), with their default OnBoard clip.
/// `--anim role=clip` picks another, `--anim role=` drops one. Tricks play `pop` (leaving the ground), `air`
/// (into the trick), `flip` and `out` (the catch) back to back, skipping the ones that are empty.
pub const CLIPS: &[(&str, &str)] = &[
    ("idle", "r_stand_idle_mongo_0_cyc"),
    ("crouch", "r_antic_crouch_ollie_n_0_into"),
    ("push", "r_pushlsp_lstr_mongo_left_cyc2"),
    ("ollie/pop", "ollie_low_g"),
    ("ollie/air", "ollie_low_a"),
    ("ollie/flip", ""),
    ("ollie/out", ""),
    ("nollie/pop", "nollie_low_g"),
    ("nollie/air", "nollie_low_a"),
    ("nollie/flip", ""),
    ("nollie/out", ""),
    ("kickflip/pop", "kickflip_in_low_g"),
    ("kickflip/air", "kickflip_in_low_a"),
    ("kickflip/flip", "t_low_kick_cyc1"),
    ("kickflip/out", "t_low_kick_out4"),
    ("heelflip/pop", "heelflip_in_low_g"),
    ("heelflip/air", "heelflip_in_low_a"),
    ("heelflip/flip", "t_low_heel_cyc1"),
    ("heelflip/out", "t_low_heel_out4"),
    ("popshuvit/pop", "popshuvit_low_g"),
    ("popshuvit/air", "popshuvit_low_a"),
    ("popshuvit/flip", ""),
    ("popshuvit/out", ""),
    ("fsshuvit/pop", "fspopshuvit_d_low_g"),
    ("fsshuvit/air", "fspopshuvit_d_low_a"),
    ("fsshuvit/flip", ""),
    ("fsshuvit/out", ""),
];

struct Options {
    lod: u8,
    clip: Option<String>,
    frame: Option<usize>,
    out: PathBuf,
    png: bool,
    max: usize,
    /// (role, clip) overrides of [`CLIPS`].
    anims: Vec<(String, String)>,
}

fn options(args: &[String]) -> Res<Options> {
    let mut o = Options {
        lod: 0,
        clip: None,
        frame: None,
        out: PathBuf::from("imported"),
        png: false,
        max: 256,
        anims: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or(USAGE);
        match a.as_str() {
            "--lod" => o.lod = val()?.parse()?,
            "--clip" => o.clip = Some(val()?.to_ascii_lowercase()),
            "--frame" => o.frame = Some(val()?.parse()?),
            "--out" => o.out = PathBuf::from(val()?),
            "--max" => o.max = val()?.parse::<usize>()?.clamp(4, 1024),
            "--png" => o.png = true,
            "--anim" => {
                let v = val()?;
                let (role, clip) = v.split_once('=').ok_or("--anim takes role=clip")?;
                if !CLIPS.iter().any(|(r, _)| r.eq_ignore_ascii_case(role)) {
                    let roles: Vec<&str> = CLIPS.iter().map(|c| c.0).collect();
                    return Err(format!("unknown role {role}; roles: {}", roles.join(", ")).into());
                }
                o.anims
                    .push((role.to_ascii_lowercase(), clip.to_ascii_lowercase()));
            }
            _ => return Err(format!("unknown option {a}\n{USAGE}").into()),
        }
    }
    Ok(o)
}

struct Part {
    slot: String,
    model: mesh::Model,
    diffuse: Option<texture::Rgba>,
    alpha_test: bool,
}

fn decode_texture<S: crate::big::ReadAt>(
    big: &mut Big<S>,
    id: &str,
) -> Result<texture::Rgba, String> {
    let path = format!("data/content/createacharacter/texture/0x{id}.rx2");
    let e = big
        .find(&path)
        .cloned()
        .ok_or_else(|| format!("missing {path}"))?;
    let bytes = big.read(&e).map_err(|e| e.to_string())?;
    let arena = rw4::Arena::parse(&bytes).map_err(|e| e.to_string())?;
    let t = texture::textures(&arena)
        .into_iter()
        .next()
        .ok_or("no texture in arena")??;
    texture::decode(t.data, &t.info)
}

/// A clip from the bank with the bind pose composed in, encoded for `game_core::anim::Clip`.
fn load_clip(bank: &Bank, name: &str) -> Res<Vec<u8>> {
    let record = bank
        .records()
        .iter()
        .find(|r| {
            r.header.name.eq_ignore_ascii_case(name)
                && matches!(r.data, anim::abin::RecordData::Clip(_))
        })
        .ok_or_else(|| format!("no clip {name} in OnBoard.abin"))?;
    let mut frames = anim::decode_clip(bank, record)?;
    anim::add_bind_pose(
        &mut frames,
        &anim::bind_pose(bank).ok_or("bank has no RIG_TPOSE bind pose")?,
    );
    Ok(anim::encode_clip(&frames))
}

pub fn run<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    let o = options(args)?;
    let recipe = cac::parse(&read_asset(disc, RECIPE)?)?;
    let bank = Bank::parse(read_asset(disc, BANK)?)?;
    let hierarchy = bank.hierarchy().ok_or("animation bank has no skeleton")?;
    let skeleton_bytes = anim::encode_skeleton(hierarchy);
    let skeleton = Skeleton::parse(&skeleton_bytes).map_err(|_| "skeleton")?;

    let entry = disc.find(ARCHIVE)?;
    let mut big = Big::new(DiscFile { disc, entry })?;
    let mut parts = Vec::new();
    for (slot, assets) in &recipe.slots {
        let Some(asset) = assets.first() else {
            continue;
        };
        // The requested LOD, else the closest one.
        let Some(m) = asset
            .models
            .iter()
            .min_by_key(|m| (m.lod as i32 - o.lod as i32).abs())
        else {
            continue;
        };
        let path = format!(
            "data/content/createacharacter/model/cas_db/{slot}/0x{}.rx2",
            m.model_id
        );
        let e = big
            .find(&path)
            .cloned()
            .ok_or_else(|| format!("missing {path}"))?;
        let bytes = big.read(&e)?;
        let model =
            mesh::read_model(&rw4::Arena::parse(&bytes)?).map_err(|e| format!("{slot}: {e}"))?;
        let tex = |name: &str| m.textures.iter().find(|t| t.0 == name).map(|t| t.1.clone());
        let mut diffuse = match tex("diffuse") {
            Some(id) => decode_texture(&mut big, &id)
                .map_err(|e| eprintln!("{slot} diffuse: {e}"))
                .ok(),
            None => None,
        };
        // Hair keeps its coverage in a separate alpha texture; fold it into the diffuse alpha.
        let mut alpha_test = false;
        if let (Some(d), Some(id)) = (diffuse.as_mut(), tex("alpha"))
            && let Ok(a) = decode_texture(&mut big, &id)
            && (a.width, a.height) == (d.width, d.height)
        {
            for (px, ap) in d.pixels.chunks_exact_mut(4).zip(a.pixels.chunks_exact(4)) {
                px[3] = ap[0];
            }
            alpha_test = true;
        }
        let tris: usize = model.meshes.iter().map(|m| m.indices.len() / 3).sum();
        let verts: usize = model.meshes.iter().map(|m| m.positions.len()).sum();
        println!(
            "{slot}: LOD {} - {verts} vertices, {tris} triangles, {} bones{}",
            m.lod,
            model.bones.len(),
            diffuse
                .as_ref()
                .map(|d| format!(", diffuse {}x{}", d.width, d.height))
                .unwrap_or_default()
        );
        parts.push(Part {
            slot: slot.clone(),
            model,
            diffuse,
            alpha_test,
        });
    }

    let models: Vec<&mesh::Model> = parts.iter().map(|p| &p.model).collect();
    let rest = anim::bind_pose(&bank);
    let rig = skin::Rig::new(&skeleton, &models, rest.as_deref());
    let mut unbound = Vec::new();
    for p in &parts {
        for (bone, driver) in skin::coverage(&rig, &p.model) {
            if driver.is_none() {
                unbound.push(format!("{}:{bone}", p.slot));
            }
        }
    }
    if !unbound.is_empty() {
        unbound.sort();
        eprintln!(
            "bones with no animated ancestor (bound to the root): {}",
            unbound.join(", ")
        );
    }

    let mut items = vec![pack::Item {
        name: "skater/skeleton".into(),
        kind: Kind::Skeleton,
        data: skeleton_bytes.clone(),
        width: 0,
        height: 0,
        gx_format: 0,
    }];
    let mut bindings = Vec::new();
    for p in &parts {
        let b = rig.bindings(&p.model);
        let slot = p.slot.to_ascii_lowercase();
        items.push(pack::Item {
            name: format!("skater/{slot}"),
            kind: Kind::SkinnedMesh,
            data: skin::encode_mesh(&p.model, &b, p.alpha_test)?,
            width: 0,
            height: 0,
            gx_format: 0,
        });
        if let Some(d) = &p.diffuse {
            let mut small = gx::shrink_to(d.clone(), o.max);
            // Only coverage alpha (hair) is kept; other diffuse alpha channels hold masks the Wii won't use.
            let (fmt, data) = if p.alpha_test {
                (gx::GX_TF_RGB5A3, gx::encode_rgb5a3(&small))
            } else {
                small.pixels.chunks_exact_mut(4).for_each(|px| px[3] = 255);
                (gx::GX_TF_CMPR, gx::encode_cmpr(&small))
            };
            items.push(pack::Item {
                name: format!("skater/{slot}/diffuse"),
                kind: Kind::Texture,
                data,
                width: small.width as u16,
                height: small.height as u16,
                gx_format: fmt,
            });
        }
        bindings.push(b);
    }
    for (role, default) in CLIPS {
        let name = o
            .anims
            .iter()
            .rev()
            .find(|(r, _)| r == role)
            .map_or(*default, |(_, c)| c.as_str());
        if name.is_empty() {
            continue;
        }
        match load_clip(&bank, name) {
            Ok(data) => {
                let clip = Clip::parse(&data).map_err(|_| "clip")?;
                // How upright the deck starts and ends (1 = grip up): catches should finish level.
                let deck = rig
                    .names
                    .iter()
                    .position(|n| n.to_ascii_lowercase().starts_with("skateboard"));
                let upright = deck
                    .map(|d| {
                        (
                            rig.pose(&clip, 0)[d][1][1],
                            rig.pose(&clip, clip.frames() - 1)[d][1][1],
                        )
                    })
                    .map(|(a, b)| format!(", deck upright {a:.2} -> {b:.2}"))
                    .unwrap_or_default();
                println!("clip {role}: {name}, {} frames{upright}", clip.frames());
                items.push(pack::Item {
                    name: format!("skater/clip/{role}"),
                    kind: Kind::Animation,
                    data,
                    width: 0,
                    height: 0,
                    gx_format: 0,
                });
            }
            Err(e) => eprintln!("clip {role}: {e} (skipped)"),
        }
    }
    std::fs::create_dir_all(&o.out)?;
    let bytes = pack::write(&items);
    let path = o.out.join("skater.fgpak");
    std::fs::write(&path, &bytes)?;
    println!(
        "{} parts -> {} ({} KiB)",
        parts.len(),
        path.display(),
        bytes.len() / 1024
    );

    if o.png {
        let png_dir = o.out.join("png");
        std::fs::create_dir_all(&png_dir)?;
        render(
            &parts,
            &bindings,
            &rig.bind_pose(),
            &png_dir.join("skater_bind.png"),
        )?;
        if let Some(name) = &o.clip {
            let clip_bytes = load_clip(&bank, name)?;
            let clip = Clip::parse(&clip_bytes).map_err(|_| "clip")?;
            let frames: Vec<usize> = match o.frame {
                Some(f) => vec![f.min(clip.frames() - 1)],
                None => vec![0, clip.frames() / 3, 2 * clip.frames() / 3],
            };
            for f in frames {
                let file = png_dir.join(format!("skater_{name}_{f}.png"));
                render(&parts, &bindings, &rig.pose(&clip, f), &file)?;
                println!("preview {}", file.display());
            }
        }
        println!("preview {}", png_dir.join("skater_bind.png").display());
    }
    Ok(())
}

/// Front and side views next to each other.
fn render(
    parts: &[Part],
    bindings: &[Vec<(usize, mesh::Mat4)>],
    world: &[mesh::Mat4],
    path: &std::path::Path,
) -> Res<()> {
    let (w, h) = (512, 512);
    let mut canvas = raster::Canvas::new(2 * w, h, [40, 44, 52]);
    let skinned: Vec<_> = parts
        .iter()
        .zip(bindings)
        .map(|(p, b)| skin::skin(&p.model, b, world))
        .collect();
    // Centre on the posed character (clips can carry root motion).
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in skinned.iter().flatten().flat_map(|(pos, _)| pos) {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let center: [f32; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0);
    let views = [
        raster::View {
            right: [1., 0., 0.],
            up: [0., 1., 0.],
            forward: [0., 0., 1.],
            center,
            span: 2.4,
        },
        raster::View {
            right: [0., 0., -1.],
            up: [0., 1., 0.],
            forward: [1., 0., 0.],
            center,
            span: 2.4,
        },
    ];
    for (vi, view) in views.iter().enumerate() {
        for (p, meshes) in parts.iter().zip(&skinned) {
            for (m, (pos, nrm)) in p.model.meshes.iter().zip(meshes) {
                let surface = raster::Surface {
                    positions: pos,
                    normals: nrm,
                    uvs: &m.uvs,
                    indices: &m.indices,
                    texture: p.diffuse.as_ref(),
                    tint: [1.0; 3],
                    alpha_test: p.alpha_test,
                };
                canvas.draw(&surface, view, vi * w, w);
            }
        }
    }
    write_png(path, &canvas.into_rgba())
}
