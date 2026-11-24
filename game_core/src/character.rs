//! An imported, animated character (`skater.fgpak` from `fg_import skater`): skinned meshes posed by the
//! pack's clips and skinned on the CPU every frame.
//!
//! GX can only transform a vertex by one matrix, so weighted skinning has to happen here. The output is a
//! pre-lit [`TexVertex`] buffer per part, drawn with [`Gfx::draw_indexed`](crate::gfx::Gfx::draw_indexed).
//!
//! Matrices are row-vector 3x4 ([`Rows`]: three rotation rows, then translation), the convention of the
//! pack: `world = local * parent world`, skin matrix = palette bind * bone world.

use alloc::vec::Vec;

use crate::anim::{Clip, Skeleton};
use crate::gfx::{TexVertex, Texture, shade};
use crate::math::{Vec3, sqrtf};
use crate::pack::{Kind, Pack};
use crate::skinned::{SkinnedMesh, Vertex as SkinVertex};

pub type Rows = [[f32; 3]; 4];

pub const IDENTITY: Rows = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]];

/// Pack entry prefixes written by `fg_import skater`.
const PREFIX: &str = "skater/";
const CLIP_PREFIX: &str = "skater/clip/";

/// The inverse of a rigid transform (orthonormal axes).
pub fn rigid_inverse(a: &Rows) -> Rows {
    let mut o = [[0.0; 3]; 4];
    for (r, row) in o.iter_mut().enumerate().take(3) {
        for (c, v) in row.iter_mut().enumerate() {
            *v = a[c][r];
        }
    }
    let t = a[3];
    let rows = [o[0], o[1], o[2]];
    for (c, v) in o[3].iter_mut().enumerate() {
        *v = -(t[0] * rows[0][c] + t[1] * rows[1][c] + t[2] * rows[2][c]);
    }
    o
}

/// `a * b` (apply `a`, then `b`).
pub fn mul(a: &Rows, b: &Rows) -> Rows {
    let mut o = [[0.0; 3]; 4];
    for r in 0..4 {
        for c in 0..3 {
            o[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    for c in 0..3 {
        o[3][c] += b[3][c];
    }
    o
}

/// Rotation (unit quaternion xyzw) and translation as row-vector rows (the importer's `from_rt`).
pub fn from_rt(q: [f32; 4], t: [f32; 3]) -> Rows {
    let [x, y, z, w] = q;
    [
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y + z * w),
            2. * (x * z - y * w),
        ],
        [
            2. * (x * y - z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z + x * w),
        ],
        [
            2. * (x * z + y * w),
            2. * (y * z - x * w),
            1. - 2. * (x * x + y * y),
        ],
        t,
    ]
}

/// One bone's local transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    pub q: [f32; 4],
    pub t: [f32; 3],
}

impl Local {
    pub const IDENTITY: Local = Local {
        q: [0.0, 0.0, 0.0, 1.0],
        t: [0.0; 3],
    };

    /// Blend towards `o` by `k` (normalised lerp, taking the short way round).
    pub fn blend(&self, o: &Local, k: f32) -> Local {
        let d: f32 = (0..4).map(|i| self.q[i] * o.q[i]).sum();
        let s = if d < 0.0 { -k } else { k };
        let mut q: [f32; 4] = core::array::from_fn(|i| self.q[i] * (1.0 - k) + o.q[i] * s);
        let n = sqrtf(q.iter().map(|c| c * c).sum::<f32>()).max(1e-6);
        q.iter_mut().for_each(|c| *c /= n);
        Local {
            q,
            t: core::array::from_fn(|i| self.t[i] + (o.t[i] - self.t[i]) * k),
        }
    }
}

