//! Binds character meshes to the animation skeleton and poses them, for conversion checks and previews.
//!
//! Matrices are row-major with row vectors (`v' = v * M`), as the RW4 files store them. A model bone is
//! matched to the animation skeleton by name. Bones the animation does not drive (finger, twist and face
//! bones) follow their nearest driven ancestor, using that ancestor's bind matrix (SK8 does the same).

use crate::mesh::{Mat4, Model};
use game_core::anim::{Clip, Skeleton};
use std::collections::HashMap;

pub const IDENTITY: Mat4 = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

/// Fallback when there is no rest pose (or it does not line up with the mesh): the animation's body-bone
/// axes as a quarter turn about X from the mesh bind matrices (SK8's `basis_transpose`). Only approximate:
/// the true per-bone roll differs, which twists the skin as soon as a clip plays.
pub const BONE_BASIS: Mat4 = [
    [1., 0., 0., 0.],
    [0., 0., -1., 0.],
    [0., 1., 0., 0.],
    [0., 0., 0., 1.],
];

/// How far (metres) a rest-pose bone may sit from the mesh's bind position and still be used as its bind.
const REST_MATCH: f32 = 0.02;

/// World matrices of a pose given as local samples (scale ignored): `world = local * parent world`.
pub fn pose_world(parents: &[i16], local: &[[f32; 10]]) -> Vec<Mat4> {
    let mut world = vec![IDENTITY; parents.len()];
    for b in 0..parents.len() {
        let s = local
            .get(b)
            .copied()
            .unwrap_or([1., 1., 1., 0., 0., 0., 1., 0., 0., 0.]);
        let l = from_rt([s[3], s[4], s[5], s[6]], [s[7], s[8], s[9]]);
        let p = parents[b];
        world[b] = if p >= 0 && (p as usize) < b {
            mul(&l, &world[p as usize])
        } else {
            l
        };
    }
    world
}

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..4).map(|k| a[r][k] * b[k][c]).sum()))
}

pub fn transform(p: [f32; 3], m: &Mat4) -> [f32; 3] {
    std::array::from_fn(|c| p[0] * m[0][c] + p[1] * m[1][c] + p[2] * m[2][c] + m[3][c])
}

pub fn rotate(v: [f32; 3], m: &Mat4) -> [f32; 3] {
    std::array::from_fn(|c| v[0] * m[0][c] + v[1] * m[1][c] + v[2] * m[2][c])
}

/// Inverse of a rigid (rotation + translation) matrix.
pub fn rigid_inverse(m: &Mat4) -> Mat4 {
    let mut o = IDENTITY;
    for r in 0..3 {
        for c in 0..3 {
            o[r][c] = m[c][r];
        }
    }
    let rot = o;
    for (c, out) in o[3].iter_mut().take(3).enumerate() {
        *out = -(0..3).map(|k| m[3][k] * rot[k][c]).sum::<f32>();
    }
    o
}

/// Skinned positions and normals of one mesh.
pub type Skinned = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// Rotation (unit quaternion xyzw) and translation as a row-vector matrix.
pub fn from_rt(q: [f32; 4], t: [f32; 3]) -> Mat4 {
    let [x, y, z, w] = q;
    [
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y + z * w),
            2. * (x * z - y * w),
            0.,
        ],
        [
            2. * (x * y - z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z + x * w),
            0.,
        ],
        [
            2. * (x * z + y * w),
            2. * (y * z - x * w),
            1. - 2. * (x * x + y * y),
            0.,
        ],
        [t[0], t[1], t[2], 1.],
    ]
}

/// Parent of a model bone name that the animation skeleton may not have (lower-case names).
/// Anatomical rig layout as reconstructed by SK8 (rx2_skeleton.py CANONICAL_PARENT).
fn rig_parent(name: &str) -> Option<String> {
    let n = name.to_ascii_lowercase();
    let side = ["left", "right"].into_iter().find(|s| n.starts_with(s));
    let rest = side.map(|s| &n[s.len()..]);
    let with = |s: &str| format!("{}{s}", side.unwrap_or(""));
    Some(match (side, rest) {
        (Some(_), Some("armtwist" | "forearm")) => with("arm"),
        (Some(_), Some("forearmtwist" | "forearmtwist1" | "hand")) => with("forearm"),
        (Some(_), Some("uplegtwist" | "leg")) => with("upleg"),
        (Some(_), Some("inhandring" | "inhandpinky")) => with("hand"),
        (Some(_), Some(r)) if r.starts_with("hand") => {
            let finger = r
                .trim_start_matches("hand")
                .trim_end_matches(|c: char| c.is_ascii_digit());
            let digit: u32 = r.chars().last().and_then(|c| c.to_digit(10)).unwrap_or(1);
            match (finger, digit) {
                ("ring" | "pinky", 1) => with(&format!("inhand{finger}")),
                (_, 1) => with("hand"),
                (_, d) => with(&format!("hand{finger}{}", d - 1)),
            }
        }
        _ => match n.as_str() {
            "face" => "head".into(),
            "offset_jaw" => "face".into(),
            "jaw" => "offset_jaw".into(),
            "offset_chin" => "jaw".into(),
            "chin" => "offset_chin".into(),
            "tongue" => "jaw".into(),
            "tonguetip" => "tongue".into(),
            // Remaining facial detail bones hang flat off the face.
            _ if n.contains("lip")
                || n.contains("cheek")
                || n.contains("eye")
                || n.contains("nose")
                || n.contains("mouth")
                || n.contains("crease") =>
            {
                "face".into()
            }
            _ => return None,
        },
    })
}

