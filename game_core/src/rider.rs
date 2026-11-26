//! Poses an imported [`Character`] with the clips Skate 3's motion graph plays (`update_graph`, each clip
//! mirrored for the stance its tree started in), and skins it once per step.

use alloc::vec;
use alloc::vec::Vec;

use crate::character::{Character, Local, Rows, mul, rigid_inverse};
use crate::gfx::SUN;
use crate::math::{Mat3, Vec3, Xform, v3};
use crate::skater::ragdoll::{PARTS, POINTS};

/// The ragdoll takes the bones over this fast at a bail, and gives them back to the clips this fast
/// (s, ours: the drives start from the animated pose, so the take-over only covers the tick the pose
/// lags the body).
const DOLL_IN: f32 = 0.1;
const DOLL_OUT: f32 = 0.25;
/// The bones that place each ragdoll point (`skater::ragdoll`'s order) and that each body part poses
/// (its first bone; the bones under it follow as the clips have them).
const DOLL_POINTS: [&[&str]; POINTS] = [
    &["HIPS"],
    &["SPINE3", "SPINE2"],
    &["HEAD", "NECK1"],
    &["LEFTUPLEG"],
    &["RIGHTUPLEG"],
    &["LEFTARM"],
    &["RIGHTARM"],
    &["LEFTFOREARM"],
    &["LEFTHAND"],
    &["RIGHTFOREARM"],
    &["RIGHTHAND"],
    &["LEFTLEG"],
    &["LEFTFOOT"],
    &["RIGHTLEG"],
    &["RIGHTFOOT"],
];
const DOLL_PARTS: [&str; PARTS] = [
    "HIPS",
    "SPINE",
    "NECK",
    "LEFTARM",
    "LEFTFOREARM",
    "RIGHTARM",
    "RIGHTFOREARM",
    "LEFTUPLEG",
    "LEFTLEG",
    "RIGHTUPLEG",
    "RIGHTLEG",
];

/// A rigid transform part way to another (rows lerped, then squared up).
fn blend_rows(a: &Rows, b: &Rows, k: f32) -> Rows {
    if k >= 1.0 {
        return *b;
    }
    if k <= 0.0 {
        return *a;
    }
    let l = |r: usize| -> Vec3 {
        v3(
            a[r][0] + (b[r][0] - a[r][0]) * k,
            a[r][1] + (b[r][1] - a[r][1]) * k,
            a[r][2] + (b[r][2] - a[r][2]) * k,
        )
    };
    let y = l(1).normalize_or(v3(0.0, 1.0, 0.0));
    let z = l(2);
    let z = (z - y * z.dot(y)).normalize_or(v3(0.0, 0.0, 1.0));
    let x = y.cross(z);
    let t = l(3);
    [
        [x.x, x.y, x.z],
        [y.x, y.y, y.z],
        [z.x, z.y, z.z],
        [t.x, t.y, t.z],
    ]
}

/// Skate 3's animated centre of mass (SK8 skeleton_animation_record.rs `update`, the weights of
/// `SkeletonAnimationMasses::from_bone_data`): each physical part's centre, its bone's pose moved by the
/// part's frame (along the bone), weighted by the part's share of the rider's bone-box volume. The
/// stock rider's 23 body parts (the board, part 0, weighs nothing).
const COM_PARTS: [(&str, f32, f32); 23] = [
    ("NECK1", 0.0642, 0.1137),
    ("NECK", 0.0096, 0.0317),
    ("LEFTHAND", 0.0041, 0.0861),
    ("LEFTFOREARM", 0.0204, 0.1187),
    ("LEFTARM", 0.0383, 0.1446),
    ("LEFTSHOULDER", 0.0181, 0.0852),
    ("RIGHTHAND", 0.0041, 0.0861),
    ("RIGHTFOREARM", 0.0204, 0.1154),
    ("RIGHTARM", 0.0373, 0.1417),
    ("RIGHTSHOULDER", 0.0181, 0.0852),
    ("SPINE3", 0.0948, 0.0727),
    ("SPINE2", 0.0857, 0.0627),
    ("SPINE1", 0.0713, 0.0552),
    ("SPINE", 0.0726, 0.0492),
    ("LEFTTOEBASE", 0.0038, 0.0320),
    ("LEFTFOOT", 0.0124, 0.0817),
    ("LEFTLEG", 0.0625, 0.2040),
    ("LEFTUPLEG", 0.0821, 0.2120),
    ("RIGHTTOEBASE", 0.0038, 0.0320),
    ("RIGHTFOOT", 0.0124, 0.0817),
    ("RIGHTLEG", 0.0625, 0.2040),
    ("RIGHTUPLEG", 0.0821, 0.2120),
    ("HIPS", 0.1193, 0.0),
];

