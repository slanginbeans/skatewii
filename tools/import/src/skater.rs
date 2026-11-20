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
    // Riding: both feet on the board.
    ("idle", "r_idle_ride_n_0_cyc"),
    ("crouch", "r_antic_crouch_ollie_n_0_into"),
    ("crouch_nollie", "r_antic_crouch_nollie_n_0_into"),
    // Pushing: the usual set and the mongo set (a push starts with `into`, then cycles).
    ("push/into", "r_pushlsp_hstr_n_0_into"),
    ("push/cyc1", "r_pushlsp_hstr_n_0_cyc1"),
    ("push/cyc2", "r_pushlsp_hstr_n_0_cyc2"),
    ("mongo/into", "r_pushlsp_hstr_mongo_0_into"),
    ("mongo/cyc1", "r_pushlsp_hstr_mongo_0_cyc1"),
    ("mongo/cyc2", "r_pushlsp_hstr_mongo_0_cyc2"),
    // Braking: the foot brake, and dragging the tail or nose from a manual.
    ("brake/into", "r_brake_n_n_0_into"),
    ("brake/cyc", "r_brake_n_n_0_cyc"),
    ("brake/out", "r_brake_n_n_0_out"),
    ("tailbrake", "m_brake_n_0_cyc"),
    ("nosebrake", "m_nosebrake_n_0_cyc"),
    // Powerslides and reverts (the 180 slide), kick turns.
    ("slide/fs", "r_turn_180slide_fs_n_0_n"),
    ("slide/bs", "r_turn_180slide_bs_n_0_n"),
    ("kickturn/fs", "r_kickturn_stat_fs"),
    ("kickturn/bs", "r_kickturn_stat_bs"),
    // Riding tail first: a pose whose head and upper back the game takes over the riding clip (the
    // rider looks round to where the board is going).
    ("fakie", "fakie_head_channel_cyc"),
    // Hippie jump: wind-up, jump, in the air.
    ("hippy/antic", "t_hippyjump_low_n_0_antic"),
    ("hippy/into", "t_hippyjump_low_n_0_into"),
    ("hippy/air", "t_hippyjump_low_n_0_cyc"),
    // Flick tricks: pop (leaving the ground) and air; kickflips and heelflips also flip and catch.
    ("ollie/pop", "ollie_low_g"),
    ("ollie/air", "ollie_low_a"),
    ("nollie/pop", "nollie_low_g"),
    ("nollie/air", "nollie_low_a"),
    ("kickflip/pop", "kickflip_in_low_g"),
    ("kickflip/air", "kickflip_in_low_a"),
    ("kickflip/flip", "t_low_kick_cyc1"),
    ("kickflip/out", "t_low_kick_out4"),
    ("heelflip/pop", "heelflip_in_low_g"),
    ("heelflip/air", "heelflip_in_low_a"),
    ("heelflip/flip", "t_low_heel_cyc1"),
    ("heelflip/out", "t_low_heel_out4"),
    ("n_kickflip/pop", "n_kickflip_in_low_g"),
    ("n_kickflip/air", "n_kickflip_in_low_a"),
    ("n_kickflip/flip", "t_low_n_kick_cyc1"),
    ("n_kickflip/out", "t_low_n_kick_out4"),
    ("n_heelflip/pop", "n_heelflip_in_low_g"),
    ("n_heelflip/air", "n_heelflip_in_low_a"),
    ("n_heelflip/flip", "t_low_n_heel_cyc1"),
    ("n_heelflip/out", "t_low_n_heel_out4"),
    ("popshuvit/pop", "popshuvit_low_g"),
    ("popshuvit/air", "popshuvit_low_a"),
    ("fspopshuvit/pop", "fspopshuvit_d_low_g"),
    ("fspopshuvit/air", "fspopshuvit_d_low_a"),
    ("360popshuvit/pop", "360popshuvit_low_g"),
    ("360popshuvit/air", "360popshuvit_low_a"),
    ("fs360popshuvit/pop", "fs360popshuvit_low_g"),
    ("fs360popshuvit/air", "fs360popshuvit_low_a"),
    ("varialkickflip/pop", "varialkickflip_low_g"),
    ("varialkickflip/air", "varialkickflip_low_a"),
    ("varialheelflip/pop", "varialheelflip_d_low_g"),
    ("varialheelflip/air", "varialheelflip_d_low_a"),
    ("hardflip/pop", "hardflip_low_g"),
    ("hardflip/air", "hardflip_low_a"),
    ("inwardheelflip/pop", "inwardheelflip_low_g"),
    ("inwardheelflip/air", "inwardheelflip_low_a"),
    ("360flip/pop", "360flip_d_low_g"),
    ("360flip/air", "360flip_d_low_a"),
    ("laserflip/pop", "laserflip_low_g"),
    ("laserflip/air", "laserflip_low_a"),
    ("360hardflip/pop", "360hardflip_low_g"),
    ("360hardflip/air", "360hardflip_low_a"),
    ("360inwardheelflip/pop", "360inwardheelflip_low_g"),
    ("360inwardheelflip/air", "360inwardheelflip_low_a"),
    ("n_popshuvit/pop", "n_popshuvit_low_g"),
    ("n_popshuvit/air", "n_popshuvit_low_a"),
    ("n_fspopshuvit/pop", "n_fspopshuvit_low_g"),
    ("n_fspopshuvit/air", "n_fspopshuvit_low_a"),
    ("n_360popshuvit/pop", "n_360popshuvit_low_g"),
    ("n_360popshuvit/air", "n_360popshuvit_low_a"),
    ("n_fs360popshuvit/pop", "n_fs360popshuvit_low_g"),
    ("n_fs360popshuvit/air", "n_fs360popshuvit_low_a"),
    ("n_varialkickflip/pop", "n_varialkickflip_low_g"),
    ("n_varialkickflip/air", "n_varialkickflip_low_a"),
    ("n_varialheelflip/pop", "n_varialheelflip_low_g"),
    ("n_varialheelflip/air", "n_varialheelflip_low_a"),
    ("n_hardflip/pop", "n_hardflip_low_g"),
    ("n_hardflip/air", "n_hardflip_low_a"),
    ("n_inwardheelflip/pop", "n_inwardheelflip_low_g"),
    ("n_inwardheelflip/air", "n_inwardheelflip_low_a"),
    ("n_360flip/pop", "n_360flip_low_g"),
    ("n_360flip/air", "n_360flip_low_a"),
    ("n_laserflip/pop", "n_laserflip_low_g"),
    ("n_laserflip/air", "n_laserflip_low_a"),
    ("n_360hardflip/pop", "n_360hardflip_low_g"),
    ("n_360hardflip/air", "n_360hardflip_low_a"),
    ("n_360inwardheelflip/pop", "n_360inwardheelflip_low_g"),
    ("n_360inwardheelflip/air", "n_360inwardheelflip_low_a"),
    // Steering lean: single poses blended over the riding clip by how hard the player steers
    // (hcom: standing tall; lcom: crouched).
    ("lean/left", "r_idle_hcom_n100"),
    ("lean/right", "r_idle_hcom_p100"),
    ("lean/crouch_left", "r_idle_lcom_n100"),
    ("lean/crouch_right", "r_idle_lcom_p100"),
    // Landings: clean, a big drop, and crooked (sketchy) catches turned frontside or backside.
    ("land/clean", "l_hcom_limp"),
    ("land/heavy", "l_hcom_himp"),
    ("land/sketchy_fs", "l_sketch_fs_hcom_limp"),
    ("land/sketchy_bs", "l_sketch_bs_hcom_limp"),
    // Manuals (on the back wheels, on the front wheels).
    ("manual", "m_idle_n_0_cyc"),
    ("nosemanual", "m_noseidle_n_0_cyc"),
    // Grabs, held in the air.
    ("grab/fs", "gr_grab_n_fs_0_cyc"),
    ("grab/bs", "gr_grab_n_bs_0_cyc"),
    ("grab/mute", "gr_mutegrab_n_0_cyc"),
    // Bails: the fall, then getting up.
    ("bail/fall", "w_brace_lean090twist000_n_0_n"),
    ("bail/getup", "w_recovery_onfront_n_0_n"),
    // Grinds, held on a rail: board along it, board across it.
    ("grind/5050", "g_5050_fs_low_0_cyc"),
    ("grind/bslide", "g_bslide_fs_low_0_cyc"),
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
        // Only bones that move vertices matter (the *Hlp helpers carry no weight in the stock parts).
        let weighted = |name: &str| {
            let Some(i) = p.model.bones.iter().position(|b| b.name == name) else {
                return false;
            };
            p.model.meshes.iter().any(|m| {
                m.skin_bones
                    .iter()
                    .zip(&m.skin_weights)
                    .any(|(b, w)| (0..4).any(|k| b[k] as usize == i && w[k] > 0.0))
            })
        };
        for (bone, driver) in skin::coverage(&rig, &p.model) {
            if driver.is_none() && weighted(&bone) {
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