/// A bone's bind matrix: its rest-pose matrix when that sits where the mesh binds the bone, else the mesh
/// bind with the approximate [`BONE_BASIS`] (body bones only).
fn choose_bind(rest: Option<Mat4>, mesh_bind: &Mat4, board: bool) -> Mat4 {
    let near = |r: &Mat4| {
        (0..3)
            .map(|c| (r[3][c] - mesh_bind[3][c]).powi(2))
            .sum::<f32>()
            .sqrt()
            < REST_MATCH
    };
    match rest.filter(near) {
        Some(r) => r,
        None if board => *mesh_bind,
        None => mul(&BONE_BASIS, mesh_bind),
    }
}

pub struct Rig {
    pub parents: Vec<i16>,
    pub names: Vec<String>,
    /// Bind-pose world matrix per animation bone, where some model provides it.
    pub bind: Vec<Option<Mat4>>,
}

impl Rig {
    /// `rest` is the animation bank's bind pose (`RIG_TPOSE`) as local samples. Where a rest bone sits where
    /// the mesh binds it, its rest matrix is the bind: same position, and exactly the bone axes the clips
    /// rotate. Otherwise the mesh bind with [`BONE_BASIS`] is used.
    pub fn new(skeleton: &Skeleton, models: &[&Model], rest: Option<&[[f32; 10]]>) -> Self {
        let names: Vec<String> = (0..skeleton.bones())
            .map(|b| skeleton.name(b).to_string())
            .collect();
        let parents: Vec<i16> = (0..skeleton.bones()).map(|b| skeleton.parent(b)).collect();
        let rest = rest.map(|r| pose_world(&parents, r));
        let mut bind = vec![None; names.len()];
        for m in models {
            for bone in &m.bones {
                if let Some(i) = names
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(&bone.name))
                {
                    bind[i].get_or_insert(choose_bind(
                        rest.as_ref().map(|r| r[i]),
                        &rigid_inverse(&bone.inverse_bind),
                        is_board_bone(&bone.name),
                    ));
                }
            }
        }
        Rig {
            parents,
            names,
            bind,
        }
    }

    /// Animation bone that drives a model bone: the bone itself, or its nearest driven rig ancestor.
    pub fn driver(&self, name: &str) -> Option<usize> {
        let mut cur = name.to_string();
        for _ in 0..16 {
            if let Some(i) = self.names.iter().position(|n| n.eq_ignore_ascii_case(&cur)) {
                return Some(i);
            }
            cur = rig_parent(&cur)?;
        }
        None
    }

    /// Per model bone: (driving animation bone, the matrix taking bind-pose space to that bone's space).
    pub fn bindings(&self, model: &Model) -> Vec<(usize, Mat4)> {
        model
            .bones
            .iter()
            .map(|b| match self.driver(&b.name) {
                // Bind via the driver's own inverse bind so the vertex rides it rigidly.
                Some(d) => (
                    d,
                    self.bind[d]
                        .map(|m| rigid_inverse(&m))
                        .unwrap_or(b.inverse_bind),
                ),
                None => (0, b.inverse_bind),
            })
            .collect()
    }

    /// World matrices for one frame of a clip whose samples are complete local transforms (the bind pose
    /// already composed in, see `anim::add_bind_pose`): `world = local * parent world`.
    pub fn pose(&self, clip: &Clip, frame: usize) -> Vec<Mat4> {
        let mut world = vec![IDENTITY; self.names.len()];
        for b in 0..self.names.len() {
            let s = clip.sample(frame, b);
            let local = from_rt(s.rotation, s.translation);
            let p = self.parents[b];
            world[b] = if p >= 0 {
                mul(&local, &world[p as usize])
            } else {
                local
            };
        }
        world
    }

    /// World matrices for the bind pose (bones without bind data follow their parent).
    pub fn bind_pose(&self) -> Vec<Mat4> {
        let mut world = vec![IDENTITY; self.names.len()];
        for b in 0..self.names.len() {
            world[b] = match self.bind[b] {
                Some(m) => m,
                None if self.parents[b] >= 0 => world[self.parents[b] as usize],
                None => IDENTITY,
            };
        }
        world
    }
}