pub struct Rider {
    pub character: Character,
    /// The animated centre of mass's parts found on this skeleton (bone, weight, offset along the
    /// bone), and the board's bone.
    com_parts: Vec<(usize, f32, f32)>,
    board_bone: Option<usize>,
    mirror: bool,
    graph_posed: bool,
    /// Draw the board this high (metres, along the world's up) over the character's origin, whatever
    /// the clips do with it: during a hippie jump it rolls on along the ground under the rider.
    board_floor: Option<f32>,
    /// Draw the board here instead (character space: the board off the feet, `skater::possession`), by
    /// this much over where the clips put it (1: all the way).
    board_place: Option<(crate::character::Rows, f32)>,
    a: Vec<Local>,
    b: Vec<Local>,
    /// The state graphs' pose stack: poses and their bones' channel weights (see
    /// `graph::playback::PoseOp`), and a sampled clip's channel weights.
    stack: Vec<(Vec<Local>, Vec<f32>, bool)>,
    weights: Vec<f32>,
    /// The ragdoll (`skater::ragdoll`) posing the bones in a bail: the bones that seed its points and
    /// that each body part poses, the parts' frames this frame (character space), the frame's time,
    /// whether it holds the bones, how much, each part's bone against the part, the last pose it made,
    /// the pose drawn and the clips' pose under it.
    doll_points: [Option<usize>; POINTS],
    doll_parts: [Option<usize>; PARTS],
    /// The rig's bones for the physical skeleton's bodies (`physics::skeleton::BONE_NAMES`).
    physical: Option<[usize; crate::physics::skeleton::ANIMATED_PARTS]>,
    doll: Option<[Rows; PARTS]>,
    doll_dt: f32,
    doll_on: bool,
    doll_weight: f32,
    doll_offset: [Rows; PARTS],
    doll_pose: Vec<Rows>,
    doll_out: Vec<Rows>,
    anim_world: Vec<Rows>,
    /// The clips' pose was mirrored at skinning (the ragdoll points are read reflected).
    anim_mirrored: bool,
    /// Off the board, the feet on the ground under them (`foot_ik`): the legs' bones, the placement's
    /// state, and this tick's ground under the toes (`None` on the board: the correction fades out).
    legs: Option<crate::foot_ik::Legs>,
    feet_ik: crate::foot_ik::FeetIk,
    feet_frame: Option<crate::foot_ik::Frame>,
}

impl Rider {
    /// How high over the board's contact point the clips' board (the character's origin, the deck's
    /// centre) is drawn so its lowest wheel rests on the ground: Skate 3 rests the physical deck on its
    /// wheels and puts the animation's board on it (SK8 skeleton_root.rs). On level wheels that is the
    /// deck's height (the imported wheels are a little smaller than Skate 3's physical 31 mm ones, so
    /// it is measured on them); a pose that pitches or rolls the board (a manual) lifts the deck about
    /// the wheels left on the ground. Without wheels in the skater pack, the deck's height.
    pub fn board_rest_height(&self) -> f32 {
        let lowest = self
            .character
            .parts
            .iter()
            .filter(|p| p.name == "skatewheel")
            .flat_map(|p| p.out.iter())
            .map(|v| v.pos[1])
            .fold(f32::MAX, f32::min);
        if lowest < 0.0 && lowest > -0.5 {
            -lowest
        } else {
            crate::skater::DECK_HEIGHT
        }
    }

