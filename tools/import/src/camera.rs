//! Skate 3's gameplay camera for the rules pack: its camera graph (`Default_cameragraph.stategraph`,
//! compiled for the game's graph runner as `rules/camera/graph`), the shots that graph chooses, with
//! every blend shot's children (`camera_shots` collections of the tuning database, as
//! `rules/camera/shots`; layout in `game_core::camera::shot`), and the camera shake's sample loop
//! (`data/camera/1.shk`, as `rules/camera/shake`; layout in `game_core::camera::shake`).
//!
//! How shots are stored follows SK8-ENGINE/skate-3-rust-engine (GPL-3.0)
//! crates/skate-game/src/camera/shot_data.rs: a blend's children are `RefSpec`s (the class hash, then
//! the child collection's name hash). The `.shk` text follows its crates/skate-game/src/camera/
//! shake_data.rs: a sample count, then eight numbers a sample (a rotation's x y z and a fourth, a
//! translation's x y z and a fourth; the fourths unused).

use std::collections::HashMap;

use game_core::camera::shake::Samples;
use game_core::camera::shot::{
    NONE, OPT_AVOID, OPT_DROP, OPT_FOLLOW, OPT_LOOK, OPT_MIRROR, OPT_PREVIOUS, OPT_SNAP, REFS,
    Shot, ShotDef, ShotSet, name_hash,
};
use game_core::camera::tracker::RAD;

use crate::Res;

pub const GRAPH: &str = "data/big/miscload.big::data/script/camera/Default_cameragraph.stategraph";
pub const CLASS: &str = "camera_shots";
/// The gameplay camera's shake samples (Skate 3 loads `1.shk` for the shake it shows, `2.shk` for a
/// variant its preferences never pick).
pub const SHAKE: &str = "data/big/miscload.big::data/camera/1.shk";

/// Reads a `.shk` sample loop.
pub fn shake(text: &str) -> Res<Samples> {
    let mut words = text.split_whitespace();
    let n: usize = words
        .next()
        .ok_or("camera shake: empty")?
        .parse()
        .map_err(|e| format!("camera shake: sample count: {e}"))?;
    if n == 0 || n > 4096 {
        return Err(format!("camera shake: {n} samples").into());
    }
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let mut v = [0.0f32; 8];
        for x in &mut v {
            *x = words
                .next()
                .ok_or_else(|| format!("camera shake: sample {i} cut short"))?
                .parse()
                .map_err(|e| format!("camera shake: sample {i}: {e}"))?;
        }
        rows.push([v[0], v[1], v[2], v[4], v[5], v[6]]);
    }
    if words.next().is_some() {
        return Err("camera shake: more numbers than samples".into());
    }
    Samples::from_rows(&rows).ok_or_else(|| "camera shake: a sample isn't a number".into())
}

/// The reference weights' fields, in the order the shots store them.
const WEIGHTS: [&str; REFS] = [
    "ReferenceWeightHead",
    "ReferenceWeightHips",
    "ReferenceWeightGrind",
    "ReferenceWeightCOM",
    "ReferenceWeightBoard",
    "ReferenceWeightTrajectory",
    "ReferenceWeightAnchor",
    "ReferenceWeightFeet",
    "ReferenceWeightDampedCOM",
    "Hash_D70C95EDC16A7DFC",
];

/// Compiles the camera graph for `game_core::graph` (one graph, named `Camera`).
pub fn graph(bytes: &[u8]) -> Res<Vec<u8>> {
    let root = crate::stategraph::elements(bytes)?;
    Ok(game_core::graph::write::compile(&[(
        game_core::camera::select::GRAPH,
        &root,
    )])?)
}