pub struct Part {
    pub name: &'static str,
    palette: Vec<(u16, Rows)>,
    verts: Vec<SkinVertex>,
    pub indices: Vec<u16>,
    pub texture: Option<Texture<'static>>,
    /// Skinned, lit vertices from the last [`Character::skin`].
    pub out: Vec<TexVertex>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Pack,
    NoSkeleton,
    NoParts,
    /// A mesh, clip or skeleton entry did not parse or does not match the skeleton.
    Bad(&'static str),
}

pub struct Character {
    skeleton: Skeleton<'static>,
    /// The skeleton's parent and mirror-partner tables (for mirroring poses).
    parents: Vec<i16>,
    partners: Vec<i16>,
    clips: Vec<(&'static str, Clip<'static>)>,
    pub parts: Vec<Part>,
    world: Vec<Rows>,
    /// Each bone's bind matrix (from whichever part binds it), for mirrored skinning.
    binds: Vec<Option<Rows>>,
    /// The axis (0 = x, 2 = z) the bind pose is symmetric across: left and right bones differ along it.
    rest_axis: usize,
    /// The light and mirroring of the last [`Character::skin`].
    skinned: (Vec3, bool),
}

impl Character {
    pub fn parse(bytes: &'static [u8]) -> Result<Character, Error> {
        let pack = Pack::parse(bytes).map_err(|_| Error::Pack)?;
        let skeleton = pack
            .find("skater/skeleton")
            .filter(|e| e.kind == Kind::Skeleton as u32)
            .ok_or(Error::NoSkeleton)?;
        let skeleton = Skeleton::parse(skeleton.data).map_err(|_| Error::NoSkeleton)?;
        let bones = skeleton.bones();
        let mut parts = Vec::new();
        let mut clips = Vec::new();
        for e in pack.iter() {
            if e.kind == Kind::Animation as u32
                && let Some(role) = e.name.strip_prefix(CLIP_PREFIX)
            {
                let clip = Clip::parse(e.data).map_err(|_| Error::Bad(e.name))?;
                if clip.bones() != bones || clip.frames() == 0 {
                    return Err(Error::Bad(e.name));
                }
                clips.push((role, clip));
            } else if e.kind == Kind::SkinnedMesh as u32
                && let Some(name) = e.name.strip_prefix(PREFIX)
            {
                let mesh = SkinnedMesh::parse(e.data).map_err(|_| Error::Bad(e.name))?;
                let palette: Vec<(u16, Rows)> = (0..mesh.palette_len())
                    .map(|i| {
                        let (bone, m) = mesh.palette(i);
                        (bone, m)
                    })
                    .collect();
                if palette.iter().any(|(b, _)| *b as usize >= bones) {
                    return Err(Error::Bad(e.name));
                }
                let verts: Vec<SkinVertex> =
                    (0..mesh.vertex_count()).map(|i| mesh.vertex(i)).collect();
                if verts
                    .iter()
                    .any(|v| v.bones.iter().any(|&b| b as usize >= palette.len().max(1)))
                {
                    return Err(Error::Bad(e.name));
                }
                let indices: Vec<u16> = (0..mesh.index_count()).map(|i| mesh.index(i)).collect();
                if indices.iter().any(|&i| i as usize >= verts.len()) {
                    return Err(Error::Bad(e.name));
                }
                let texture = pack
                    .iter()
                    .find(|t| {
                        t.kind == Kind::Texture as u32
                            && t.name.len() == e.name.len() + "/diffuse".len()
                            && t.name.starts_with(e.name)
                            && t.name.ends_with("/diffuse")
                    })
                    .map(|t| Texture {
                        data: t.data,
                        width: t.width,
                        height: t.height,
                        format: t.gx_format,
                        alpha_test: mesh.alpha_test(),
                    });
                parts.push(Part {
                    name,
                    palette,
                    out: Vec::with_capacity(verts.len()),
                    verts,
                    indices,
                    texture,
                });
            }
        }
        if parts.is_empty() {
            return Err(Error::NoParts);
        }
        let mut binds: Vec<Option<Rows>> = alloc::vec![None; bones];
        for p in &parts {
            for (b, m) in &p.palette {
                binds[*b as usize].get_or_insert(*m);
            }
        }
        // The pair of mirrored bones furthest apart in the bind pose shows the symmetry axis.
        let mut rest_axis = 0;
        let mut widest = 0.0;
        for b in 0..bones {
            let m = skeleton.mirror(b);
            if m < 0 || m as usize == b || m as usize >= bones {
                continue;
            }
            if let (Some(a), Some(c)) = (binds[b], binds[m as usize]) {
                let (pa, pc) = (rest_position(&a), rest_position(&c));
                for axis in [0, 2] {
                    let d = (pa[axis] - pc[axis]).abs();
                    if d > widest {
                        widest = d;
                        rest_axis = axis;
                    }
                }
            }
        }
        let parents = (0..bones).map(|b| skeleton.parent(b)).collect();
        let partners = (0..bones).map(|b| skeleton.mirror(b)).collect();
        Ok(Character {
            skeleton,
            parents,
            partners,
            clips,
            parts,
            world: alloc::vec![IDENTITY; bones],
            binds,
            rest_axis,
            skinned: (Vec3::UP, false),
        })
    }

