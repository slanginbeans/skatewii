//! `fg_import motion`: Skate 3's on-board motion library as `motion.fgpak` (format: `game_core::motion`).
//!
//! - `motion/db`: every clip of the on-board animation bank with its timing, attribute tracks and events,
//!   and every animation tree (blend spaces, phase blends, selectors, selection spaces).
//! - `motion/clip/<NAME>`: poses (`game_core::anim::Clip`, bind pose composed in) for the clips that the
//!   on-board motion graph (`MotionGraph_OnBoard.stategraph`) plays, directly or through its trees, within
//!   a memory budget: graph areas are taken in [`AREAS`] order until the budget is used up.
//!
//! The bank and graph layouts come from SK8-ENGINE/skate-3-rust-engine (GPL-3.0).

use crate::anim::{self, Sqt, abin::Bank, meta};
use crate::xdvdfs::Disc;
use crate::{Res, pack, read_asset, stategraph};
use game_core::motion::write::{AttrIn, ClipIn, SimplexIn, TreeIn};
use game_core::pack::Kind;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};
use std::path::PathBuf;

const BANK: &str = "data/big/miscload.big::data/anim/OnBoard.abin";
/// Walking, running, getting on and off and carrying the board: the clips the graphs' `OffBoard` states play.
const OFF_BANK: &str = "data/big/miscload.big::data/anim/OffBoard.abin";
const GRAPH: &str = "data/big/miscload.big::data/state/MotionGraph_OnBoard.stategraph";
const ACTION_GRAPH: &str = "data/big/miscload.big::data/state/ActionGraph_OnBoard.stategraph";

/// Default pose budget (MiB). The Wii has 24 MiB MEM1 + 64 MiB MEM2; the park, skater and the game's own
/// heap need the rest (about 47 MiB was still free with a 20 MiB budget; the off-board clips take ~8.3).
const BUDGET_MIB: f64 = 30.0;

/// Graph areas in the order they are converted, with keywords for the graph's state names. A reference
/// belongs to the area of its most specific (deepest) state that has a keyword; references under no
/// keyword are `other`, last.
/// The fakie head and torso channel's tree (played by `game_core::graph` while riding fakie).
pub const FAKIE_CHANNEL: &str = game_core::graph::host::FAKIE_CHANNEL;

const OFF_AREA: &str = "offboard";

pub const AREAS: &[(&str, &[&str])] = &[
    ("riding", &["turning", "riding", "idle"]),
    ("push", &["push"]),
    ("brake", &["brake"]),
    ("anticipation", &["antic"]),
    ("tricks", &["trick", "flip", "shuv", "ollie"]),
    ("manual", &["manual"]),
    ("landing", &["land"]),
    ("kickturn", &["kickturn"]),
    ("revert", &["revert"]),
    ("switch", &["switch", "fakie"]),
    ("powerslide", &["slid"]),
    ("grind", &["grind"]),
    ("air", &["air"]),
    ("grab", &["grab"]),
    // Off the board: walking, running, getting off and on (also the on-board states that step off).
    // Last, so the riding keeps its clips if the budget is ever tight.
    (OFF_AREA, &["offboard", "mount"]),
];

struct Options {
    out: PathBuf,
    budget: f64,
    list: bool,
}

fn options(args: &[String]) -> Res<Options> {
    let mut o = Options {
        out: PathBuf::from("imported"),
        budget: BUDGET_MIB,
        list: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or(crate::USAGE);
        match a.as_str() {
            "--out" => o.out = PathBuf::from(val()?),
            "--budget" => o.budget = val()?.parse()?,
            "--list" => o.list = true,
            _ => return Err(format!("unknown option {a}\n{}", crate::USAGE).into()),
        }
    }
    Ok(o)
}

/// States that are not about skating the board itself (hanging on to cars, handplants, getting on and off,
/// riding moving objects): `other`, whatever else their names say.
const OTHER: &[&str] = &["skitch", "handplant", "movingobject", "coffin", "teleport"];