    pub fn new(character: Character) -> Rider {
        let n = character.bones();
        let com_parts: Vec<(usize, f32, f32)> = COM_PARTS
            .iter()
            .filter_map(|&(name, w, along)| Some((character.bone(name)?, w, along)))
            .collect();
        let board_bone = character.bone("SKATEBOARD_ROOT");
        let doll_points = DOLL_POINTS.map(|names| names.iter().find_map(|b| character.bone(b)));
        let doll_parts = DOLL_PARTS.map(|b| character.bone(b));
        let found = crate::physics::skeleton::BONE_NAMES.map(|b| character.bone(b));
        let physical = found
            .iter()
            .all(Option::is_some)
            .then(|| found.map(|b| b.unwrap_or(0)));
        let legs = crate::foot_ik::Legs::find(&character);
        let mut r = Rider {
            legs,
            feet_ik: crate::foot_ik::FeetIk::default(),
            feet_frame: None,
            doll_points,
            doll_parts,
            physical,
            doll: None,
            doll_dt: 0.0,
            doll_on: false,
            doll_weight: 0.0,
            doll_offset: [crate::character::IDENTITY; PARTS],
            doll_pose: vec![crate::character::IDENTITY; n],
            doll_out: vec![crate::character::IDENTITY; n],
            anim_world: vec![crate::character::IDENTITY; n],
            anim_mirrored: false,
            com_parts,
            board_bone,
            character,
            mirror: false,
            graph_posed: false,
            board_floor: None,
            board_place: None,
            a: vec![Local::IDENTITY; n],
            b: vec![Local::IDENTITY; n],
            stack: Vec::new(),
            weights: vec![1.0; n],
        };
        r.pose_and_skin(Mat3::IDENTITY);
        r
    }

    /// One step driven by the state graphs: blends the clips the motion graph plays (`leaves`, weights
    /// summing to 1), each sampled and mirrored for the stance its tree started in before blending, so a
    /// stance change crossfades instead of popping. False (nothing drawn new) when the clips can't be
    /// posed on this skeleton.
    /// The feet against the board from the last pose: along its nose and up (m), left then right (SK8
    /// PushFootFrame `out_distance`; the imported character's space is the board's).
    pub fn feet(&self) -> Option<[[f32; 2]; 2]> {
        let c = &self.character;
        let w = c.world();
        let b = w.get(c.bone("SKATEBOARD_ROOT")?)?[3];
        let foot = |name: &str| -> Option<[f32; 2]> {
            let t = w.get(c.bone(name)?)?[3];
            Some([t[2] - b[2], t[1] - b[1]])
        };
        Some([foot("LEFTFOOT")?, foot("RIGHTFOOT")?])
    }

    /// The posed board's up and nose axes (`SKATEBOARD_ROOT`'s y and z) in the character's space, from the
    /// last pose: up `(0, 1, 0)` and nose along the character's +-z when it rides flat under him.
    pub fn board_axes(&self) -> Option<(Vec3, Vec3)> {
        let m = self.character.world().get(self.board_bone?)?;
        Some((v3(m[1][0], m[1][1], m[1][2]), v3(m[2][0], m[2][1], m[2][2])))
    }

    /// The posed board's centre (`SKATEBOARD_ROOT`) in the character's space, from the last pose.
    pub fn board_in_character(&self) -> Option<Vec3> {
        let b = self.character.world().get(self.board_bone?)?[3];
        Some(v3(b[0], b[1], b[2]))
    }

    /// How high the animated centre of mass is over the posed board's centre (m, along the posed
    /// board's up), from the last pose: SK8's `com_to_deck` (skeleton_animation_record.rs). About 0.98 riding,
    /// 0.6 crouched to pop, 0.5 with the legs tucked in the air. `None` without the stock skeleton.
    pub fn com_above_board(&self) -> Option<f32> {
        if self.com_parts.len() < COM_PARTS.len() {
            return None;
        }
        let w = self.character.world();
        let board = w.get(self.board_bone?)?;
        let mut com = [0.0f32; 3];
        for &(bone, weight, along) in &self.com_parts {
            let m = w.get(bone)?;
            for (i, c) in com.iter_mut().enumerate() {
                *c += (m[3][i] + m[0][i] * along) * weight;
            }
        }
        // Along the posed board's up (SK8's `com_to_deck` is in the board's frame); a flip turns the
        // posed board over under the rider, and then along the character's up (the physical board's).
        let rel = [
            com[0] - board[3][0],
            com[1] - board[3][1],
            com[2] - board[3][2],
        ];
        let up = board[1];
        let along_board: f32 = (0..3).map(|i| rel[i] * up[i]).sum();
        let k = ((up[1] - 0.3) / 0.4).clamp(0.0, 1.0);
        Some(rel[1] + (along_board - rel[1]) * k)
    }