    pub fn bones(&self) -> usize {
        self.skeleton.bones()
    }

    /// The index of the bone called `name` (case-insensitive).
    pub fn bone(&self, name: &str) -> Option<usize> {
        (0..self.bones()).find(|&b| self.skeleton.name(b).eq_ignore_ascii_case(name))
    }

    pub fn has_clip(&self, role: &str) -> bool {
        self.clip(role).is_some()
    }

    fn clip(&self, role: &str) -> Option<&Clip<'static>> {
        self.clips.iter().find(|(r, _)| *r == role).map(|(_, c)| c)
    }

    /// Length of a clip in seconds (0 when missing).
    pub fn duration(&self, role: &str) -> f32 {
        self.clip(role).map_or(0.0, |c| c.frames() as f32 / c.fps())
    }

    /// Samples `role` at `time` seconds into `out` (one entry per bone), interpolating between frames.
    /// Looping clips wrap, others hold their last frame. False (and `out` untouched) if there is no such
    /// clip. The trajectory root is pinned to the origin: the game moves the character itself.
    pub fn sample(&self, role: &str, time: f32, out: &mut [Local]) -> bool {
        let Some(clip) = self.clip(role) else {
            return false;
        };
        let n = clip.frames();
        let f = (time.max(0.0) * clip.fps()).max(0.0);
        let (a, b, k) = if clip.looping() {
            let f = f % n as f32;
            let a = f as usize % n;
            (a, (a + 1) % n, f - f as usize as f32)
        } else if f >= (n - 1) as f32 {
            (n - 1, n - 1, 0.0)
        } else {
            (f as usize, f as usize + 1, f - f as usize as f32)
        };
        for (bone, o) in out.iter_mut().enumerate().take(clip.bones()) {
            let (sa, sb) = (clip.sample(a, bone), clip.sample(b, bone));
            let la = Local {
                q: sa.rotation,
                t: sa.translation,
            };
            *o = la.blend(
                &Local {
                    q: sb.rotation,
                    t: sb.translation,
                },
                k,
            );
        }
        if self.skeleton.has_trajectory()
            && let Some(root) = out.first_mut()
        {
            *root = Local::IDENTITY;
        }
        true
    }

    /// Samples any clip with this skeleton's bones (a motion-library clip) at `time` seconds into `out`,
    /// mirrored left for right first when `mirror` (a pose of the other stance: Skate 3 mirrors each
    /// tree's pose before blending). False when the clip has a different bone count.
    /// The clip's channel weight for each bone as posed (mirrored, left and right swap): 1 everywhere
    /// unless it is a channel animation.
    pub fn channel_weights(&self, clip: &Clip, mirror: bool, out: &mut [f32]) {
        for (b, o) in out.iter_mut().enumerate() {
            let src = match self.partners.get(b) {
                Some(&p) if mirror && p >= 0 => p as usize,
                _ => b,
            };
            *o = clip.channel_weight(src);
        }
    }