/// Skins every vertex of `model` with the given animation-bone world matrices.
pub fn skin(model: &Model, bindings: &[(usize, Mat4)], world: &[Mat4]) -> Vec<Skinned> {
    let palette: Vec<Mat4> = bindings
        .iter()
        .map(|(d, inv)| mul(inv, &world[*d]))
        .collect();
    model
        .meshes
        .iter()
        .map(|m| {
            let mut pos = Vec::with_capacity(m.positions.len());
            let mut nrm = Vec::with_capacity(m.positions.len());
            for v in 0..m.positions.len() {
                let (mut p, mut n) = ([0f32; 3], [0f32; 3]);
                for k in 0..4 {
                    let w = m.skin_weights[v][k];
                    if w == 0.0 {
                        continue;
                    }
                    let mat = palette
                        .get(m.skin_bones[v][k] as usize)
                        .unwrap_or(&IDENTITY);
                    let (tp, tn) = (transform(m.positions[v], mat), rotate(m.normals[v], mat));
                    for c in 0..3 {
                        p[c] += w * tp[c];
                        n[c] += w * tn[c];
                    }
                }
                pos.push(p);
                nrm.push(n);
            }
            (pos, nrm)
        })
        .collect()
}

/// Bone-name lookup helper for reports.
pub fn coverage(rig: &Rig, model: &Model) -> HashMap<String, Option<String>> {
    model
        .bones
        .iter()
        .map(|b| {
            (
                b.name.clone(),
                rig.driver(&b.name).map(|d| rig.names[d].clone()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rig_parents_walk_to_driven_bones() {
        assert_eq!(
            rig_parent("LeftHandIndex3").as_deref(),
            Some("lefthandindex2")
        );
        assert_eq!(rig_parent("lefthandindex1").as_deref(), Some("lefthand"));
        assert_eq!(
            rig_parent("RightHandPinky1").as_deref(),
            Some("rightinhandpinky")
        );
        assert_eq!(rig_parent("rightinhandpinky").as_deref(), Some("righthand"));
        assert_eq!(rig_parent("LeftArmTwist").as_deref(), Some("leftarm"));
        assert_eq!(rig_parent("UpperLip").as_deref(), Some("face"));
        assert_eq!(rig_parent("Hips"), None);
    }

    #[test]
    fn binds_to_the_rest_pose_where_it_lines_up() {
        // Rest bone rolled 30 degrees about its X axis relative to the mesh's frame, same position.
        let (s, c) = (15f32.to_radians().sin(), 15f32.to_radians().cos());
        let mut rest = from_rt([s, 0.0, 0.0, c], [0.1, 1.0, 0.0]);
        let mesh_bind = from_rt([0.0, 0.0, 0.0, 1.0], [0.1, 1.0, 0.0]);
        assert_eq!(choose_bind(Some(rest), &mesh_bind, false), rest);
        assert_eq!(choose_bind(Some(rest), &mesh_bind, true), rest);
        // Rest bone elsewhere: fall back to the mesh bind (with the basis for body bones).
        rest[3] = [0.5, 1.0, 0.0, 1.0];
        assert_eq!(choose_bind(Some(rest), &mesh_bind, true), mesh_bind);
        assert_eq!(
            choose_bind(Some(rest), &mesh_bind, false),
            mul(&BONE_BASIS, &mesh_bind)
        );
        assert_eq!(choose_bind(None, &mesh_bind, true), mesh_bind);
    }

    #[test]
    fn pose_world_chains_parents() {
        let id = [1., 1., 1., 0., 0., 0., 1.];
        let up = |y: f32| -> [f32; 10] {
            let mut s = [0.0; 10];
            s[..7].copy_from_slice(&id);
            s[8] = y;
            s
        };
        let w = pose_world(&[-1, 0, 1], &[up(1.0), up(0.5), up(0.25)]);
        assert_eq!(w[2][3], [0.0, 1.75, 0.0, 1.0]);
    }

    #[test]
    fn matrices() {
        let q = [0.0, (0.5f32).sqrt(), 0.0, (0.5f32).sqrt()]; // 90 degrees about Y
        let m = from_rt(q, [1.0, 2.0, 3.0]);
        let p = transform([1.0, 0.0, 0.0], &m);
        assert!(
            (p[0] - 1.0).abs() < 1e-5 && (p[2] - 2.0).abs() < 1e-5,
            "{p:?}"
        );
        let back = transform(p, &rigid_inverse(&m));
        assert!((back[0] - 1.0).abs() < 1e-5 && back[1].abs() < 1e-5 && back[2].abs() < 1e-5);
        assert_eq!(mul(&IDENTITY, &m), m);
    }
}

/// Encodes all meshes of a model as one `game_core::skinned::SkinnedMesh` entry.
pub fn encode_mesh(
    model: &Model,
    bindings: &[(usize, Mat4)],
    alpha_test: bool,
) -> Result<Vec<u8>, String> {
    use game_core::skinned::FLAG_ALPHA_TEST;
    let vertices: usize = model.meshes.iter().map(|m| m.positions.len()).sum();
    let indices: usize = model.meshes.iter().map(|m| m.indices.len()).sum();
    if vertices > u16::MAX as usize || indices > u16::MAX as usize || bindings.len() > 255 {
        return Err(format!(
            "mesh too large for the Wii format ({vertices} vertices, {indices} indices)"
        ));
    }
    let mut out = Vec::new();
    for v in [
        vertices as u16,
        indices as u16,
        bindings.len() as u16,
        if alpha_test { FLAG_ALPHA_TEST } else { 0 },
    ] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for (driver, m) in bindings {
        out.extend_from_slice(&(*driver as u16).to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        for row in m {
            for v in &row[..3] {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
    }
    for m in &model.meshes {
        for v in 0..m.positions.len() {
            for c in m.positions[v] {
                out.extend_from_slice(&c.to_be_bytes());
            }
            for c in m.normals[v] {
                out.push(((c * 127.0).round().clamp(-127.0, 127.0) as i8) as u8);
            }
            out.push(0);
            for c in m.uvs[v] {
                out.extend_from_slice(
                    &((c * 4096.0).round().clamp(-32768.0, 32767.0) as i16).to_be_bytes(),
                );
            }
            out.extend(m.skin_bones[v].map(|b| b.min(255) as u8));
            // Weights to bytes, keeping the sum at exactly 255.
            let mut w = m.skin_weights[v].map(|x| (x * 255.0).round().clamp(0.0, 255.0) as i32);
            let sum: i32 = w.iter().sum();
            if sum > 0 {
                let biggest = (0..4).max_by_key(|&k| w[k]).unwrap();
                w[biggest] += 255 - sum;
            }
            out.extend(w.map(|x| x.clamp(0, 255) as u8));
        }
    }
    let mut base = 0u16;
    for m in &model.meshes {
        for &i in &m.indices {
            out.extend_from_slice(&(i + base).to_be_bytes());
        }
        base += m.positions.len() as u16;
    }
    Ok(out)
}

#[cfg(test)]
mod encode_tests {
    use super::*;
    use crate::mesh::{Bone, Mesh};
    use game_core::skinned::SkinnedMesh;

    #[test]
    fn mesh_round_trips_through_the_game_reader() {
        let mesh = Mesh {
            positions: vec![[0.0, 1.0, 2.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            normals: vec![[0.0, 1.0, 0.0]; 3],
            uvs: vec![[0.5, -1.25], [0.0, 0.0], [1.0, 1.0]],
            skin_bones: vec![[0, 1, 0, 0]; 3],
            skin_weights: vec![[0.333, 0.667, 0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
        };
        let model = Model {
            bones: vec![
                Bone {
                    name: "A".into(),
                    inverse_bind: IDENTITY,
                },
                Bone {
                    name: "B".into(),
                    inverse_bind: IDENTITY,
                },
            ],
            meshes: vec![mesh.clone(), mesh],
        };
        let bindings = vec![
            (3usize, IDENTITY),
            (7usize, from_rt([0., 0., 0., 1.], [1., 2., 3.])),
        ];
        let bytes = encode_mesh(&model, &bindings, true).unwrap();
        let m = SkinnedMesh::parse(&bytes).unwrap();
        assert_eq!(
            (
                m.vertex_count(),
                m.index_count(),
                m.palette_len(),
                m.alpha_test()
            ),
            (6, 6, 2, true)
        );
        assert_eq!(m.palette(1).0, 7);
        assert_eq!(m.palette(1).1[3], [1.0, 2.0, 3.0]);
        let v = m.vertex(3);
        assert_eq!(v.position, [0.0, 1.0, 2.0]);
        assert_eq!(v.uv, [0.5, -1.25]);
        assert_eq!(v.bones, [0, 1, 0, 0]);
        assert!((v.weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert_eq!(
            (m.index(3), m.index(5)),
            (3, 5),
            "second mesh indices are rebased"
        );
    }
}

/// Skateboard bones (deck, trucks, wheels).
pub fn is_board_bone(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("skateboard") || n.starts_with("truck") || n.contains("wheel")
}