    /// `turned`: the board's frame is turned end for end now (`Skater::frame_turned`); a tree started
    /// the other way is turned half round before blending, so it stays where it was in the world.
    pub fn update_graph(
        &mut self,
        lib: &crate::motion::Library,
        prog: &crate::graph::playback::PoseProgram,
        rot: Mat3,
        turned: bool,
    ) -> bool {
        use crate::graph::playback::PoseOp;
        let n = self.a.len();
        let mut depth = 0usize;
        for op in &prog.ops {
            match *op {
                PoseOp::Group { start, end } => {
                    if self.stack.len() <= depth {
                        self.stack
                            .push((vec![Local::IDENTITY; n], vec![1.0; n], false));
                    }
                    let (pose, chan, valid) = &mut self.stack[depth];
                    let mut total = 0.0;
                    for l in &prog.leaves[start as usize..end as usize] {
                        let Some(clip) = lib.pose(l.clip) else {
                            continue;
                        };
                        if !self
                            .character
                            .sample_clip(&clip, l.time, l.mirror, &mut self.b)
                        {
                            return false;
                        }
                        if l.turned != turned {
                            self.character.turn_half(&mut self.b);
                        }
                        self.character
                            .channel_weights(&clip, l.mirror, &mut self.weights);
                        if total == 0.0 {
                            pose.copy_from_slice(&self.b);
                            chan.copy_from_slice(&self.weights);
                        } else {
                            let k = l.weight / (total + l.weight);
                            for ((a, b), (c, w)) in pose
                                .iter_mut()
                                .zip(&self.b)
                                .zip(chan.iter_mut().zip(&self.weights))
                            {
                                *a = a.blend(b, k);
                                *c += (w - *c) * k;
                            }
                        }
                        total += l.weight;
                    }
                    *valid = total > 0.0;
                    depth += 1;
                }
                PoseOp::Blend(_) | PoseOp::Channel { .. } => {
                    if depth < 2 {
                        return false;
                    }
                    let (low, high) = self.stack.split_at_mut(depth - 1);
                    let (a, ca, va) = &mut low[depth - 2];
                    let (b, cb, vb) = &high[0];
                    match (*va, *vb) {
                        (_, false) => {}
                        (false, true) => {
                            a.copy_from_slice(b);
                            ca.copy_from_slice(cb);
                            *va = true;
                        }
                        (true, true) => {
                            for i in 0..n {
                                let k = match *op {
                                    PoseOp::Channel { weight, first } => {
                                        (weight * if first { ca[i] } else { cb[i] }).clamp(0.0, 1.0)
                                    }
                                    PoseOp::Blend(w) => w,
                                    PoseOp::Group { .. } => 0.0,
                                };
                                a[i] = a[i].blend(&b[i], k);
                                ca[i] += (cb[i] - ca[i]) * k;
                            }
                        }
                    }
                    depth -= 1;
                }
            }
        }
        if depth == 0 || !self.stack[0].2 {
            return false;
        }
        self.a.copy_from_slice(&self.stack[0].0);
        self.graph_posed = true;
        let ch = &mut self.character;
        ch.pose(&self.a);
        let c = &rot.cols;
        if let (Some(b), Some(h)) = (self.board_bone, self.board_floor) {
            // The world's up in the character's space, and the board's height along it now.
            let up = v3(c[0].y, c[1].y, c[2].y);
            let at = ch.world()[b][3];
            let now = v3(at[0], at[1], at[2]).dot(up);
            ch.shift_subtree(b, up * (h - now));
        }
        self.apply_doll();
        self.place_board(false);
        if let Some(legs) = self.legs {
            self.feet_ik
                .update(&mut self.character, &legs, self.feet_frame.as_ref());
        }
        let ch = &mut self.character;
        ch.skin(v3(c[0].dot(SUN), c[1].dot(SUN), c[2].dot(SUN)), false);
        true
    }