    pub fn sample_clip(&self, clip: &Clip, time: f32, mirror: bool, out: &mut [Local]) -> bool {
        let n = clip.frames();
        if clip.bones() != self.bones() || n == 0 {
            return false;
        }
        let f = (time.max(0.0) * clip.fps()).max(0.0);
        let (a, b, k) = if clip.looping() {
            let f = f % n as f32;
            let a = f as usize % n;
            (a, (a + 1) % n, f - f as usize as f32)
        } else if f >= (n - 1) as f32 {
            (n - 1, n - 1, 0.0)
        } else {
            (f as usize, f as usize + 1, f - f as usize as f32)
        };
        let mut pose: Vec<crate::anim::Sample> = Vec::with_capacity(clip.bones());
        for bone in 0..clip.bones() {
            let (sa, sb) = (clip.sample(a, bone), clip.sample(b, bone));
            let l = Local {
                q: sa.rotation,
                t: sa.translation,
            }
            .blend(
                &Local {
                    q: sb.rotation,
                    t: sb.translation,
                },
                k,
            );
            pose.push(crate::anim::Sample {
                rotation: l.q,
                translation: l.t,
                scale: [1.0; 3],
            });
        }
        if mirror {
            crate::anim::mirror_pose(&mut pose, &self.parents, &self.partners, 1);
        }
        for (o, p) in out.iter_mut().zip(&pose) {
            *o = Local {
                q: p.rotation,
                t: p.translation,
            };
        }
        if self.skeleton.has_trajectory()
            && let Some(root) = out.first_mut()
        {
            *root = Local::IDENTITY;
        }
        true
    }

    /// Turns a local pose half round about the character's up (y) through its origin: what a tree
    /// started before the board's frame was turned end for end looks like in the turned frame. The
    /// trajectory root stays pinned; its children (the hips, the board) turn, so a blend with a pose
    /// started in the new frame turns only by what the clips differ (no half-turn slerp of the root).
    pub fn turn_half(&self, local: &mut [Local]) {
        let pinned = self.skeleton.has_trajectory();
        for (b, l) in local.iter_mut().enumerate().take(self.bones()) {
            let p = self.skeleton.parent(b);
            let top = if pinned { b != 0 && p == 0 } else { p < 0 };
            if !top {
                continue;
            }
            // q then half a turn about y, as `mul(from_rt(q), from_rt([0, 1, 0, 0]))`.
            let [x, y, z, w] = l.q;
            l.q = [z, w, -x, -y];
            l.t = [-l.t[0], l.t[1], -l.t[2]];
        }
    }

    /// Bone world matrices from local transforms.
    pub fn pose(&mut self, local: &[Local]) {
        for b in 0..self.world.len() {
            let l = local.get(b).copied().unwrap_or(Local::IDENTITY);
            let m = from_rt(l.q, l.t);
            let p = self.skeleton.parent(b);
            self.world[b] = if p >= 0 && (p as usize) < b {
                mul(&m, &self.world[p as usize])
            } else {
                m
            };
        }
    }

    /// Moves a posed bone and everything under it by `by` (the character's space), before skinning.
    pub fn shift_subtree(&mut self, bone: usize, by: Vec3) {
        let mut moved = alloc::vec![false; self.world.len()];
        for b in bone..self.world.len() {
            let p = self.skeleton.parent(b);
            moved[b] = b == bone || (p >= 0 && moved.get(p as usize).copied().unwrap_or(false));
            if moved[b] {
                let t = &mut self.world[b][3];
                t[0] += by.x;
                t[1] += by.y;
                t[2] += by.z;
            }
        }
    }

    /// Moves `bone`'s subtree rigidly so the bone's transform (character space) becomes `to`.
    pub fn place_subtree(&mut self, bone: usize, to: &Rows) {
        let Some(from) = self.world.get(bone) else {
            return;
        };
        let d = mul(&rigid_inverse(from), to);
        let mut moved = alloc::vec![false; self.world.len()];
        for b in bone..self.world.len() {
            let p = self.skeleton.parent(b);
            moved[b] = b == bone || (p >= 0 && moved.get(p as usize).copied().unwrap_or(false));
            if moved[b] {
                self.world[b] = mul(&self.world[b], &d);
            }
        }
    }

    pub fn world(&self) -> &[Rows] {
        &self.world
    }

