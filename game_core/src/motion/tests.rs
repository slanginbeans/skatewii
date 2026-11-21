//! A small synthetic motion library (made-up clips and trees) and tests of the database reader.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::write::{AttrIn, ClipIn, SimplexIn, TreeIn, write};
use super::*;

/// Names of the test library's trees.
pub struct Lib;
impl Lib {
    pub const PHASE: &'static str = "PB_SPEED";
    pub const SPACE: &'static str = "BS_XY";
    pub const SELECT: &'static str = "SEL_STANCE";
    pub const CHOOSE: &'static str = "SS_ANGLE";
    pub const BROKEN: &'static str = "PB_BROKEN";
    /// A phase blend by LEAN whose authored first child is an extreme (LOOP_B -1, LOOP_A 0, SHORT 1).
    pub const LEAN: &'static str = "PB_LEAN";
}

/// Packs a name the way Skate 3 does (FastString), for bone-reference attributes.
pub fn fast_words(name: &str) -> [u32; 5] {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_";
    let mut out = [0u32; 5];
    for (w, chunk) in out.iter_mut().zip(name.as_bytes().chunks(6)) {
        let mut v = 0u32;
        for i in 0..6 {
            let digit = chunk
                .get(i)
                .and_then(|c| ALPHABET.iter().position(|a| a == c))
                .map_or(0, |p| p as u32 + 1);
            v = v * 38 + digit;
        }
        *w = v;
    }
    out
}

fn scalar(name: &str, v: f32) -> AttrIn {
    AttrIn {
        name: name.into(),
        kind: KIND_SCALAR,
        begin: -1.0,
        end: -1.0,
        words: vec![v.to_bits()],
    }
}

fn clip(name: &str, frames: f32, looping: bool, speed: f32, mut extra: Vec<AttrIn>) -> ClipIn {
    let mut attrs = vec![scalar("SPEED", speed)];
    attrs.append(&mut extra);
    ClipIn {
        name: name.into(),
        fps: 30.0,
        frames,
        base_speed: 1.0,
        andale_flags: if looping { ANDALE_LOOP } else { 0 },
        has_pose: true,
        attrs,
    }
}

pub fn lib() -> Vec<u8> {
    let bone = fast_words("L_FOOT");
    let a = clip(
        "loop_a",
        31.0,
        true,
        2.0,
        vec![
            // An event at the middle and a window over the last quarter.
            AttrIn {
                name: "JUMP".into(),
                kind: KIND_SCALAR,
                begin: 0.5,
                end: 0.5,
                words: vec![1.0f32.to_bits()],
            },
            AttrIn {
                name: "EARLYOUT".into(),
                kind: KIND_SCALAR,
                begin: 0.75,
                end: 1.0,
                words: vec![0],
            },
            AttrIn {
                name: "ANTIC".into(),
                kind: KIND_CURVE,
                begin: -1.0,
                end: -1.0,
                words: [0, 3]
                    .into_iter()
                    .chain([0.0f32, 0.0, 15.0, 1.0, 30.0, 0.5].map(f32::to_bits))
                    .collect(),
            },
            AttrIn {
                name: "VEL".into(),
                kind: KIND_VECTOR,
                begin: -1.0,
                end: -1.0,
                words: [1.0f32, 2.0, 3.0, 0.0].map(f32::to_bits).to_vec(),
            },
            AttrIn {
                name: "CONTACT".into(),
                kind: KIND_BONE,
                begin: 0.0,
                end: 0.25,
                words: bone.into_iter().chain([0.5f32.to_bits()]).collect(),
            },
            scalar("LEAN", 0.0),
        ],
    );
    let b = clip("LOOP_B", 61.0, true, 6.0, vec![scalar("LEAN", -1.0)]);
    let short = clip("SHORT", 16.0, false, 4.0, vec![scalar("LEAN", 1.0)]);
    let s = |x: &str| String::from(x);
    let trees = vec![
        (
            s(Lib::PHASE),
            TreeIn::PhaseBlend {
                parameter: s("SPEED"),
                children: vec![s("LOOP_A"), s("LOOP_B"), s("SHORT")],
            },
        ),
        (
            s(Lib::SPACE),
            TreeIn::BlendSpace {
                parameters: vec![s("X"), s("Y")],
                children: vec![s("LOOP_A"), s("LOOP_B"), s("SHORT")],
                simplexes: vec![SimplexIn {
                    children: vec![0, 1, 2],
                    vertices: vec![vec![0.0, 0.0], vec![1.0, 0.0], vec![0.0, 1.0]],
                    normals: vec![vec![-1.0, -1.0], vec![1.0, 0.0], vec![0.0, 1.0]],
                    scales: vec![1.0; 3],
                }],
            },
        ),
        (
            s(Lib::SELECT),
            TreeIn::Selector {
                parameter: s("STANCE"),
                default: s("LOOP_A"),
                options: vec![(s("REGULAR"), s("LOOP_A")), (s("GOOFY"), s("LOOP_B"))],
            },
        ),
        (
            s(Lib::CHOOSE),
            TreeIn::SelectionSpace {
                parameters: vec![(s("ANGLE"), 0, 1.0, 0.0, 360.0)],
                candidates: vec![(s("LOOP_A"), vec![0.0]), (s("SHORT"), vec![180.0])],
            },
        ),
        (
            // Sorts before the trees it plays: children are resolved against the whole table.
            s("A_NESTED"),
            TreeIn::PhaseBlend {
                parameter: s("SPEED"),
                children: vec![s(Lib::PHASE), s(Lib::SELECT)],
            },
        ),
        (
            s(Lib::LEAN),
            TreeIn::PhaseBlend {
                parameter: s("LEAN"),
                children: vec![s("LOOP_B"), s("LOOP_A"), s("SHORT")],
            },
        ),
        (
            s(Lib::BROKEN),
            TreeIn::PhaseBlend {
                parameter: s("SPEED"),
                children: vec![s("LOOP_A"), s("GONE")],
            },
        ),
    ];
    write(&[a, b, short], &trees)
}