/// The area of a graph reference from its state path (`Motion/OnBoard/...`).
pub fn area(path: &str) -> usize {
    // Everything under an off-board state is off-board, whatever its own name says (`Land`, `InAir`).
    if path
        .split('/')
        .any(|s| s.to_ascii_lowercase().starts_with("offboard"))
    {
        return AREAS
            .iter()
            .position(|a| a.0 == OFF_AREA)
            .unwrap_or(AREAS.len());
    }
    for segment in path.rsplit('/') {
        let segment = segment.to_ascii_lowercase();
        if OTHER.iter().any(|k| segment.contains(k)) {
            return AREAS.len();
        }
        if let Some(i) = AREAS
            .iter()
            .position(|(_, keys)| keys.iter().any(|k| segment.contains(k)))
        {
            return i;
        }
    }
    AREAS.len()
}

pub fn area_name(i: usize) -> &'static str {
    AREAS.get(i).map_or("other", |a| a.0)
}

/// Every clip and tree `name` plays, and names it refers to that the bank doesn't have (or can't decode).
pub fn closure(
    m: &meta::Meta,
    name: &str,
    clips: &mut BTreeSet<String>,
    trees: &mut BTreeSet<String>,
    missing: &mut BTreeSet<String>,
) {
    let mut todo = vec![name.to_ascii_uppercase()];
    while let Some(n) = todo.pop() {
        match m.get(&n) {
            Some(meta::Animation::Clip(_)) => {
                clips.insert(n);
            }
            Some(meta::Animation::Tree(t)) => {
                if trees.insert(n) {
                    todo.extend(t.children().into_iter().map(str::to_ascii_uppercase));
                }
            }
            _ => {
                missing.insert(n);
            }
        }
    }
}

fn tree_in(t: &meta::Tree) -> TreeIn {
    match t {
        meta::Tree::BlendSpace {
            parameters,
            children,
            simplexes,
        } => TreeIn::BlendSpace {
            parameters: parameters.clone(),
            children: children.clone(),
            simplexes: simplexes
                .iter()
                .map(|s| SimplexIn {
                    children: s.children.iter().map(|&c| c as u16).collect(),
                    vertices: s.vertices.clone(),
                    normals: s.normals.clone(),
                    scales: s.scales.clone(),
                })
                .collect(),
        },
        meta::Tree::PhaseBlend {
            parameter,
            children,
        } => TreeIn::PhaseBlend {
            parameter: parameter.clone(),
            children: children.clone(),
        },
        meta::Tree::Selector {
            parameter,
            default,
            children,
            values,
        } => TreeIn::Selector {
            parameter: parameter.clone(),
            default: default.clone(),
            options: values
                .iter()
                .cloned()
                .zip(children.iter().cloned())
                .collect(),
        },
        meta::Tree::SelectionSpace {
            parameters,
            candidates,
        } => TreeIn::SelectionSpace {
            parameters: parameters
                .iter()
                .map(|p| (p.name.clone(), p.mode, p.weight, p.minimum, p.maximum))
                .collect(),
            candidates: candidates.clone(),
        },
    }
}

/// A clip's pose entry: decoded, bind pose composed in, quantised.
/// The clip's pose from the first bank that has it (the on-board bank wins a name both have, as in
/// [`meta::Meta::merge`]), with that bank's bind pose composed in.
fn encode_pose(banks: &[(Bank, Vec<Sqt>)], name: &str) -> Res<Vec<u8>> {
    for (bank, bind) in banks {
        let Some(record) = bank.records().iter().rev().find(|r| {
            r.header.name.eq_ignore_ascii_case(name)
                && matches!(r.data, anim::abin::RecordData::Clip(_))
        }) else {
            continue;
        };
        let mut frames = anim::decode_clip(bank, record)?;
        anim::add_bind_pose(&mut frames, bind);
        return Ok(anim::encode_clip(&frames));
    }
    Err(format!("no clip {name}").into())
}