    /// Sets the posed bones' transforms (character space) directly, as [`Character::pose`] leaves them
    /// (the replay draws a recorded skeleton). Extra bones are ignored, missing ones kept.
    pub fn set_world(&mut self, world: &[Rows]) {
        for (d, s) in self.world.iter_mut().zip(world) {
            *d = *s;
        }
    }

    /// The light and mirroring the parts were last skinned with.
    pub fn last_skin(&self) -> (Vec3, bool) {
        self.skinned
    }

    /// Skins every part with the current pose and lights it from `light` (unit vector towards the light,
    /// in the character's own space). `mirror` draws the mirror image across the board's long axis
    /// (x = 0, swapping the toe and heel sides): each bone drives its left/right partner's vertices,
    /// reflected, which turns the clips' goofy rider into a regular one still facing down the board.
    pub fn skin(&mut self, light: Vec3, mirror: bool) {
        self.skinned = (light, mirror);
        let world = &self.world;
        let skeleton = &self.skeleton;
        let binds = &self.binds;
        let rest_axis = self.rest_axis;
        let mut palette: Vec<Rows> = Vec::new();
        for part in &mut self.parts {
            palette.clear();
            palette.extend(part.palette.iter().map(|(bone, bind)| {
                if !mirror {
                    return mul(bind, &world[*bone as usize]);
                }
                let b = *bone as usize;
                let m = skeleton.mirror(b);
                let m = if m >= 0 && (m as usize) < world.len() {
                    m as usize
                } else {
                    b
                };
                let bind_m = binds[m].unwrap_or(*bind);
                mirrored(&mul(&bind_m, &world[m]), rest_axis, 0)
            }));
            part.out.clear();
            for v in &part.verts {
                let (mut p, mut n) = ([0.0f32; 3], [0.0f32; 3]);
                for k in 0..4 {
                    let w = v.weights[k];
                    if w == 0.0 {
                        continue;
                    }
                    let m = palette.get(v.bones[k] as usize).unwrap_or(&IDENTITY);
                    for c in 0..3 {
                        let r = v.position[0] * m[0][c] + v.position[1] * m[1][c];
                        p[c] += w * (r + v.position[2] * m[2][c] + m[3][c]);
                        n[c] += w
                            * (v.normal[0] * m[0][c]
                                + v.normal[1] * m[1][c]
                                + v.normal[2] * m[2][c]);
                    }
                }
                let len = sqrtf(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).max(1e-6);
                let d = (n[0] * light.x + n[1] * light.y + n[2] * light.z) / len;
                part.out.push(TexVertex {
                    pos: p,
                    rgba: shade(0xffff_ffff, 0.55 + 0.45 * d.max(0.0)),
                    uv: v.uv,
                });
            }
        }
    }
}

/// A bone's position in the bind pose, from its bind (inverse rest) matrix.
fn rest_position(bind: &Rows) -> [f32; 3] {
    // bind = [R; t] maps rest space to bone space, so the bone sits at -t R^T.
    let t = bind[3];
    core::array::from_fn(|c| -(t[0] * bind[c][0] + t[1] * bind[c][1] + t[2] * bind[c][2]))
}