    /// Off the board: the ground under the toes this tick for the next graph pose's foot placement
    /// (`foot_ik`), or `None` (on the board, climbing) to fade it out.
    pub fn set_feet(&mut self, frame: Option<crate::foot_ik::Frame>) {
        self.feet_frame = frame;
    }

    /// The toes where the last pose put them (character space), for their ground tests.
    pub fn toes(&self) -> Option<[Vec3; 2]> {
        let legs = self.legs?;
        let w = self.character.world();
        Some(legs.toe.map(|b| v3(w[b][3][0], w[b][3][1], w[b][3][2])))
    }

    /// Which foot holds the ground off the board (0 left, 1 right, 2 neither, 3 in the air).
    pub fn foot_support(&self) -> u8 {
        self.feet_ik.support()
    }

    /// Draws the board at this height over the character's origin (world metres) from the next pose on,
    /// or where the clips put it (`None`).
    pub fn set_board_floor(&mut self, h: Option<f32>) {
        self.board_floor = h;
    }

    /// Draws the board at this frame (character space) from the next pose on, blended over where the
    /// clips put it by `weight` (the board off the feet), or where the clips put it (`None`).
    pub fn set_board_place(&mut self, place: Option<(crate::character::Rows, f32)>) {
        self.board_place = place;
    }

    /// The board's subtree moved to [`Rider::set_board_place`]'s frame. `mirror`: the skin mirrors the
    /// pose after this (the frame is mirrored first, so it ends where it was asked for).
    fn place_board(&mut self, mirror: bool) {
        let (Some(b), Some((mut to, w))) = (self.board_bone, self.board_place) else {
            return;
        };
        if mirror {
            for row in &mut to {
                row[0] = -row[0];
            }
        }
        let ch = &mut self.character;
        let Some(&from) = ch.world().get(b) else {
            return;
        };
        let w = w.clamp(0.0, 1.0);
        let lerp = |a: [f32; 3], b: [f32; 3]| -> Vec3 {
            v3(
                a[0] + (b[0] - a[0]) * w,
                a[1] + (b[1] - a[1]) * w,
                a[2] + (b[2] - a[2]) * w,
            )
        };
        let up = lerp(from[1], to[1]).normalize_or(v3(0.0, 1.0, 0.0));
        let nose = lerp(from[2], to[2]);
        let nose = (nose - up * nose.dot(up)).normalize_or(v3(0.0, 0.0, 1.0));
        let x = v3(from[0][0], from[0][1], from[0][2]);
        let handed = x.dot(
            v3(from[1][0], from[1][1], from[1][2]).cross(v3(from[2][0], from[2][1], from[2][2])),
        );
        let side = if handed < 0.0 {
            nose.cross(up)
        } else {
            up.cross(nose)
        };
        let at = lerp(from[3], to[3]);
        let rows = [
            [side.x, side.y, side.z],
            [up.x, up.y, up.z],
            [nose.x, nose.y, nose.z],
            [at.x, at.y, at.z],
        ];
        ch.place_subtree(b, &rows);
    }

    /// The ragdoll's body parts this frame (character space; `None` outside a bail) and the frame's
    /// time: the next pose puts the bones on them, blending in at a bail and back out after.
    pub fn set_ragdoll(&mut self, parts: Option<[Rows; PARTS]>, dt: f32) {
        self.doll = parts;
        self.doll_dt = dt;
    }

    /// How much of the pose the ragdoll holds (0..1; blending in or out between).
    pub fn ragdoll_weight(&self) -> f32 {
        self.doll_weight
    }