pub fn run<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    let o = options(args)?;
    let bank = Bank::parse(read_asset(disc, BANK)?)?;
    let mut m = meta::Meta::read(&bank)?;
    let bind = anim::bind_pose(&bank).ok_or("bank has no RIG_TPOSE bind pose")?;
    let mut banks = vec![(bank, bind)];
    // The off-board bank: its animations under the names the on-board bank doesn't use.
    match read_asset(disc, OFF_BANK)
        .map_err(|e| e.to_string())
        .and_then(|b| Bank::parse(b).map_err(|e| e.to_string()))
    {
        Ok(off) => {
            let added = m.merge(meta::Meta::read(&off)?);
            let off_bind = anim::bind_pose(&off).unwrap_or_else(|| banks[0].1.clone());
            println!("off-board bank: {} animations added", added.len());
            banks.push((off, off_bind));
        }
        Err(e) => eprintln!("no off-board bank ({e}): walking will use placeholders"),
    }
    let graph = read_asset(disc, GRAPH)?;
    let attrs = stategraph::attributes(&graph)?;

    // Graph references: each bank animation named anywhere in the graph, with its area.
    let mut roots: BTreeMap<String, usize> = BTreeMap::new();
    for a in &attrs {
        if m.contains(a.text) {
            let ar = area(&a.path);
            if o.list {
                println!(
                    "  reference {:<13} {:<40} {}",
                    area_name(ar),
                    a.text,
                    a.path
                );
            }
            let e = roots.entry(a.text.to_ascii_uppercase()).or_insert(ar);
            *e = (*e).min(ar);
        }
    }
    // Trees the runner plays by name rather than from a graph attribute: the fakie head and torso
    // channel (SK8 FakieHeadChannel).
    // The pump layer (Pumping) and the d-pad gestures' clips (CharacterGesture), last.
    let mut named: Vec<(String, usize)> = vec![
        (FAKIE_CHANNEL.to_string(), 0usize),
        (game_core::graph::host::PUMP_TREE.to_string(), 0),
    ];
    for d in 0..4 {
        for stage in 0..3 {
            named.push((game_core::graph::host::gesture_clip(d, stage), AREAS.len()));
        }
    }
    for (name, ar) in &named {
        if m.contains(name) {
            let e = roots.entry(name.to_ascii_uppercase()).or_insert(*ar);
            *e = (*e).min(*ar);
        } else if o.list {
            println!("  (no {name} in the bank)");
        }
    }
    // Per area: the clips it needs that no earlier area brought in.
    let mut per_area: Vec<(BTreeSet<String>, usize)> = vec![(BTreeSet::new(), 0); AREAS.len() + 1];
    let (mut all_clips, mut all_trees, mut missing) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    let mut by_area: Vec<Vec<&String>> = vec![Vec::new(); AREAS.len() + 1];
    for (name, &ar) in &roots {
        by_area[ar].push(name);
    }
    for (ar, names) in by_area.iter().enumerate() {
        for name in names {
            let mut clips = BTreeSet::new();
            closure(&m, name, &mut clips, &mut all_trees, &mut missing);
            for c in clips {
                if all_clips.insert(c.clone()) {
                    per_area[ar].0.insert(c);
                }
            }
        }
        per_area[ar].1 = names.len();
    }
    // Every pose the graph can play, converted, so the budget is measured on the real sizes.
    let mut poses: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut failed = Vec::new();
    for name in &all_clips {
        match encode_pose(&banks, name) {
            Ok(data) => {
                poses.insert(name.clone(), data);
            }
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    if !failed.is_empty() {
        eprintln!(
            "{} clips not converted: {}",
            failed.len(),
            failed.join("; ")
        );
    }
    let size_of = |c: &String| poses.get(c).map_or(0, Vec::len);
    // Take whole areas while they fit.
    let budget = (o.budget * 1024.0 * 1024.0) as usize;
    let mut chosen = BTreeSet::new();
    let mut used = 0usize;
    let mut left_out = Vec::new();
    println!("graph references: {} animations, by area:", roots.len());
    for (ar, (clips, refs)) in per_area.iter().enumerate() {
        let size: usize = clips.iter().map(size_of).sum();
        let fits = used + size <= budget;
        println!(
            "  {:<13} {:>4} references, {:>4} new clips, {:>6} KiB{}",
            area_name(ar),
            refs,
            clips.len(),
            size / 1024,
            if fits {
                ""
            } else {
                "  (left out: over budget)"
            }
        );
        if fits {
            used += size;
            chosen.extend(clips.iter().cloned());
        } else {
            left_out.push(area_name(ar));
        }
    }
    if !missing.is_empty() {
        println!(
            "{} names the trees refer to are not decodable clips or trees: {}",
            missing.len(),
            missing
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // The database: every clip of the bank, every tree.
    let mut clips_in = Vec::new();
    let mut seen = BTreeSet::new();
    for c in m.clips.iter().rev() {
        let n = c.name.to_ascii_uppercase();
        // The bank's last record for a name wins (a later tree of the same name hides the clip).
        if !seen.insert(n.clone()) || !matches!(m.get(&n), Some(meta::Animation::Clip(_))) {
            continue;
        }
        clips_in.push(ClipIn {
            name: n.clone(),
            fps: c.fps,
            frames: c.frames,
            base_speed: c.base_speed,
            andale_flags: c.flags,
            has_pose: chosen.contains(&n) && poses.contains_key(&n),
            attrs: c
                .attributes
                .iter()
                .map(|a| AttrIn {
                    name: a.name.clone(),
                    kind: a.kind,
                    begin: a.begin,
                    end: a.end,
                    words: a.words.clone(),
                })
                .collect(),
        });
    }
    let mut trees_in = Vec::new();
    let mut seen_trees = BTreeSet::new();
    for (n, t) in m.trees.iter().rev() {
        let n = n.to_ascii_uppercase();
        // The bank's last record for a name wins.
        if seen_trees.insert(n.clone()) && matches!(m.get(&n), Some(meta::Animation::Tree(_))) {
            trees_in.push((n, tree_in(t)));
        }
    }
    let db = game_core::motion::write::write(&clips_in, &trees_in);
    // Check it reads back.
    let parsed =
        game_core::motion::MotionDb::parse(&db).map_err(|e| format!("motion db: {e:?}"))?;

    // Every animation the graph names must build as a playable tree (all children present).
    let mut broken = Vec::new();
    let mut no_pose = BTreeSet::new();
    for name in roots.keys() {
        match game_core::motion::tree::Playback::build(&parsed, name, &[]) {
            Ok(mut p) => {
                // Its first pose, with every parameter at its default: are those clips' poses here?
                let mut out = Vec::new();
                p.evaluate(0.01, true, true, &mut out);
                for (c, _, _) in game_core::motion::tree::leaf_weights(&out) {
                    if !parsed.clip(c).has_pose {
                        no_pose.insert(parsed.name(parsed.clip(c).name));
                    }
                }
            }
            Err(e) => broken.push(format!("{name} ({e:?})")),
        }
    }
    if !no_pose.is_empty() {
        println!(
            "{} clips the graph plays have no pose in the pack (over budget)",
            no_pose.len()
        );
    }
    println!(
        "{} of {} graph animations build as playable trees{}",
        roots.len() - broken.len(),
        roots.len(),
        if broken.is_empty() {
            String::new()
        } else {
            format!("; not: {}", broken.join(", "))
        }
    );

    let mut items = vec![pack::Item {
        name: "motion/db".into(),
        kind: Kind::MotionDb,
        data: db.clone(),
        width: 0,
        height: 0,
        gx_format: 0,
    }];
    // Both state graphs, compiled for the game's graph runner (`game_core::graph`).
    let action = read_asset(disc, ACTION_GRAPH)?;
    let graphs = game_core::graph::write::compile(&[
        (
            game_core::graph::host::ACTION,
            &stategraph::elements(&action)?,
        ),
        (
            game_core::graph::host::MOTION,
            &stategraph::elements(&graph)?,
        ),
    ])?;
    let parsed_graphs = game_core::graph::Pack::parse(&graphs)
        .map_err(|e| format!("compiled graphs don't read back: {e:?}"))?;
    for g in &parsed_graphs.graphs {
        println!(
            "graph {}: {} states, {} transitions, {} expressions, {} operations, {} attributes",
            parsed_graphs.str(g.name),
            g.states.len(),
            g.transitions.len(),
            g.expressions.len(),
            g.ops.len(),
            g.attrs.len()
        );
    }
    println!(
        "graphs: {} strings, {} KiB in the pack",
        parsed_graphs.strings(),
        graphs.len() / 1024
    );
    // Their operations typed once here (names resolved against the graph strings and the motion
    // library), so the game reads them back instead of parsing them at load.
    let parsed_db =
        game_core::motion::MotionDb::parse(&db).map_err(|e| format!("motion db: {e:?}"))?;
    let typed: Vec<_> = parsed_graphs
        .graphs
        .iter()
        .map(|g| game_core::graph::ops::parse(&parsed_graphs, g, Some(&parsed_db)))
        .collect();
    let ops = game_core::graph::opcode::encode(graphs.len(), &typed);
    if game_core::graph::opcode::decode(&ops, graphs.len()).as_ref() != Some(&typed) {
        return Err("typed graph operations don't read back".into());
    }
    println!("graph operations: {} KiB typed", ops.len() / 1024);
    items.push(pack::Item {
        name: "motion/graph".into(),
        kind: Kind::StateGraph,
        data: graphs,
        width: 0,
        height: 0,
        gx_format: 0,
    });
    items.push(pack::Item {
        name: game_core::graph::opcode::ENTRY.into(),
        kind: Kind::Data,
        data: ops,
        width: 0,
        height: 0,
        gx_format: 0,
    });
    let mut pose_bytes = 0usize;
    for (name, data) in poses {
        if chosen.contains(&name) {
            pose_bytes += data.len();
            items.push(pack::Item {
                name: format!("motion/clip/{name}"),
                kind: Kind::Animation,
                data,
                width: 0,
                height: 0,
                gx_format: 0,
            });
        }
    }

    // What the attributes are: name, how many clips carry it, kinds, timed or not.
    let mut attr_stats: BTreeMap<&str, (usize, BTreeSet<u8>, bool)> = BTreeMap::new();
    for c in &m.clips {
        for a in &c.attributes {
            let e = attr_stats.entry(&a.name).or_default();
            e.0 += 1;
            e.1.insert(a.kind);
            e.2 |= a.begin != -1.0;
        }
    }
    let kinds = trees_in.iter().fold([0usize; 12], |mut k, (_, t)| {
        k[match t {
            TreeIn::BlendSpace { .. } => 6,
            TreeIn::PhaseBlend { .. } => 7,
            TreeIn::Selector { .. } => 8,
            TreeIn::SelectionSpace { .. } => 11,
        }] += 1;
        k
    });
    println!(
        "bank: {} clips, {} trees ({} blend spaces, {} phase blends, {} selectors, {} selection spaces), \
         {} of tree types 9/10 not decoded",
        parsed.clips().len(),
        parsed.trees().len(),
        kinds[6],
        kinds[7],
        kinds[8],
        kinds[11],
        m.unsupported.len()
    );
    println!(
        "attributes: {} records, {} names; the graph plays {} clips through {} trees",
        parsed.attribute_count(),
        attr_stats.len(),
        all_clips.len(),
        all_trees.len()
    );
    if o.list {
        for (name, (count, kinds, timed)) in &attr_stats {
            println!(
                "  attribute {name:<30} {count:>5} clips, kind {kinds:?}{}",
                if *timed { ", timed" } else { "" }
            );
        }
    }
    std::fs::create_dir_all(&o.out)?;
    let bytes = pack::write(&items);
    let path = o.out.join("motion.fgpak");
    std::fs::write(&path, &bytes)?;
    println!(
        "{} clip poses ({} KiB) + database ({} KiB) -> {} ({} KiB){}",
        items.len() - 2,
        pose_bytes / 1024,
        db.len() / 1024,
        path.display(),
        bytes.len() / 1024,
        if left_out.is_empty() {
            String::new()
        } else {
            format!("; left out: {}", left_out.join(", "))
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_by_the_deepest_state_with_a_keyword() {
        let a = |p: &str| area_name(area(p));
        assert_eq!(
            a("Motion/OnBoard/Riding/Turning/Push/PushStart/Left"),
            "push"
        );
        assert_eq!(a("Motion/OnBoard/Riding/Turning/Idle/Default"), "riding");
        assert_eq!(a("Motion/OnBoard/InAir/GrabsTweaks/MuteGrab"), "grab");
        assert_eq!(a("Motion/OnBoard/InAir/BoardAdjusts/Up"), "air");
        assert_eq!(a("Motion/Teleport"), "other");
        assert_eq!(a("Motion/OnBoard/OnGround/Skitching/SkitchPush"), "other");
    }
}