/// `S_in * m * S_out` for row vectors: reflect the input across `axis_in`, apply `m`, reflect the
/// output across `axis_out`.
fn mirrored(m: &Rows, axis_in: usize, axis_out: usize) -> Rows {
    let mut o = *m;
    for (r, row) in o.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            if r == axis_in {
                *v = -*v;
            }
            if c == axis_out {
                *v = -*v;
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
    }

    #[test]
    fn rows_compose_like_the_importer() {
        // 90 degrees about Y takes +X to -Z (row vectors).
        let s = core::f32::consts::FRAC_1_SQRT_2;
        let m = from_rt([0.0, s, 0.0, s], [1.0, 2.0, 3.0]);
        let p = [1.0, 0.0, 0.0];
        let t: [f32; 3] =
            core::array::from_fn(|c| p[0] * m[0][c] + p[1] * m[1][c] + p[2] * m[2][c] + m[3][c]);
        assert!(close(t, [1.0, 2.0, 2.0]), "{t:?}");
        let child = from_rt([0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        let w = mul(&child, &m);
        assert!(close(w[3], [1.0, 3.0, 3.0]), "{:?}", w[3]);
    }

    #[test]
    fn blend_takes_the_short_way() {
        let a = Local::IDENTITY;
        let b = Local {
            q: [0.0, 0.0, 0.0, -1.0], // same rotation, other sign
            t: [2.0, 0.0, 0.0],
        };
        let m = a.blend(&b, 0.5);
        assert!((m.q[3].abs() - 1.0).abs() < 1e-5);
        assert_eq!(m.t, [1.0, 0.0, 0.0]);
    }

    /// Builds a one-part, two-bone character pack in memory (synthetic, like the importer tests).
    fn synthetic_pack() -> &'static [u8] {
        use crate::anim::{ANIM_HEADER, FLAG_LOOP, NAME_LEN};
        let be16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
        let bef = |v: &mut Vec<u8>, x: f32| v.extend_from_slice(&x.to_be_bytes());
        // Skeleton: root (trajectory) and one child.
        let mut skel = vec![];
        be16(&mut skel, 2);
        be16(&mut skel, 1);
        for (p, m) in [(-1i16, -1i16), (0, -1)] {
            be16(&mut skel, p as u16);
            be16(&mut skel, m as u16);
        }
        for name in ["traj", "hips"] {
            let mut f = [0u8; NAME_LEN];
            f[..name.len()].copy_from_slice(name.as_bytes());
            skel.extend_from_slice(&f);
        }
        // Clip: 2 frames; hips translate up by 1 (q = 1.0 * step), root translation is ignored.
        let mut clip = vec![0u8; ANIM_HEADER];
        clip[0..2].copy_from_slice(&2u16.to_be_bytes());
        clip[2..4].copy_from_slice(&2u16.to_be_bytes());
        clip[4..8].copy_from_slice(&30f32.to_be_bytes());
        clip[8..12].copy_from_slice(&FLAG_LOOP.to_be_bytes());
        for i in 0..3 {
            clip[36 + 4 * i..40 + 4 * i].copy_from_slice(&1f32.to_be_bytes()); // step 1
        }
        for frame in 0..2u16 {
            for bone in 0..2u16 {
                for q in [0i16, 0, 0, 32767] {
                    be16(&mut clip, q as u16);
                }
                let up = if bone == 1 { frame } else { 5 };
                for t in [0, up, 0] {
                    be16(&mut clip, t);
                }
            }
        }
        // Mesh: one triangle on bone 1 (hips), identity bind.
        let mut mesh = vec![];
        for x in [3u16, 3, 1, 0] {
            be16(&mut mesh, x);
        }
        be16(&mut mesh, 1);
        be16(&mut mesh, 0);
        for r in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]] {
            for c in r {
                bef(&mut mesh, c);
            }
        }
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in p {
                bef(&mut mesh, c);
            }
            mesh.extend_from_slice(&[0, 127, 0, 0]);
            be16(&mut mesh, 0);
            be16(&mut mesh, 4096);
            mesh.extend_from_slice(&[0, 0, 0, 0, 255, 0, 0, 0]);
        }
        for i in [0u16, 1, 2] {
            be16(&mut mesh, i);
        }
        // Pack.
        let items: [(&str, u32, &[u8]); 3] = [
            ("skater/skeleton", Kind::Skeleton as u32, &skel),
            ("skater/clip/idle", Kind::Animation as u32, &clip),
            ("skater/body", Kind::SkinnedMesh as u32, &mesh),
        ];
        let mut strings = vec![];
        let mut name_at = vec![];
        for (n, _, _) in &items {
            name_at.push(strings.len() as u32);
            strings.extend_from_slice(n.as_bytes());
            strings.push(0);
        }
        let strings_at = 16 + 32 * items.len();
        let mut data_at = (strings_at + strings.len()).next_multiple_of(32);
        let mut out = vec![];
        out.extend_from_slice(b"FGPA");
        for x in [1u32, items.len() as u32, strings_at as u32] {
            out.extend_from_slice(&x.to_be_bytes());
        }
        let mut offsets = vec![];
        for (i, (n, kind, d)) in items.iter().enumerate() {
            for x in [
                crate::pack::name_hash(n),
                name_at[i],
                *kind,
                data_at as u32,
                d.len() as u32,
            ] {
                out.extend_from_slice(&x.to_be_bytes());
            }
            out.extend_from_slice(&[0; 12]);
            offsets.push(data_at);
            data_at = (data_at + d.len()).next_multiple_of(32);
        }
        out.extend_from_slice(&strings);
        for (i, (_, _, d)) in items.iter().enumerate() {
            out.resize(offsets[i], 0);
            out.extend_from_slice(d);
        }
        alloc::boxed::Box::leak(out.into_boxed_slice())
    }

    #[test]
    fn poses_and_skins_a_synthetic_character() {
        let mut c = Character::parse(synthetic_pack()).unwrap();
        assert_eq!(c.parts.len(), 1);
        assert!(c.has_clip("idle") && !c.has_clip("push"));
        assert!((c.duration("idle") - 2.0 / 30.0).abs() < 1e-6);
        let mut local = vec![Local::IDENTITY; c.bones()];
        // Half way between frame 0 (hips at 0) and frame 1 (hips at 1).
        assert!(c.sample("idle", 0.5 / 30.0, &mut local));
        assert_eq!(local[0], Local::IDENTITY, "trajectory root is pinned");
        assert!((local[1].t[1] - 0.5).abs() < 1e-4);
        c.pose(&local);
        c.skin(Vec3::UP, false);
        let out = &c.parts[0].out;
        assert!(close(out[1].pos, [1.0, 0.5, 0.0]), "{:?}", out[1].pos);
        assert_eq!(out[0].uv, [0.0, 1.0]);
        // Normal +Y faces the light: full brightness.
        assert_eq!(out[0].rgba, 0xffff_ffff);
        assert_eq!(c.parts[0].indices, [0, 1, 2]);
        // Mirrored: the vertex comes from across the rest pose's symmetry plane (x here: the skeleton
        // has no left/right pairs) and is reflected across the board's long axis (x): a bone that only
        // moves up lands where it was.
        c.skin(Vec3::UP, true);
        let p = c.parts[0].out[1].pos;
        assert!(close(p, [1.0, 0.5, 0.0]), "{p:?}");
        assert_eq!(c.parts[0].out[0].rgba, 0xffff_ffff);
        // Turned about y, the mirror image turns the other way.
        let s = core::f32::consts::FRAC_1_SQRT_2;
        local[1].q = [0.0, s, 0.0, s];
        c.pose(&local);
        c.skin(Vec3::UP, false);
        let plain = c.parts[0].out[1].pos;
        c.skin(Vec3::UP, true);
        let mirror = c.parts[0].out[1].pos;
        assert!(
            close(plain, [mirror[0], mirror[1], -mirror[2]]),
            "{plain:?} {mirror:?}"
        );
    }

    #[test]
    fn turning_a_pose_half_round_turns_every_bone_about_the_origin() {
        let mut c = Character::parse(synthetic_pack()).unwrap();
        let mut local = vec![Local::IDENTITY; c.bones()];
        let (s, k) = (0.6f32, 0.8f32);
        local[1] = Local {
            q: [0.1 * s, s, 0.0, k],
            t: [0.3, 0.9, -0.2],
        };
        let n = libm::sqrtf(local[1].q.iter().map(|v| v * v).sum());
        local[1].q = local[1].q.map(|v| v / n);
        c.pose(&local);
        let before = c.world()[1];
        c.turn_half(&mut local);
        assert_eq!(
            local[0],
            Local::IDENTITY,
            "the trajectory root stays pinned"
        );
        c.pose(&local);
        let half = from_rt([0.0, 1.0, 0.0, 0.0], [0.0; 3]);
        let want = mul(&before, &half);
        let got = c.world()[1];
        for r in 0..4 {
            assert!(close(got[r], want[r]), "{got:?} {want:?}");
        }
    }
}