/// The shot names a compiled camera graph chooses (and compares with `IsPreviousShot`).
pub fn shots_named(compiled: &[u8]) -> Res<Vec<String>> {
    let p = game_core::graph::Pack::parse(compiled).map_err(|e| format!("camera graph: {e:?}"))?;
    let mut names = Vec::new();
    for g in &p.graphs {
        for i in 0..g.ops.len() {
            let a = g.op_attrs(i as u16);
            for k in ["shot1", "shot2", "shot3", "shot4"] {
                if let Some(n) = p.text(a, k) {
                    let n = n.to_ascii_lowercase();
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
            }
        }
    }
    Ok(names)
}

/// Reads one field of collection `key`: its raw bytes (through the collection's parents).
pub type Field<'a> = dyn Fn(u64, &str) -> Option<Vec<u8>> + 'a;

fn float(field: &Field, key: u64, name: &str) -> f32 {
    field(key, name)
        .filter(|b| b.len() == 4)
        .map(|b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .filter(|f| f.is_finite())
        .unwrap_or(0.0)
}

fn word(field: &Field, key: u64, name: &str) -> u32 {
    field(key, name)
        .filter(|b| b.len() == 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .unwrap_or(0)
}

fn flag(field: &Field, key: u64, name: &str) -> bool {
    field(key, name).is_some_and(|b| b.first().is_some_and(|&v| v != 0))
}

/// A blend's child: the collection key its `RefSpec` names (none when empty).
fn child(field: &Field, key: u64, name: &str) -> Res<Option<u64>> {
    let Some(b) = field(key, name) else {
        return Ok(None);
    };
    if b.len() < 16 {
        return Err(format!("{CLASS} {key:016X}/{name}: not a reference").into());
    }
    let class = u64::from_be_bytes(b[..8].try_into().unwrap());
    let child = u64::from_be_bytes(b[8..16].try_into().unwrap());
    if child == 0 {
        return Ok(None);
    }
    if class != name_hash(CLASS) {
        return Err(format!("{CLASS} {key:016X}/{name}: refers outside the shots").into());
    }
    Ok(Some(child))
}

/// Every shot `roots` name, with the children of their blends, read through `field`.
pub fn shots(roots: &[String], field: &Field) -> Res<ShotSet> {
    let mut keys: Vec<u64> = Vec::new();
    let mut index: HashMap<u64, usize> = HashMap::new();
    let mut todo: Vec<u64> = roots.iter().map(|n| name_hash(n)).collect();
    let mut defs: Vec<(ShotDef, [Option<u64>; 3])> = Vec::new();
    while let Some(key) = todo.pop() {
        if index.contains_key(&key) {
            continue;
        }
        // A shot the graph names but the database doesn't have: left out (the game logs it).
        if field(key, "ShotType").is_none() && field(key, "PositionDistance").is_none() {
            continue;
        }
        let mut weights = [0.0; REFS];
        for (w, n) in weights.iter_mut().zip(WEIGHTS) {
            *w = float(field, key, n);
        }
        let mut options = 0;
        for (o, n) in [
            (OPT_FOLLOW, "OptionFollowSubjectInAir"),
            (OPT_MIRROR, "OptionMirrorForStance"),
            (OPT_PREVIOUS, "OptionUsePreviousShot"),
            (OPT_DROP, "OptionUseDropPredictor"),
            (OPT_AVOID, "OptionAvoidanceOverride"),
            (OPT_SNAP, "OptionSnapToReferencePoint"),
            (OPT_LOOK, "OptionUseFreeCamStick"),
        ] {
            if flag(field, key, n) {
                options |= o;
            }
        }
        let heading = float(field, key, "PositionHeading") * RAD;
        let elevation = float(field, key, "PositionElevation") * RAD;
        let shot = Shot {
            distance: float(field, key, "PositionDistance"),
            lens: float(field, key, "FramingLensLength"),
            smoothing: [
                float(field, key, "SmoothingDirection"),
                float(field, key, "SmoothingElevation"),
                float(field, key, "SmoothingYaw"),
                float(field, key, "SmoothingPitch"),
            ],
            weights,
            board_offset: float(field, key, "ReferenceBoardOffset"),
            heading,
            elevation,
            framing: [
                float(field, key, "FramingRoll") * RAD,
                float(field, key, "FramingYaw") * RAD,
                float(field, key, "FramingPitch") * RAD,
            ],
            options,
            anchor: word(field, key, "OptionAnchor").min(6) as u8,
            compass: word(field, key, "OptionCompassNorth").min(8) as u8,
            collision: word(field, key, "OptionCollisionType").min(255) as u8,
            arm_heading: heading,
            arm_elevation: elevation,
        };
        let blend = word(field, key, "ShotType") == 1;
        let mut kids = [None; 3];
        if blend {
            for (k, n) in ["BlendShot1", "BlendShot2", "BlendShot3"]
                .iter()
                .enumerate()
            {
                kids[k] = child(field, key, n)?;
                // Children stop at the first empty one (as the game reads them).
                match kids[k] {
                    Some(c) => todo.push(c),
                    None => break,
                }
            }
        }
        index.insert(key, defs.len());
        keys.push(key);
        defs.push((
            ShotDef {
                key,
                blend,
                shot,
                transition_time: float(field, key, "TransitionTime"),
                transition_units: word(field, key, "TransitionTimeUnits").min(255) as u8,
                children: [NONE; 3],
                points: [
                    float(field, key, "BlendPoint1"),
                    float(field, key, "BlendPoint2"),
                    float(field, key, "BlendPoint3"),
                ],
                blend_value: float(field, key, "BlendValue"),
                blend_type: word(field, key, "BlendType").min(255) as u8,
                blend_smoothing: float(field, key, "SmoothingBlendValue"),
            },
            kids,
        ));
        if defs.len() > 2000 {
            return Err("too many camera shots".into());
        }
    }
    let mut out = Vec::with_capacity(defs.len());
    for (mut d, kids) in defs {
        for (k, c) in kids.iter().enumerate() {
            if let Some(c) = c {
                let i = index
                    .get(c)
                    .ok_or_else(|| format!("camera shot {:016X}: missing child {c:016X}", d.key))?;
                d.children[k] = *i as u16;
            }
        }
        out.push(d);
    }
    let set = ShotSet {
        defs: out,
        imported: true,
    };
    // Reads back (no cycles, children in range).
    ShotSet::parse(&set.encode()).map_err(|e| format!("camera shots: {e:?}"))?;
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic shot database: a blend of two plain shots by speed.
    fn db() -> HashMap<(u64, String), Vec<u8>> {
        let mut m = HashMap::new();
        let mut put = |shot: &str, field: &str, v: Vec<u8>| {
            m.insert((name_hash(shot), field.to_string()), v);
        };
        let f = |x: f32| x.to_be_bytes().to_vec();
        let w = |x: u32| x.to_be_bytes().to_vec();
        let r = |name: &str| {
            let mut v = name_hash(CLASS).to_be_bytes().to_vec();
            v.extend_from_slice(&name_hash(name).to_be_bytes());
            v.extend_from_slice(&[0; 8]);
            v
        };
        put("mix", "ShotType", w(1));
        put("mix", "BlendType", w(1));
        put("mix", "BlendPoint1", f(3.0));
        put("mix", "BlendPoint2", f(9.0));
        put("mix", "BlendShot1", r("near"));
        put("mix", "BlendShot2", r("far"));
        put("mix", "BlendShot3", vec![0; 24]);
        put("mix", "TransitionTime", f(1.25));
        for (n, d) in [("near", 1.5), ("far", 3.0)] {
            put(n, "ShotType", w(0));
            put(n, "PositionDistance", f(d));
            put(n, "PositionHeading", f(-150.0));
            put(n, "FramingYaw", f(12.0));
            put(n, "OptionMirrorForStance", vec![1]);
            put(n, "OptionCompassNorth", w(7));
        }
        put("far", "OptionUseFreeCamStick", vec![1]);
        m
    }

    #[test]
    fn reads_a_shake_loop_centred() {
        // Two made-up samples: rotation x y z w, translation x y z w.
        let s = shake("2\n0.1 0.0 0.0 1.0  1.0 2.0 3.0 9.0\n0.3 0.0 0.0 1.0  3.0 2.0 1.0 9.0\n")
            .unwrap();
        assert_eq!(s.rotation.len(), 2);
        assert!((s.rotation[0][0] + 0.1).abs() < 1e-6 && (s.rotation[1][0] - 0.1).abs() < 1e-6);
        assert_eq!(s.translation[0], [-1.0, 0.0, 1.0]);
        assert!(Samples::parse(&s.encode()).is_some());
        assert!(shake("2\n1 2 3 4 5 6 7 8\n").is_err(), "cut short");
        assert!(shake("1\n1 2 3 4 5 6 7 8 9\n").is_err(), "too many");
        assert!(shake("").is_err());
    }

    #[test]
    fn reads_a_blend_and_its_children() {
        let m = db();
        let field = |k: u64, n: &str| m.get(&(k, n.to_string())).cloned();
        let set = shots(&["mix".to_string(), "missing".to_string()], &field).unwrap();
        assert_eq!(set.defs.len(), 3);
        let mix = &set.defs[set.find("mix").unwrap()];
        assert!(mix.blend);
        assert_eq!(mix.blend_type, 1);
        assert_eq!(mix.points[..2], [3.0, 9.0]);
        assert_eq!(mix.transition_time, 1.25);
        let near = &set.defs[mix.children[0] as usize];
        let far = &set.defs[mix.children[1] as usize];
        assert_eq!(mix.children[2], NONE);
        assert_eq!((near.shot.distance, far.shot.distance), (1.5, 3.0));
        assert!((near.shot.heading + 150.0 * RAD).abs() < 1e-6);
        assert!(near.shot.has(OPT_MIRROR));
        assert_eq!(near.shot.compass, 7);
        assert!(far.shot.has(OPT_LOOK) && !near.shot.has(OPT_LOOK));
        // Through the entry and back.
        let back = ShotSet::parse(&set.encode()).unwrap();
        assert_eq!(back.defs.len(), 3);
    }

    #[test]
    fn the_database_hash_is_the_shot_name_hash() {
        for n in ["a", "chase_one", "a_name_longer_than_twenty_four_bytes"] {
            assert_eq!(crate::attrib::hash(n), name_hash(n), "{n}");
        }
    }

    #[test]
    fn a_reference_to_another_class_is_refused() {
        let mut m = db();
        let mut bad = name_hash("other_class").to_be_bytes().to_vec();
        bad.extend_from_slice(&name_hash("near").to_be_bytes());
        bad.extend_from_slice(&[0; 8]);
        m.insert((name_hash("mix"), "BlendShot1".to_string()), bad);
        let field = |k: u64, n: &str| m.get(&(k, n.to_string())).cloned();
        assert!(shots(&["mix".to_string()], &field).is_err());
    }

    #[test]
    fn compiles_a_camera_graph() {
        use game_core::graph::write::Element;
        let root = Element::of(
            "state",
            &[("name", "Camera")],
            vec![
                Element::of(
                    "state",
                    &[("name", "Air")],
                    vec![
                        Element::of(
                            "expression",
                            &[],
                            vec![Element::of(
                                "condition",
                                &[
                                    ("name", "PredictedAirTime"),
                                    ("value", "0.5"),
                                    ("comparison", "GreaterThan"),
                                    ("mask", "always"),
                                ],
                                vec![],
                            )],
                        ),
                        Element::of(
                            "behaviour",
                            &[("name", "CameraChooseShot"), ("shot1", "Up_High")],
                            vec![],
                        ),
                    ],
                ),
                Element::of(
                    "state",
                    &[("name", "Ground")],
                    vec![Element::of(
                        "behaviour",
                        &[("name", "CameraChooseShot"), ("shot1", "ride")],
                        vec![],
                    )],
                ),
            ],
        );
        let compiled =
            game_core::graph::write::compile(&[(game_core::camera::select::GRAPH, &root)]).unwrap();
        assert_eq!(shots_named(&compiled).unwrap(), vec!["up_high", "ride"]);
        let mut g = game_core::camera::select::CamGraph::load(&compiled).unwrap();
        let mut c = game_core::camera::select::Conditions::default();
        assert_eq!(
            g.update(&c, 1.0 / 60.0, &game_core::tuning::CamTuning::BUILT_IN)[0].name,
            "ride"
        );
        c.air_time = 1.0;
        assert_eq!(
            g.update(&c, 1.0 / 60.0, &game_core::tuning::CamTuning::BUILT_IN)[0].name,
            "Up_High"
        );
    }
}