/// A `.fgpak` for tests: header, entries, strings, 32-byte aligned data.
pub fn pack(items: &[(&str, crate::pack::Kind, Vec<u8>)]) -> Vec<u8> {
    let mut strings = Vec::new();
    let mut offs = Vec::new();
    for (n, _, _) in items {
        offs.push(strings.len() as u32);
        strings.extend_from_slice(n.as_bytes());
        strings.push(0);
    }
    let strings_at = 16 + items.len() * 32;
    let mut out = Vec::new();
    out.extend_from_slice(b"FGPA");
    for v in [1u32, items.len() as u32, strings_at as u32] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.resize(strings_at, 0);
    out.extend_from_slice(&strings);
    for (i, (n, k, d)) in items.iter().enumerate() {
        out.resize(out.len().next_multiple_of(32), 0);
        let off = out.len();
        out.extend_from_slice(d);
        let e = 16 + i * 32;
        out[e..e + 4].copy_from_slice(&crate::pack::name_hash(n).to_be_bytes());
        out[e + 4..e + 8].copy_from_slice(&offs[i].to_be_bytes());
        out[e + 8..e + 12].copy_from_slice(&(*k as u32).to_be_bytes());
        out[e + 12..e + 16].copy_from_slice(&(off as u32).to_be_bytes());
        out[e + 16..e + 20].copy_from_slice(&(d.len() as u32).to_be_bytes());
    }
    out
}

#[test]
fn round_trips_names_clips_and_trees() {
    let bytes = lib();
    let db = MotionDb::parse(&bytes).unwrap();
    assert_eq!(db.clips().len(), 3);
    assert_eq!(db.trees().len(), 7);
    let nested = db.tree(db.tree_index("A_NESTED").unwrap());
    let TreeDef::PhaseBlend { children, .. } = &nested.def else {
        panic!()
    };
    assert!(
        children.iter().all(|c| matches!(c, Node::Tree(_))),
        "{children:?}"
    );
    let a = db.clip_index("Loop_A").expect("case-insensitive lookup");
    let c = db.clip(a);
    assert_eq!(db.name(c.name), "LOOP_A");
    assert!(c.looping() && c.has_pose && !c.phase_controlled());
    assert!((c.length() - 1.0).abs() < 1e-6);
    assert_eq!(db.attributes(a).len(), 7);
    assert!(matches!(db.find(Lib::PHASE), Some(Node::Tree(_))));
    let t = db.tree(db.tree_index(Lib::BROKEN).unwrap());
    let TreeDef::PhaseBlend { children, .. } = &t.def else {
        panic!()
    };
    assert!(matches!(children[1], Node::Missing(_)));
    assert_eq!(db.name_id("nothing"), None);
}