    /// Where the clips' pose (before the ragdoll) puts the ragdoll's points (character space, as
    /// drawn), when this skeleton has every bone for them.
    pub fn ragdoll_points(&self) -> Option<[Vec3; POINTS]> {
        let mut out = [Vec3::ZERO; POINTS];
        for (o, b) in out.iter_mut().zip(self.doll_points) {
            let b = b?;
            let src = if self.anim_mirrored {
                self.character.mirror_partner(b).unwrap_or(b)
            } else {
                b
            };
            let t = self.anim_world.get(src)?[3];
            *o = if self.anim_mirrored {
                v3(-t[0], t[1], t[2])
            } else {
                v3(t[0], t[1], t[2])
            };
        }
        Some(out)
    }

    /// The clips' pose (before the ragdoll) of the physical skeleton's bones (character space, axes
    /// then origin), and whether it is drawn mirrored; none when the rig lacks one of them.
    pub fn physical_bones(
        &self,
    ) -> Option<([Xform; crate::physics::skeleton::ANIMATED_PARTS], bool)> {
        let bones = self.physical?;
        let mut out = [Xform::IDENTITY; crate::physics::skeleton::ANIMATED_PARTS];
        for (o, b) in out.iter_mut().zip(bones) {
            let r = self.anim_world.get(b)?;
            let v = |a: [f32; 3]| v3(a[0], a[1], a[2]);
            *o = Xform::new(Mat3::from_cols(v(r[0]), v(r[1]), v(r[2])), v(r[3]));
        }
        Some((out, self.anim_mirrored))
    }

    /// The ragdoll's pose over the clips' (see [`Rider::set_ragdoll`]).
    fn apply_doll(&mut self) {
        let ch = &mut self.character;
        let n = ch.bones().min(self.doll_pose.len());
        self.anim_world.clear();
        self.anim_world.extend_from_slice(ch.world());
        let doll = self.doll.is_some() || self.doll_weight > 0.0;
        self.anim_mirrored = self.mirror && !doll && !self.graph_posed;
        let dt = self.doll_dt;
        match self.doll {
            Some(parts) => {
                if !self.doll_on {
                    // Each part's bone against the part, where the clips have it now.
                    for (p, b) in self.doll_parts.iter().enumerate() {
                        if let Some(b) = *b {
                            self.doll_offset[p] =
                                mul(&self.anim_world[b], &rigid_inverse(&parts[p]));
                        }
                    }
                    self.doll_on = true;
                }
                self.doll_weight = (self.doll_weight + dt / DOLL_IN).min(1.0);
                for b in 0..n {
                    let part = self.doll_parts.iter().position(|x| *x == Some(b));
                    let p = ch.parent(b);
                    self.doll_pose[b] = match part {
                        Some(k) => mul(&self.doll_offset[k], &parts[k]),
                        None if p >= 0 && (p as usize) < b => {
                            let p = p as usize;
                            let local =
                                mul(&self.anim_world[b], &rigid_inverse(&self.anim_world[p]));
                            mul(&local, &self.doll_pose[p])
                        }
                        None => self.anim_world[b],
                    };
                }
            }
            None => {
                self.doll_on = false;
                if self.doll_weight <= 0.0 {
                    return;
                }
                self.doll_weight = (self.doll_weight - dt / DOLL_OUT).max(0.0);
            }
        }
        let k = self.doll_weight;
        if k <= 0.0 {
            return;
        }
        for b in 0..n {
            self.doll_out[b] = blend_rows(&self.anim_world[b], &self.doll_pose[b], k);
        }
        ch.set_world(&self.doll_out[..n]);
    }

    /// The last pose came from the state graphs' clips (mirrored per tree, not at skin time).
    pub fn graph_posed(&self) -> bool {
        self.graph_posed
    }

    /// The rest pose before the first graph pose: the `idle` clip's first frame where the pack has one.
    fn pose_and_skin(&mut self, rot: Mat3) {
        let ch = &mut self.character;
        if ch.sample("idle", 0.0, &mut self.a) {
            ch.pose(&self.a);
        }
        let c = &rot.cols;
        ch.skin(v3(c[0].dot(SUN), c[1].dot(SUN), c[2].dot(SUN)), false);
    }
}