#[test]
fn attributes_by_time() {
    let bytes = lib();
    let db = MotionDb::parse(&bytes).unwrap();
    let a = db.clip_index("LOOP_A").unwrap();
    assert_eq!(db.attribute_at(a, "speed", 0.3).unwrap().value, 2.0);
    // The curve is over frames: frame 7.5 is half way to the second point.
    let antic = db.attribute_at(a, "ANTIC", 0.25).unwrap();
    assert!((antic.value - 0.5).abs() < 1e-5, "{}", antic.value);
    assert!((db.attribute_at(a, "ANTIC", 0.75).unwrap().value - 0.75).abs() < 1e-5);
    assert_eq!(db.attribute_at(a, "ANTIC", 5.0).unwrap().value, 0.5, "held");
    assert_eq!(
        db.attribute_at(a, "VEL", 0.0).unwrap().vector,
        [1.0, 2.0, 3.0, 0.0]
    );
    // A window: on inside it, absent outside.
    assert!(db.attribute_at(a, "EARLYOUT", 0.5).is_none());
    let w = db.attribute_at(a, "EARLYOUT", 0.8).unwrap();
    assert_eq!((w.status, w.begin, w.end), (5, 0.75, 1.0));
    let foot = db.attribute_at(a, "CONTACT", 0.1).unwrap();
    assert_eq!(fast_string(&foot.bone.unwrap()), "L_FOOT");
    assert_eq!(foot.value, 0.5);
}

#[test]
fn events_in_a_step_including_wraps() {
    let bytes = lib();
    let db = MotionDb::parse(&bytes).unwrap();
    let a = db.clip_index("LOOP_A").unwrap();
    let jump = db.name_id("JUMP").unwrap();
    let names = |prev: f32, time: f32, loops: u32| -> Vec<NameId> {
        db.events(a, prev, time, loops).map(|v| v.name).collect()
    };
    assert!(names(0.4, 0.6, 0).contains(&jump), "crossed the event");
    assert!(!names(0.1, 0.4, 0).contains(&jump));
    assert!(!names(0.6, 0.7, 0).contains(&jump));
    // Wrapped from 0.9 round to 0.1: the window at the end and the contact at the start.
    let wrapped = names(0.9, 0.1, 1);
    assert!(wrapped.contains(&db.name_id("EARLYOUT").unwrap()));
    assert!(wrapped.contains(&db.name_id("CONTACT").unwrap()));
    assert!(!wrapped.contains(&jump));
    assert_eq!(status(0.5, 0.5, 0.4, 0.6, 0), 9);
    assert_eq!(status(0.2, 0.8, 0.4, 0.6, 0), 5);
    assert_eq!(status(-1.0, -1.0, 0.0, 0.0, 0), 6);
    assert!(passes(15, 9) && passes(15, 5) && passes(15, 6) && !passes(15, 17));
}

#[test]
fn mirroring_flips_vectors_and_swaps_bones() {
    let bytes = lib();
    let db = MotionDb::parse(&bytes).unwrap();
    let a = db.clip_index("LOOP_A").unwrap();
    let mut v = db.attribute_at(a, "VEL", 0.0).unwrap();
    mirror_attribute(&mut v, |_| None);
    assert_eq!(v.vector, [-1.0, 2.0, 3.0, 0.0]);
    let mut b = db.attribute_at(a, "CONTACT", 0.0).unwrap();
    mirror_attribute(&mut b, |n| (n == "L_FOOT").then(|| fast_words("R_FOOT")));
    assert_eq!(fast_string(&b.bone.unwrap()), "R_FOOT");
}

#[test]
fn rejects_garbage() {
    assert_eq!(MotionDb::parse(b"nope").err(), Some(Error::BadMagic));
    let bytes = lib();
    let mut v = bytes.clone();
    v[7] = 9;
    assert_eq!(MotionDb::parse(&v).err(), Some(Error::BadVersion(9)));
    for cut in [10, 40, bytes.len() / 2] {
        assert!(MotionDb::parse(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    assert!(sample_curve(&[0, 5, 0, 0], 1.0).is_none());
}

#[test]
fn library_finds_poses_by_clip() {
    use crate::pack::Kind;
    // One bone, one frame, old (unpacked) layout: identity.
    let mut pose = vec![0u8; crate::anim::ANIM_HEADER + 14];
    pose[1] = 1;
    pose[3] = 1;
    pose[4..8].copy_from_slice(&30.0f32.to_be_bytes());
    pose[crate::anim::ANIM_HEADER + 6..crate::anim::ANIM_HEADER + 8]
        .copy_from_slice(&32767i16.to_be_bytes());
    let bytes = pack(&[
        ("motion/db", Kind::MotionDb, lib()),
        ("motion/clip/SHORT", Kind::Animation, pose),
    ]);
    let l = Library::parse(&bytes).unwrap();
    assert_eq!(l.pose_count(), 1);
    let short = l.db.clip_index("SHORT").unwrap();
    assert_eq!(l.pose(short).unwrap().frames(), 1);
    assert!(l.pose(l.db.clip_index("LOOP_A").unwrap()).is_none());
    assert_eq!(Library::parse(&pack(&[])).err(), Some(LibraryError::NoDb));
}
