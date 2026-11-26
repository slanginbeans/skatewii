//! Synthetic graphs (built here, nothing from a disc) through the compiler, the reader and the runner.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::write::{Element, compile};
use super::{Controller, Frame, Graph, Host, NONE, OpId, Pack};

fn st(name: &str, attrs: &[(&str, &str)], children: Vec<Element>) -> Element {
    let mut a = vec![("name", name)];
    a.extend_from_slice(attrs);
    Element::of("state", &a, children)
}

/// A condition `Flag` named `flag` (true while the test host has it set).
fn flag(name: &str, mask: &str) -> Element {
    Element::of(
        "condition",
        &[("name", "Flag"), ("flag", name), ("mask", mask)],
        vec![],
    )
}

fn when(conds: Vec<Element>) -> Element {
    Element::of("expression", &[("op", "and")], conds)
}

fn go(target: &str, priority: Option<&str>, conds: Vec<Element>) -> Element {
    let mut a = vec![("target", target)];
    if let Some(p) = priority {
        a.push(("priority", p));
    }
    Element::of("transition", &a, vec![when(conds)])
}

fn beh(tag: &str) -> Element {
    Element::of("behaviour", &[("name", "Log"), ("tag", tag)], vec![])
}

struct TestHost<'p> {
    pack: &'p Pack<'p>,
    g: &'p Graph,
    flags: Vec<&'static str>,
    log: Vec<String>,
}

impl TestHost<'_> {
    fn attr(&self, op: OpId, key: &str) -> &str {
        self.pack.text(self.g.op_attrs(op), key).unwrap_or("")
    }
}

impl Host for TestHost<'_> {
    fn condition(&mut self, op: OpId, frame: &Frame) -> bool {
        let name = self.pack.str(self.g.ops[op as usize].name);
        match name {
            "Flag" => self.flags.contains(&self.attr(op, "flag")),
            "InParentStateForTime" | "InStateForTime" => {
                let s = self.g.ops[op as usize].bound;
                let t = self.pack.float(self.g.op_attrs(op), "greater", 0.0);
                s != NONE && frame.time_in(s).is_some_and(|x| x > t)
            }
            _ => false,
        }
    }
    fn begin(&mut self, op: OpId, _: &Frame) {
        let t = self.attr(op, "tag");
        self.log.push(alloc::format!("begin {t}"));
    }
    fn update(&mut self, _: OpId, _: &Frame) {}
    fn end(&mut self, op: OpId, _: &Frame) {
        let t = self.attr(op, "tag");
        self.log.push(alloc::format!("end {t}"));
    }
    fn hook(&mut self, op: OpId, _: &Frame) {
        let t = self.attr(op, "tag");
        self.log.push(alloc::format!("hook {t}"));
    }
}

fn path(p: &Pack, g: &Graph, c: &Controller) -> String {
    p.state_path(g, c.frame.current)
}

/// Root
///   Ground (interruptable)            [Idle (default leaf), Push (needs push, precond)]
///   Air (precond air, not interruptable)
fn riding() -> Element {
    st(
        "Root",
        &[],
        vec![
            st(
                "Ground",
                &[],
                vec![
                    Element::of(
                        "expression",
                        &[("op", "and")],
                        vec![flag("ground", "always")],
                    ),
                    st(
                        "Push",
                        &[],
                        vec![
                            when(vec![flag("push", "always")]),
                            beh("push"),
                            go("Idle", None, vec![flag("stop", "precond")]),
                        ],
                    ),
                    st("Idle", &[], vec![beh("idle")]),
                ],
            ),
            st(
                "Air",
                &[("interruptable", "false")],
                vec![
                    when(vec![flag("air", "precond")]),
                    beh("air"),
                    go("Ground", Some("high"), vec![flag("ground", "precond")]),
                ],
            ),
        ],
    )
}

fn run(root: &Element, steps: &[&[&'static str]]) -> (Vec<String>, Vec<String>) {
    let bytes = compile(&[("G", root)]).unwrap();
    let pack = Pack::parse(&bytes).unwrap();
    let g = &pack.graphs[0];
    let mut c = Controller::new(g);
    let mut h = TestHost {
        pack: &pack,
        g,
        flags: Vec::new(),
        log: Vec::new(),
    };
    let mut paths = Vec::new();
    for s in steps {
        h.flags = s.to_vec();
        c.update(g, 1.0 / 60.0, &mut h);
        paths.push(path(&pack, g, &c));
    }
    (paths, h.log)
}

#[test]
fn compiles_and_reads_back() {
    let bytes = compile(&[("G", &riding())]).unwrap();
    let p = Pack::parse(&bytes).unwrap();
    let g = p.graph("G").unwrap();
    assert_eq!(g.states.len(), 5);
    assert_eq!(p.state_name(g, g.root), "Root");
    assert_eq!(g.transitions.len(), 2);
    let t = g.transitions[1];
    assert_eq!(p.state_name(g, t.target), "Ground");
    assert_eq!(t.priority, 2);
    assert!(!g.states[4].interruptable());
    assert!(Pack::parse(&bytes[..bytes.len() / 2]).is_err());
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert!(Pack::parse(&bad).is_err());
}

impl super::State {
    fn interruptable(&self) -> bool {
        self.interruptable != 0
    }
}

#[test]
fn descends_through_the_first_active_children_in_order() {
    let (paths, log) = run(&riding(), &[&["ground"], &["ground", "push"], &["ground"]]);
    // Push needs `push` (always: to enter and to stay): Idle first. An interruptable state re-resolves
    // every tick, so the push is picked up, and dropped again when the flag goes.
    assert_eq!(paths, ["Ground/Idle", "Ground/Push", "Ground/Idle"]);
    assert_eq!(
        log,
        [
            "begin idle",
            "end idle",
            "begin push",
            "end push",
            "begin idle"
        ]
    );
}

#[test]
fn non_interruptable_states_stay_while_sustained_and_leave_by_transition() {
    let (paths, log) = run(&riding(), &[&["air"], &[], &["air"], &["ground", "push"]]);
    // In the air with no flags: Air isn't interruptable and has no sustain condition, so it stays.
    assert_eq!(paths, ["Air", "Air", "Air", "Ground/Push"]);
    assert_eq!(log, ["begin air", "end air", "begin push"]);
}

#[test]
fn urgent_transitions_win_and_disabled_states_are_skipped() {
    let root = st(
        "Root",
        &[],
        vec![
            st(
                "A",
                &[("interruptable", "false")],
                vec![
                    go("B", None, vec![flag("x", "precond")]),
                    go("C", Some("urgent"), vec![flag("x", "precond")]),
                ],
            ),
            st("B", &[], vec![]),
            st("C", &[], vec![]),
            st("Off", &[("active", "false")], vec![]),
        ],
    );
    let (paths, _) = run(&root, &[&[], &["x"]]);
    assert_eq!(paths, ["A", "C"]);
}

#[test]
fn sustain_conditions_hold_a_state_and_always_both() {
    let root = st(
        "Root",
        &[],
        vec![
            st(
                "Hold",
                &[("interruptable", "false")],
                vec![when(vec![
                    flag("enter", "precond"),
                    flag("keep", "sustain"),
                ])],
            ),
            st("Else", &[], vec![]),
        ],
    );
    let (paths, _) = run(
        &root,
        &[&["enter"], &["keep"], &["enter", "keep"], &[], &["keep"]],
    );
    // Entering needs `enter` only; staying needs `keep` only.
    assert_eq!(paths, ["Hold", "Hold", "Hold", "Else", "Else"]);
}

#[test]
fn preconditions_only_count_when_entering() {
    let root = st(
        "Root",
        &[],
        vec![
            st("Push", &[], vec![when(vec![flag("push", "precond")])]),
            st("Idle", &[], vec![]),
        ],
    );
    // Re-resolving checks states on the current path with their sustain mask: a precondition no longer
    // holding doesn't move the rider out.
    let (paths, _) = run(&root, &[&["push"], &[]]);
    assert_eq!(paths, ["Push", "Push"]);
}

#[test]
fn interrupt_ancestors_reresolve_below_them() {
    let root = st(
        "Root",
        &[],
        vec![
            st(
                "Trick",
                &[],
                vec![
                    st(
                        "Flip",
                        &[("interruptable", "Trick")],
                        vec![when(vec![flag("flip", "always")])],
                    ),
                    st("Hold", &[], vec![]),
                ],
            ),
            st("Other", &[], vec![]),
        ],
    );
    let (paths, _) = run(&root, &[&["flip"], &[]]);
    assert_eq!(paths, ["Trick/Flip", "Trick/Hold"]);
}

#[test]
fn parent_state_time_binds_to_the_owner_and_hooks_run_between_exit_and_enter() {
    let wait = Element::of(
        "condition",
        &[("name", "InParentStateForTime"), ("greater", "0.05")],
        vec![],
    );
    let mut t = go("B", None, vec![wait]);
    t.children.push(Element::of(
        "hook",
        &[("name", "Log"), ("tag", "h")],
        vec![],
    ));
    let root = st(
        "Root",
        &[],
        vec![
            st("A", &[("interruptable", "false")], vec![beh("a"), t]),
            st("B", &[("interruptable", "false")], vec![beh("b")]),
        ],
    );
    let steps: Vec<&[&str]> = vec![&[]; 6];
    let (paths, log) = run(&root, &steps);
    // 0.05 s is three ticks in A, then the move.
    assert_eq!(paths, ["A", "A", "A", "A", "B", "B"]);
    assert_eq!(log, ["begin a", "end a", "hook h", "begin b"]);
}

#[test]
fn transitions_find_targets_by_name_up_the_tree_and_dotted() {
    let root = st(
        "Root",
        &[],
        vec![
            st(
                "A",
                &[("interruptable", "false")],
                vec![go("B.Inner", None, vec![flag("x", "precond")])],
            ),
            st(
                "B",
                &[],
                vec![st("Other", &[], vec![]), st("Inner", &[], vec![])],
            ),
        ],
    );
    let (paths, _) = run(&root, &[&[], &["x"]]);
    assert_eq!(paths, ["A", "B/Inner"]);
}

#[test]
fn unknown_elements_and_bad_parents_are_errors() {
    let bad = st("Root", &[], vec![Element::of("thing", &[], vec![])]);
    assert!(compile(&[("G", &bad)]).is_err());
    let bad = st("Root", &[], vec![flag("x", "precond")]);
    assert!(compile(&[("G", &bad)]).is_err());
}

mod runner {
    use super::*;
    use crate::graph::host::{Runner, group_trick, mirror_trick};
    use crate::graph::ops::Group;

    #[test]
    fn gesture_groups_fit_flicks_to_the_board_end() {
        let t = |g, s| group_trick(g, s).map(|x| x.0);
        assert_eq!(t(Group::Square, "Kickflip").as_deref(), Some("Kickflip"));
        assert_eq!(
            t(Group::Square, "N_Kickflip").as_deref(),
            Some("N_Kickflip")
        );
        // Nose manual: a tail flick becomes a nollie, a tail shove-it the nose one.
        assert_eq!(t(Group::Nose, "Kickflip").as_deref(), Some("Nollie"));
        assert_eq!(
            t(Group::Nose, "360PopShuvit").as_deref(),
            Some("N_PopShuvit")
        );
        assert_eq!(
            t(Group::Tail, "N_FSPopShuvit").as_deref(),
            Some("FSPopShuvit")
        );
        assert_eq!(t(Group::Tail, "Heelflip").as_deref(), Some("Heelflip"));
        // 90 groups: their own prefixed flicks, and the plain ones.
        assert_eq!(t(Group::Tail90, "90_Kickflip").as_deref(), Some("Kickflip"));
        assert_eq!(t(Group::Tail90, "90_N_Kickflip"), None);
        assert_eq!(
            t(Group::NoseN90, "N90_N_Heelflip").as_deref(),
            Some("N_Heelflip")
        );
        assert_eq!(t(Group::Square, "90_Kickflip"), None);
        assert_eq!(t(Group::Square, "SlideFs180"), None);
        assert_eq!(mirror_trick("N_Hardflip"), "N_InwardHeelflip");
        assert_eq!(mirror_trick("Ollie"), "Ollie");
    }

    /// A tiny action graph and motion graph: a push button becomes a motion intent that moves the motion
    /// graph into Push.
    fn graphs() -> Vec<u8> {
        let action = st(
            "Action",
            &[],
            vec![Element::of(
                "behaviour",
                &[
                    ("name", "CreateMGIntentFromAGIntent"),
                    ("MGIntent", "Push"),
                    ("AGIntent", "RightPush"),
                ],
                vec![],
            )],
        );
        let has = Element::of(
            "condition",
            &[
                ("name", "HasIntent"),
                ("intent", "Push"),
                ("mask", "always"),
            ],
            vec![],
        );
        let motion = st(
            "Motion",
            &[],
            vec![
                st("Push", &[], vec![when(vec![has])]),
                st("Idle", &[], vec![]),
            ],
        );
        compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    #[test]
    fn action_intents_reach_the_motion_graph() {
        let bytes = graphs();
        let pack = Pack::parse(&bytes).unwrap();
        let mut r = Runner::new(pack, None).unwrap();
        r.update(None, 1.0 / 60.0);
        assert_eq!(r.motion_state(), "Idle");
        let push = r.pack.find("RightPush").unwrap();
        r.s.ag.set(push, 1.0);
        r.update(None, 1.0 / 60.0);
        assert_eq!(r.motion_state(), "Push");
        r.s.ag.clear();
        r.update(None, 1.0 / 60.0);
        r.update(None, 1.0 / 60.0);
        assert_eq!(r.motion_state(), "Idle");
        assert!(r.coverage().2.is_empty());
    }

    /// An action graph running BodyFlippingSignal and a motion graph with a state held by
    /// `IsBodyFlipping`, and the flip intents attached for the physics.
    fn flip_graphs() -> Vec<u8> {
        let action = st(
            "Action",
            &[],
            vec![Element::of(
                "behaviour",
                &[("name", "BodyFlippingSignal")],
                vec![],
            )],
        );
        let flipping = Element::of(
            "condition",
            &[("name", "IsBodyFlipping"), ("mask", "always")],
            vec![],
        );
        let attach = |n: &'static str| {
            Element::of(
                "behaviour",
                &[("name", "AttachIntent"), ("intent", n), ("attr", n)],
                vec![],
            )
        };
        let motion = st(
            "Motion",
            &[],
            vec![
                attach("FrontFlip"),
                attach("BackFlip"),
                st("Flipping", &[], vec![when(vec![flipping])]),
                st("Idle", &[], vec![]),
            ],
        );
        compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    #[test]
    fn a_flip_gesture_just_after_take_off_asks_the_physics_for_a_flip() {
        use crate::graph::ops::phys;
        let bytes = flip_graphs();
        let mut r = Runner::new(Pack::parse(&bytes).unwrap(), None).unwrap();
        let dt = 1.0 / 60.0;
        let back = r.pack.find("BackFlip").unwrap();
        // On the ground the gesture does nothing.
        r.s.ag.set(back, 1.0);
        r.update(None, dt);
        assert_eq!(r.packet_named("BackFlip"), None);
        // In the air, early: the motion intent, attached for the physics.
        r.s.world.phys = phys::AIR;
        r.update(None, dt);
        assert_eq!(r.packet_named("BackFlip"), Some(1.0));
        assert_eq!(r.packet_named("FrontFlip"), None);
        // The gesture let go: it still counts a moment, then not.
        r.s.ag.clear();
        r.update(None, dt);
        assert_eq!(r.packet_named("BackFlip"), Some(1.0));
        for _ in 0..30 {
            r.update(None, dt);
        }
        assert_eq!(r.packet_named("BackFlip"), None);
        // The physics flipping holds the state that reads it.
        assert_eq!(r.motion_state(), "Idle");
        r.s.world.body_flipping = true;
        r.update(None, dt);
        assert_eq!(r.motion_state(), "Flipping");
    }

    #[test]
    fn pad_intents_from_the_sticks_and_buttons() {
        use crate::graph::input::{Gates, Intentions};
        use crate::input::{Pad, btn};
        let bytes = graphs();
        let pack = Pack::parse(&bytes).unwrap();
        let mut input = Intentions::new(&pack, vec![("skater", crate::gesture::built_in())]);
        let mut r = Runner::new(Pack::parse(&bytes).unwrap(), None).unwrap();
        let mut rec = Vec::new();
        let pad = Pad {
            held: btn::A,
            ..Pad::default()
        };
        input.produce(
            &r.pack,
            &pad,
            Gates::default(),
            1.0 / 60.0,
            &mut r.s.ag,
            &mut rec,
        );
        assert!(r.s.ag.has(r.pack.find("RightPush").unwrap()));
        // A and B together: both pushes, which the action graph takes as the hippie jump.
        let pad = Pad {
            held: btn::A | btn::B,
            ..Pad::default()
        };
        input.produce(
            &r.pack,
            &pad,
            Gates::default(),
            1.0 / 60.0,
            &mut r.s.ag,
            &mut rec,
        );
        assert!(r.s.ag.has(r.pack.find("RightPush").unwrap()));
    }

    #[test]
    fn the_d_pad_asks_for_gestures() {
        use crate::graph::input::{Gates, Intentions};
        use crate::graph::write::Element;
        use crate::input::{Pad, btn};
        let action = st(
            "Action",
            &[],
            vec![
                Element::of(
                    "behaviour",
                    &[
                        ("name", "CreateMGIntentFromAGIntent"),
                        ("MGIntent", "GestureUpStart"),
                        ("AGIntent", "GestureUpStart"),
                    ],
                    vec![],
                ),
                Element::of(
                    "behaviour",
                    &[
                        ("name", "CreateMGIntentFromAGIntent"),
                        ("MGIntent", "GestureUpHeld"),
                        ("AGIntent", "GestureUpHeld"),
                    ],
                    vec![],
                ),
            ],
        );
        let motion = st("Motion", &[], vec![st("Idle", &[], vec![])]);
        let bytes = compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap();
        let pack = Pack::parse(&bytes).unwrap();
        let mut input = Intentions::new(&pack, Vec::new());
        let mut r = Runner::new(Pack::parse(&bytes).unwrap(), None).unwrap();
        let mut rec = Vec::new();
        let start = r.pack.find("GestureUpStart").unwrap();
        let held = r.pack.find("GestureUpHeld").unwrap();
        let mut press = |r: &mut Runner, buttons: u32| {
            let pad = Pad {
                held: buttons,
                ..Pad::default()
            };
            input.produce(
                &r.pack,
                &pad,
                Gates::default(),
                1.0 / 60.0,
                &mut r.s.ag,
                &mut rec,
            );
            r.update(None, 1.0 / 60.0);
        };
        press(&mut r, btn::DPAD_UP);
        assert!(r.s.mg.has(start) && r.s.mg.has(held));
        // Held: no new start.
        press(&mut r, btn::DPAD_UP);
        assert!(!r.s.mg.has(start) && r.s.mg.has(held));
        press(&mut r, 0);
        assert!(!r.s.mg.has(held));
    }

    /// Off the board in the air (SK8 offboard_intentions.rs by position): the C-stick is always
    /// `OB_AirBodyTweakX/Y`, Z (RB) `OB_DoAirBodyTweak`; the action graph passes them on and
    /// OffboardBodyTweakBlend tweaks once the stick has been let go, not within 0.1 s of landing. The
    /// hippie hurdle state is picked by `IsHippyHurdling`.
    #[test]
    fn the_c_stick_and_z_tweak_the_body_in_the_air_off_the_board() {
        use crate::graph::input::{Gates, Intentions};
        use crate::graph::write::Element;
        use crate::input::{Pad, btn};
        let pass = |n: &str| {
            Element::of(
                "behaviour",
                &[
                    ("name", "CreateMGIntentFromAGIntent"),
                    ("MGIntent", n),
                    ("AGIntent", n),
                ],
                vec![],
            )
        };
        let action = st(
            "Action",
            &[],
            vec![
                pass("OB_AirBodyTweakX"),
                pass("OB_AirBodyTweakY"),
                pass("OB_DoAirBodyTweak"),
            ],
        );
        let hurdling = |m: &str| {
            Element::of(
                "condition",
                &[("name", "IsHippyHurdling"), ("mask", m)],
                vec![],
            )
        };
        let motion = st(
            "Motion",
            &[],
            vec![
                Element::of("behaviour", &[("name", "OffboardBodyTweakBlend")], vec![]),
                st("HurdleCyc", &[], vec![when(vec![hurdling("always")])]),
                st(
                    "Cyc",
                    &[],
                    vec![Element::of(
                        "expression",
                        &[("op", "not")],
                        vec![hurdling("always")],
                    )],
                ),
            ],
        );
        let bytes = compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap();
        let pack = Pack::parse(&bytes).unwrap();
        let mut input = Intentions::new(&pack, Vec::new());
        let mut r = Runner::new(Pack::parse(&bytes).unwrap(), None).unwrap();
        let mut rec = Vec::new();
        let [x, y, z] = ["OB_AirBodyTweakX", "OB_AirBodyTweakY", "OB_DoAirBodyTweak"]
            .map(|n| r.pack.find(n).unwrap());
        let mut tick = |r: &mut Runner, flick: [f32; 2], buttons: u32| {
            let pad = Pad {
                held: buttons,
                flick,
                ..Pad::default()
            };
            input.produce(
                &r.pack,
                &pad,
                Gates::default(),
                1.0 / 60.0,
                &mut r.s.ag,
                &mut rec,
            );
            r.update(None, 1.0 / 60.0);
        };
        r.s.world.time_to_land = Some(0.6);
        // The stick held from before: published, but no tweak until it is let go.
        tick(&mut r, [0.0, 0.8], 0);
        assert_eq!(r.s.mg.get(x), Some(0.0));
        assert_eq!(r.s.mg.get(y), Some(0.8));
        assert!(!r.s.mg.has(z) && !r.s.air_tweaking);
        tick(&mut r, [0.0, 0.0], 0);
        assert_eq!(r.s.mg.get(x), Some(0.0), "published centred too");
        assert!(!r.s.air_tweaking);
        tick(&mut r, [0.7, 0.0], 0);
        assert!(r.s.air_tweaking, "let go and pushed: tweaks");
        // About to land: not any more.
        r.s.world.time_to_land = Some(0.05);
        tick(&mut r, [0.7, 0.0], 0);
        assert!(!r.s.air_tweaking);
        // Z: centred.
        r.s.world.time_to_land = Some(0.6);
        tick(&mut r, [0.0, 0.0], btn::Z);
        assert!(r.s.mg.has(z) && r.s.air_tweaking);
        // The hippie hurdle picks its state.
        assert_eq!(r.motion_state(), "Cyc");
        r.s.world.hippy_hurdling = true;
        tick(&mut r, [0.0, 0.0], 0);
        assert_eq!(r.motion_state(), "HurdleCyc");
    }
}

/// The motion graph and the physics agree on wipeouts: a grab state shaped like Skate 3's (wiped out when
/// the board isn't in the air or its trucks or deck touch something) stays a grab while the physics has
/// the board in the air, and only a grind counts as the trucks touching.
mod wipeout_agreement {
    use super::*;
    use crate::graph::drive::Driver;
    use crate::input::Pad;
    use crate::math::Vec3;
    use crate::park::Park;
    use crate::skater::Skater;

    fn cond(name: &str, extra: &[(&str, &str)]) -> Element {
        let mut a = vec![("name", name)];
        a.extend_from_slice(extra);
        Element::of("condition", &a, vec![])
    }

    fn graphs() -> Vec<u8> {
        let action = st("Action", &[], vec![]);
        let air = || cond("PhysFilteredState", &[("state", "air")]);
        let wipe = Element::of(
            "transition",
            &[("target", "WipeOut"), ("priority", "high")],
            vec![Element::of(
                "expression",
                &[("op", "or")],
                vec![
                    Element::of("expression", &[("op", "not")], vec![air()]),
                    cond("TrucksOrDeckInContact", &[]),
                ],
            )],
        );
        let motion = st(
            "Motion",
            &[],
            vec![
                st(
                    "OnBoard",
                    &[],
                    vec![
                        st(
                            "InAir",
                            &[],
                            vec![
                                when(vec![cond(
                                    "PhysFilteredState",
                                    &[("state", "air"), ("mask", "always")],
                                )]),
                                st("Grab", &[], vec![wipe]),
                            ],
                        ),
                        st("Riding", &[], vec![]),
                    ],
                ),
                st("WipeOut", &[("active", "false")], vec![]),
            ],
        );
        compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    #[test]
    fn a_grab_in_the_air_is_not_a_wipeout() {
        let entry: &'static [u8] = alloc::boxed::Box::leak(graphs().into_boxed_slice());
        let lib = crate::motion::tests::lib();
        let db = crate::motion::MotionDb::parse(&lib).unwrap();
        let mut d = Driver::new(entry, None, &db, Vec::new()).unwrap();
        let park = Park {
            shapes: Vec::new(),
            half_size: 50.0,
            wall_height: 2.0,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
            rails: Vec::new(),
        };
        let mut s = Skater::spawn(&park);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.2;
        s.states.time = 0.2;
        s.pos.y = 1.0;
        let grab = Pad {
            trig_r: 1.0,
            ..Pad::default()
        };
        for _ in 0..30 {
            let w = d.world(&s, Some(0.4), 0.0);
            assert!(
                !w.trucks_or_deck,
                "wheels or nothing touching is not trucks or deck"
            );
            d.tick(&db, &grab, w, Default::default(), 1.0 / 60.0, None);
            let state = d.runner.motion_state();
            assert_eq!(state, "OnBoard/InAir/Grab", "the physics is in the air");
        }
        // On a rail the trucks touch.
        s.force_state(crate::skater::State::GrindFiftyFifty);
        assert!(d.world(&s, None, 0.0).trucks_or_deck);
        s.force_state(crate::skater::State::PhysicsGround);
        assert!(!d.world(&s, None, 0.0).trucks_or_deck);
    }
}

/// The riding behaviours set their tree parameters: the carve lean follows the turn, the body tilt
/// stays flat on flat ground, the crouch height comes from the trigger, and a tree played with a
/// parameter nothing sets is reported once.
mod riding_parameters {
    use super::*;
    use crate::graph::host::Runner;
    use crate::motion::MotionDb;

    fn b(name: &str, extra: &[(&str, &str)]) -> Element {
        let mut a = vec![("name", name)];
        a.extend_from_slice(extra);
        Element::of("behaviour", &a, vec![])
    }

    fn graphs() -> Vec<u8> {
        let action = st(
            "Action",
            &[],
            vec![b(
                "CreateMGIntentFromAGIntent",
                &[("MGIntent", "Crouch"), ("AGIntent", "Crouch")],
            )],
        );
        let motion = st(
            "Motion",
            &[],
            vec![st(
                "Riding",
                &[],
                vec![
                    b("SettingBodyTilt", &[("tilt_x", "X")]),
                    b("SetDistComToBoard", &[("attribute", "Y")]),
                    b("Crouching", &[("crouchName", "Y")]),
                    b("SetTurning", &[("angleName", "LEAN")]),
                    st(
                        "Idle",
                        &[],
                        vec![
                            b("ApplyingBodyTilt", &[]),
                            b("PlayAnimation", &[("anim", "PB_LEAN")]),
                            b("PlayAnimation", &[("anim", "BS_XY")]),
                        ],
                    ),
                ],
            )],
        );
        compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    fn param(r: &Runner, db: &MotionDb, name: &str) -> Option<f32> {
        let n = db.name_id(name)?;
        r.s.player
            .params
            .iter()
            .find(|p| p.name == n)
            .map(|p| p.value)
    }

    #[test]
    fn riding_behaviours_set_their_tree_parameters() {
        let bytes = graphs();
        let lib = crate::motion::tests::lib();
        let db = MotionDb::parse(&lib).unwrap();
        let mut r = Runner::new(Pack::parse(&bytes).unwrap(), Some(&db)).unwrap();
        let dt = 1.0 / 60.0;
        r.s.world.speed = 5.0;
        r.s.world.forward_speed = 5.0;
        r.s.world.ground_speed = 5.0;
        for _ in 0..60 {
            r.update(Some(&db), dt);
        }
        assert_eq!(r.motion_state(), "Riding/Idle");
        assert_eq!(param(&r, &db, "LEAN"), Some(0.0), "riding straight");
        assert_eq!(param(&r, &db, "X"), Some(0.0), "flat ground");
        let standing = param(&r, &db, "Y").unwrap();
        // A held carve leans; the trigger crouches.
        r.s.world.turn = 1.0;
        r.s.world.lean = 0.3;
        let crouch = r.pack.find("Crouch").unwrap();
        for _ in 0..180 {
            r.s.ag.set(crouch, 1.0);
            r.update(Some(&db), dt);
        }
        let lean = param(&r, &db, "LEAN").unwrap();
        assert!(lean.abs() > 0.2, "{lean}");
        let crouched = param(&r, &db, "Y").unwrap();
        assert!(crouched < standing - 0.2, "{standing} -> {crouched}");
        // The blend space's X and Y are set; nothing set LEAN's sibling parameters in PB_LEAN's
        // children (clips), so only the blend space's own parameters count: none unset.
        assert!(
            r.s.player.unset.is_empty(),
            "{:?}",
            r.s.player
                .unset
                .iter()
                .map(|u| db.name(u.1))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_manual_out_timer_starts_when_the_manual_ends() {
        let has = Element::of(
            "condition",
            &[
                ("name", "HasIntent"),
                ("intent", "Manual"),
                ("mask", "always"),
            ],
            vec![],
        );
        let motion = st(
            "Motion",
            &[],
            vec![st(
                "Riding",
                &[],
                vec![
                    b("UpdateManualOutTimer", &[]),
                    st(
                        "Manual",
                        &[],
                        vec![
                            when(vec![has]),
                            b("IsManualing", &[]),
                            b("SetManualOutTimer", &[("length", "0.1")]),
                        ],
                    ),
                    st("Idle", &[], vec![]),
                ],
            )],
        );
        let action = st(
            "Action",
            &[],
            vec![b(
                "CreateMGIntentFromAGIntent",
                &[("MGIntent", "Manual"), ("AGIntent", "Manual")],
            )],
        );
        let bytes = compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap();
        let mut r = Runner::new(Pack::parse(&bytes).unwrap(), None).unwrap();
        let manual = r.pack.find("Manual").unwrap();
        let dt = 1.0 / 60.0;
        for _ in 0..30 {
            r.s.ag.set(manual, -1.0);
            r.update(None, dt);
        }
        assert_eq!(r.motion_state(), "Riding/Manual");
        assert_eq!(r.s.manual_out_timer, 0.0, "not while manualing");
        r.s.ag.clear();
        r.update(None, dt);
        r.update(None, dt);
        assert_eq!(r.motion_state(), "Riding/Idle");
        assert!(r.s.manual_out_timer > 0.05, "{}", r.s.manual_out_timer);
        for _ in 0..8 {
            r.update(None, dt);
        }
        assert_eq!(r.s.manual_out_timer, 0.0);
    }

    #[test]
    fn a_tree_with_a_parameter_nothing_sets_is_reported_once() {
        use crate::graph::ops::TransitionDef;
        use crate::graph::playback::{Player, Tree};
        use crate::motion::tree::Playback;
        let lib = crate::motion::tests::lib();
        let db = MotionDb::parse(&lib).unwrap();
        let mut p = Player::default();
        let tree = |name: &str| Tree {
            play: Playback::build(&db, name, &[]).unwrap(),
            mirror: false,
            turned: false,
            name: 7,
        };
        let cut = TransitionDef {
            kind: 1,
            seconds: 0.0,
            under: false,
            matching: 0,
            channels_from_weights: false,
        };
        p.play(&db, tree("PB_LEAN"), 1.0, cut);
        p.advance(&db, 1.0 / 60.0);
        assert_eq!(p.unset, [(7, db.name_id("LEAN").unwrap())]);
        p.unset.clear();
        p.play(&db, tree("PB_LEAN"), 1.0, cut);
        p.advance(&db, 1.0 / 60.0);
        assert!(p.unset.is_empty(), "once per parameter");
        // Set before the tick ends: not reported.
        p.play(&db, tree("BS_XY"), 1.0, cut);
        p.set_param(db.name_id("X").unwrap(), 0.5, false);
        p.set_param(db.name_id("Y").unwrap(), 0.5, false);
        p.advance(&db, 1.0 / 60.0);
        assert!(p.unset.is_empty());
    }
}

/// The whole game on a synthetic motion pack (made-up clips, a made-up pair of graphs shaped like the
/// manual part of Skate 3's): the pad through the action graph, the motion graph and the physics.
mod in_game {
    use super::*;
    use crate::game::Game;
    use crate::input::{PadRaw, flag as pad_flag};
    use crate::pack::Kind;

    fn has(intent: &str, cmp: &str, v: &str, mask: &str) -> Element {
        Element::of(
            "condition",
            &[
                ("name", "HasIntent"),
                ("intent", intent),
                (cmp, v),
                ("mask", mask),
            ],
            vec![],
        )
    }

    /// Action graph: the C-stick's `Manual` and how long it has been held, as motion intents. Motion
    /// graph: `OnBoard/Riding/Turning/Manual/TailManual` once held over 0.2 s, while `Manual` < 0.
    pub fn manual_graphs() -> Vec<u8> {
        let action = st(
            "Action",
            &[],
            vec![
                Element::of(
                    "behaviour",
                    &[
                        ("name", "CreateMGIntentFromAGIntent"),
                        ("MGIntent", "Manual"),
                        ("AGIntent", "Manual"),
                    ],
                    vec![],
                ),
                Element::of(
                    "behaviour",
                    &[
                        ("name", "CreateMGTimeIntentFromAGIntent"),
                        ("MGIntent", "ManualEngageTime"),
                        ("AGIntent", "Manual"),
                    ],
                    vec![],
                ),
            ],
        );
        let manual = st(
            "Manual",
            &[],
            vec![
                when(vec![has("ManualEngageTime", "greater", "0.2", "precond")]),
                st(
                    "TailManual",
                    &[],
                    vec![when(vec![has("Manual", "less", "0.0", "always")])],
                ),
            ],
        );
        let motion = st(
            "Motion",
            &[],
            vec![st(
                "OnBoard",
                &[],
                vec![st(
                    "Riding",
                    &[],
                    vec![st("Turning", &[], vec![manual, st("Idle", &[], vec![])])],
                )],
            )],
        );
        crate::graph::write::compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    fn game() -> Game {
        let bytes = crate::motion::tests::pack(&[
            ("motion/db", Kind::MotionDb, crate::motion::tests::lib()),
            ("motion/graph", Kind::StateGraph, manual_graphs()),
        ]);
        let mut g = Game::new();
        g.load_motion(alloc::boxed::Box::leak(bytes.into_boxed_slice()))
            .unwrap();
        assert!(g.graph.is_some());
        g
    }

    #[test]
    fn a_held_manual_lasts_until_the_stick_is_let_go() {
        let mut g = game();
        g.skater.speed = 4.0;
        let light_back = PadRaw {
            flags: pad_flag::CONNECTED,
            cstick_y: -47,
            ..PadRaw::default()
        };
        let mut manual = 0;
        for _ in 0..(60 * 4) {
            g.update(&light_back, 1.0 / 60.0);
            manual += g.skater.manual.is_some() as u32;
        }
        let state = g.graph.as_ref().unwrap().runner.motion_state();
        assert!(state.ends_with("Manual/TailManual"), "{state}");
        assert!(manual >= 3 * 60, "manualled for {manual} ticks");
        g.update(
            &PadRaw {
                flags: pad_flag::CONNECTED,
                ..PadRaw::default()
            },
            1.0 / 60.0,
        );
        g.update(&PadRaw::default(), 1.0 / 60.0);
        assert!(g.skater.manual.is_none());
    }
}

/// The manual's way out through `PhysicsWantsManualExit`, on a synthetic pair of graphs shaped like
/// Skate 3's tail manual: `Cycle` (entered only without `ManualBrake`, held while the physics doesn't
/// want out) and `Brake` (only from `Cycle`, held while dragging and the physics doesn't want out).
mod manual_exit {
    use super::*;
    use crate::game::Game;
    use crate::input::{PadRaw, flag as pad_flag};
    use crate::pack::Kind;

    fn intent(name: &str, extra: &[(&str, &str)]) -> Element {
        let mut a = vec![("name", "HasIntent"), ("intent", name)];
        a.extend_from_slice(extra);
        Element::of("condition", &a, vec![])
    }

    fn not(e: Element) -> Element {
        Element::of("expression", &[("op", "not")], vec![e])
    }

    fn wants_exit() -> Element {
        Element::of(
            "condition",
            &[("name", "PhysicsWantsManualExit"), ("mask", "always")],
            vec![],
        )
    }

    fn graphs() -> Vec<u8> {
        let mg = |mg: &str, ag: &str, time: bool| {
            Element::of(
                "behaviour",
                &[
                    (
                        "name",
                        if time {
                            "CreateMGTimeIntentFromAGIntent"
                        } else {
                            "CreateMGIntentFromAGIntent"
                        },
                    ),
                    ("MGIntent", mg),
                    ("AGIntent", ag),
                ],
                vec![],
            )
        };
        let action = st(
            "Action",
            &[],
            vec![
                mg("Manual", "Manual", false),
                mg("ManualEngageTime", "Manual", true),
                mg("ManualBrake", "ManualBrake", false),
            ],
        );
        let cycle = st(
            "Cycle",
            &[],
            vec![
                when(vec![
                    not(intent("ManualBrake", &[("mask", "precond")])),
                    not(wants_exit()),
                ]),
                go("Brake", None, vec![intent("ManualBrake", &[])]),
            ],
        );
        let brake = st(
            "Brake",
            &[("active", "false"), ("interruptable", "false")],
            vec![when(vec![
                intent("ManualBrake", &[("mask", "always")]),
                not(wants_exit()),
            ])],
        );
        let manual = st(
            "Manual",
            &[],
            vec![
                when(vec![intent(
                    "ManualEngageTime",
                    &[("greater", "0.2"), ("mask", "precond")],
                )]),
                st(
                    "TailManual",
                    &[],
                    vec![
                        when(vec![intent(
                            "Manual",
                            &[("less", "0.0"), ("mask", "always")],
                        )]),
                        cycle,
                        brake,
                    ],
                ),
            ],
        );
        let motion = st(
            "Motion",
            &[],
            vec![st(
                "OnBoard",
                &[],
                vec![st(
                    "Riding",
                    &[],
                    vec![st("Turning", &[], vec![manual, st("Idle", &[], vec![])])],
                )],
            )],
        );
        compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    fn game(corrective: bool) -> Game {
        game_at(corrective, 5.0)
    }

    fn game_at(corrective: bool, speed: f32) -> Game {
        let bytes = crate::motion::tests::pack(&[
            ("motion/db", Kind::MotionDb, crate::motion::tests::lib()),
            ("motion/graph", Kind::StateGraph, graphs()),
        ]);
        let mut g = Game::new();
        g.load_motion(alloc::boxed::Box::leak(bytes.into_boxed_slice()))
            .unwrap();
        g.skater.tuning.manual_corrective = corrective;
        g.skater.speed = speed;
        g
    }

    fn cstick(y: i8) -> PadRaw {
        PadRaw {
            flags: pad_flag::CONNECTED,
            cstick_y: y,
            ..PadRaw::default()
        }
    }

    /// Light back into the manual, then all the way back: the tail drags.
    fn drag(g: &mut Game) -> (u32, String) {
        for _ in 0..60 {
            g.update(&cstick(-47), 1.0 / 60.0);
        }
        let state = g.graph.as_ref().unwrap().runner.motion_state();
        assert!(state.ends_with("TailManual/Cycle"), "{state}");
        let mut manual = 0;
        for _ in 0..30 {
            g.update(&cstick(-127), 1.0 / 60.0);
            manual += g.skater.manual.is_some() as u32;
        }
        (manual, g.graph.as_ref().unwrap().runner.motion_state())
    }

    #[test]
    fn with_the_corrective_force_the_tail_drags_in_the_manual() {
        let mut g = game(true);
        let (manual, state) = drag(&mut g);
        assert!(state.ends_with("TailManual/Brake"), "{state}");
        assert_eq!(manual, 30);
        assert!(g.skater.manual.is_some_and(|m| m.brake > 0.5));
    }

    #[test]
    fn without_it_dragging_the_tail_rolling_backwards_ends_the_manual() {
        // Skate 3's balance is positive on the tail: the drag opposes the travel rolling tail first
        // (SK8 manual/controller.rs `apply_correction`). flatground used to end it rolling forwards.
        let mut g = game_at(false, -5.0);
        let (manual, state) = drag(&mut g);
        assert!(!state.contains("Manual"), "{state}");
        assert!(manual <= 3, "manualled {manual} ticks into the drag");
        assert!(g.skater.manual.is_none() && !g.skater.manual_exit);
    }

    #[test]
    fn without_it_dragging_the_tail_rolling_forwards_stays_in_the_manual() {
        let mut g = game(false);
        let (manual, state) = drag(&mut g);
        assert!(state.ends_with("TailManual/Brake"), "{state}");
        assert_eq!(manual, 30);
    }
}

/// Phase 4: channels, the typed operations, string lookups, gesture order, air tricks.
mod phase4 {
    use super::*;
    use crate::graph::ops::{Op, TransitionDef};
    use crate::graph::playback::{ChannelSettings, Fade, Player, PoseOp, PoseProgram, Tree};
    use crate::motion::MotionDb;
    use crate::motion::tree::Playback;

    fn tree(db: &MotionDb, name: &str) -> Tree {
        let node = db.find(name).unwrap();
        Tree {
            play: Playback::build_node(db, node, &[], &mut Vec::new()).unwrap(),
            mirror: false,
            turned: false,
            name: NONE,
        }
    }

    fn blend(kind: u8, first: bool) -> TransitionDef {
        TransitionDef {
            kind,
            seconds: 0.2,
            under: false,
            matching: 0,
            channels_from_weights: first,
        }
    }

    #[test]
    fn channel_blends_are_their_own_pose_operation() {
        let bytes = crate::motion::tests::lib();
        let db = MotionDb::parse(&bytes).unwrap();
        let mut p = Player::default();
        p.play(&db, tree(&db, "LOOP_A"), 1.0, blend(1, false));
        p.play(&db, tree(&db, "LOOP_B"), 1.0, blend(3, true));
        p.advance(&db, 0.1);
        let mut prog = PoseProgram::default();
        p.program(&mut prog, &mut Vec::new());
        assert_eq!(prog.ops.len(), 3, "{:?}", prog.ops);
        assert!(
            matches!(prog.ops[2], PoseOp::Channel { first: true, weight } if weight > 0.0 && weight < 1.0)
        );
        let shares: f32 = prog.shares().iter().map(|s| s.1).sum();
        assert!((shares - 1.0).abs() < 1e-4);
        // A plain blend stays a blend.
        p.play(&db, tree(&db, "SHORT"), 1.0, blend(2, false));
        p.program(&mut prog, &mut Vec::new());
        assert!(matches!(prog.ops.last(), Some(PoseOp::Blend(_))));
    }

    #[test]
    fn a_channel_fades_in_over_the_animation_and_out_when_ended() {
        let bytes = crate::motion::tests::lib();
        let db = MotionDb::parse(&bytes).unwrap();
        let mut p = Player::default();
        p.play(&db, tree(&db, "LOOP_A"), 1.0, blend(1, false));
        let settings = ChannelSettings {
            priority: 0,
            keep_alive: true,
            speed: 1.0,
            blend_in: 0.3,
            hold_in: false,
            blend_out: 0.3,
            hold_out: false,
        };
        p.transition_channel(
            &db,
            7,
            tree(&db, "LOOP_B"),
            settings,
            blend(2, false),
            true,
            true,
        );
        for _ in 0..30 {
            p.advance(&db, 1.0 / 60.0);
        }
        assert!((p.channel_weight(7) - 1.0).abs() < 1e-4);
        let mut prog = PoseProgram::default();
        p.program(&mut prog, &mut Vec::new());
        assert!(matches!(
            prog.ops.last(),
            Some(PoseOp::Channel { first: false, .. })
        ));
        p.end_channel(7);
        for _ in 0..40 {
            p.advance(&db, 1.0 / 60.0);
        }
        assert!(!p.has_channel(7), "faded out and gone");
        // Fades use the time before the step, as Skate 3's.
        let mut f = Fade::new(settings);
        f.advance(0.15, 1.0);
        assert_eq!(f.weight, 0.0);
        f.advance(0.15, 1.0);
        assert!((f.weight - 0.5).abs() < 1e-4);
    }

    #[test]
    fn typed_operations_read_back_the_same() {
        let action = st(
            "Action",
            &[],
            vec![Element::of(
                "behaviour",
                &[
                    ("name", "CreateMGIntentFromAGIntent"),
                    ("MGIntent", "Manual"),
                    ("AGIntent", "Manual"),
                    ("scale", "2.0"),
                ],
                vec![],
            )],
        );
        let motion = st(
            "Motion",
            &[],
            vec![
                st(
                    "A",
                    &[],
                    vec![
                        when(vec![Element::of(
                            "condition",
                            &[("name", "HasIntent"), ("intent", "Manual"), ("less", "0.0")],
                            vec![],
                        )]),
                        Element::of(
                            "behaviour",
                            &[("name", "PushCycle"), ("newPushName", "RightPush")],
                            vec![],
                        ),
                        Element::of("behaviour", &[("name", "FakieHeadChannel")], vec![]),
                    ],
                ),
                st("B", &[], vec![]),
            ],
        );
        let bytes = compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap();
        let pack = Pack::parse(&bytes).unwrap();
        let typed: Vec<Vec<Op>> = pack
            .graphs
            .iter()
            .map(|g| crate::graph::ops::parse(&pack, g, None))
            .collect();
        let code = crate::graph::opcode::encode(bytes.len(), &typed);
        assert_eq!(
            crate::graph::opcode::decode(&code, bytes.len()).as_ref(),
            Some(&typed)
        );
        // Made for another graph entry: not used.
        assert!(crate::graph::opcode::decode(&code, bytes.len() + 1).is_none());
        assert!(crate::graph::opcode::decode(&code[..code.len() - 1], bytes.len()).is_none());
        let r =
            crate::graph::host::Runner::with_ops(Pack::parse(&bytes).unwrap(), None, Some(typed));
        assert!(r.is_some());
    }

    #[test]
    fn strings_are_found_by_binary_search() {
        let bytes = compile(&[("G", &riding())]).unwrap();
        let p = Pack::parse(&bytes).unwrap();
        for i in 0..p.strings() as u16 {
            assert_eq!(p.find(p.str(i)), Some(i), "{}", p.str(i));
        }
        assert_eq!(p.find("NotThere"), None);
    }

    #[test]
    fn simultaneous_gestures_go_by_skate_3s_hash_bucket() {
        use crate::graph::host::gesture_bucket;
        // The engine's text encoding folds case; the bucket is the words' sum modulo 43.
        assert_eq!(gesture_bucket("Kickflip"), gesture_bucket("KICKFLIP"));
        assert!(gesture_bucket("360InwardHeelflip") < 43);
        // "A": one digit (11) at the first weight, 38^5.
        assert_eq!(gesture_bucket("A"), (11 * 79_235_168u32) % 43);
    }

    #[test]
    fn air_tricks_by_their_scoring_names() {
        use crate::gesture::Trick;
        use crate::graph::drive::{air_flick, air_trick_state};
        assert_eq!(air_flick("Kickflip3").0.trick, Trick::Kickflip);
        assert_eq!(air_flick("Kickflip3").1, 3);
        assert_eq!(air_flick("N_Heelflip").0.trick, Trick::Heelflip);
        assert_eq!(air_flick("L_F_Heelflip").0.trick, Trick::Heelflip);
        assert_eq!(air_flick("U_Kickflip").0.trick, Trick::Kickflip);
        assert_eq!(air_flick("FS_Varial").0.trick, Trick::VarialKickflip);
        assert!(air_trick_state(
            "OnBoard/Trick/TailTrick/Kickflip/LeftGround/Default"
        ));
        assert!(!air_trick_state(
            "OnBoard/Trick/TailTrick/Kickflip/Takeoff/FromAntic"
        ));
        assert!(air_trick_state("OnBoard/InAir/Lateflip/LateKickflip"));
        assert!(!air_trick_state("OnBoard/InAir/InAirStatic/Air/Default"));
    }
}

/// BoardAdjust (SK8 action_board_adjust.rs) publishes the C-stick to the motion graph every update, the
/// angle filtered, negated when mirrored and held at the +-pi seam; SetDeckPitchAndYaw hands the posed
/// deck's yaw and pitch to its trees.
mod board_adjust {
    use super::*;
    use crate::graph::host::{AdjustSeam, Runner, deck_angles};
    use crate::graph::ops::{Beh, Op};
    use crate::math::{PI, v3};

    fn b(name: &str, extra: &[(&str, &str)]) -> Element {
        let mut a = vec![("name", name)];
        a.extend_from_slice(extra);
        Element::of("behaviour", &a, vec![])
    }

    fn graphs(extra: &[(&str, &str)]) -> Vec<u8> {
        let mut a = vec![
            ("MGIntentMag", "RightBoardAdjustMag"),
            ("MGIntentAngle", "RightBoardAdjustAngle"),
        ];
        a.extend_from_slice(extra);
        // The pad's intents are graph strings in Skate 3's graphs (conditions read them).
        let pad = |n: &'static str| {
            Element::of(
                "condition",
                &[("name", "HasIntent"), ("intent", n), ("mask", "always")],
                vec![],
            )
        };
        let action = st(
            "Action",
            &[],
            vec![
                b("BoardAdjust", &a),
                st(
                    "Stick",
                    &[],
                    vec![when(vec![pad("BoardAdjustMag"), pad("BoardAdjustAngle")])],
                ),
                st("Rest", &[], vec![]),
            ],
        );
        let adjusting = Element::of(
            "condition",
            &[
                ("name", "HasIntent"),
                ("intent", "RightBoardAdjustMag"),
                ("mask", "always"),
            ],
            vec![],
        );
        let motion = st(
            "Motion",
            &[],
            vec![
                st(
                    "Adjust",
                    &[],
                    vec![
                        when(vec![adjusting]),
                        b(
                            "SetDeckPitchAndYaw",
                            &[("skateyaw", "X"), ("skatepitch", "Y")],
                        ),
                    ],
                ),
                st("Idle", &[], vec![]),
            ],
        );
        compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    #[test]
    fn the_seam_holds_the_side_the_angle_crossed_from() {
        let mut s = AdjustSeam::default();
        let step = |s: &mut AdjustSeam, a: f32| s.update(Some(0.9), Some(a), 0, true, false);
        assert_eq!(step(&mut s, 3.0), Some((0.9, 3.0)));
        // Round past straight down (+pi to -pi): held at +pi while the raw angle stays across.
        assert_eq!(step(&mut s, -3.0), Some((0.9, PI)));
        assert_eq!(step(&mut s, -2.0), Some((0.9, PI)));
        // Back across the seam (from the negative side): free again.
        assert_eq!(step(&mut s, 3.0), Some((0.9, 3.0)));
        // Through zero (not beyond pi/2) is no seam.
        assert_eq!(step(&mut s, 0.5), Some((0.9, 0.5)));
        assert_eq!(step(&mut s, -0.5), Some((0.9, -0.5)));
        // The other way: held at -pi.
        assert_eq!(step(&mut s, -3.0), Some((0.9, -3.0)));
        assert_eq!(step(&mut s, 3.0), Some((0.9, -PI)));
        // An input gone: nothing published and the state reset.
        assert_eq!(s.update(None, Some(1.0), 0, true, false), None);
        assert_eq!(s, AdjustSeam::default());
        // A present zero is valid.
        assert_eq!(step(&mut s, 0.0), Some((0.9, 0.0)));
        // Mirrored: negated (only with negateOnMirror); the angle filter first.
        let mut m = AdjustSeam::default();
        assert_eq!(
            m.update(Some(1.0), Some(1.0), 0, true, true),
            Some((1.0, -1.0))
        );
        let mut m = AdjustSeam::default();
        assert_eq!(
            m.update(Some(1.0), Some(1.0), 0, false, true),
            Some((1.0, 1.0))
        );
        let mut m = AdjustSeam::default();
        let (_, a) = m.update(Some(1.0), Some(1.0), 6, false, false).unwrap();
        assert!((a - (1.0 - PI / 2.0)).abs() < 1e-6, "angleRot90: {a}");
    }

    #[test]
    fn board_adjust_publishes_the_c_stick_to_the_motion_graph() {
        let bytes = graphs(&[]);
        let lib = crate::motion::tests::lib();
        let db = crate::motion::MotionDb::parse(&lib).unwrap();
        let mut r = Runner::new(Pack::parse(&bytes).unwrap(), Some(&db)).unwrap();
        let dt = 1.0 / 60.0;
        let (mag, angle) = (
            r.pack.find("BoardAdjustMag").unwrap(),
            r.pack.find("BoardAdjustAngle").unwrap(),
        );
        let (mg_mag, mg_angle) = (
            r.pack.find("RightBoardAdjustMag").unwrap(),
            r.pack.find("RightBoardAdjustAngle").unwrap(),
        );
        r.update(Some(&db), dt);
        assert_eq!(r.motion_state(), "Idle");
        assert!(!r.board_adjusting());
        r.s.world.deck_yaw_pitch = [0.25, -0.5];
        for a in [1.0, 2.5, -2.5] {
            r.s.ag.set(mag, 0.9);
            r.s.ag.set(angle, a);
            r.update(Some(&db), dt);
        }
        assert_eq!(r.s.mg.get(mg_mag), Some(0.9));
        // 2.5 then -2.5 crossed the seam: held at +pi.
        assert_eq!(r.s.mg.get(mg_angle), Some(PI));
        assert!(r.board_adjusting());
        r.update(Some(&db), dt);
        assert_eq!(r.motion_state(), "Adjust");
        // SetDeckPitchAndYaw: the deck's yaw and pitch on entering.
        let param = |r: &Runner, n: &str| {
            let n = db.name_id(n).unwrap();
            r.s.player
                .params
                .iter()
                .find(|p| p.name == n)
                .map(|p| p.value)
        };
        assert_eq!(param(&r, "X"), Some(0.25));
        assert_eq!(param(&r, "Y"), Some(-0.5));
        // Let go: both removed, the motion graph leaves the adjust.
        r.s.ag.clear();
        r.update(Some(&db), dt);
        assert_eq!(r.s.mg.get(mg_mag), None);
        assert_eq!(r.s.mg.get(mg_angle), None);
        assert!(!r.board_adjusting());
        r.update(Some(&db), dt);
        assert_eq!(r.motion_state(), "Idle");
        // Mirrored (the seam reset by letting go): negated.
        r.s.world.mirrored = true;
        r.s.ag.set(mag, 1.0);
        r.s.ag.set(angle, 1.0);
        r.update(Some(&db), dt);
        assert_eq!(r.s.mg.get(mg_angle), Some(-1.0));
    }

    #[test]
    fn board_adjust_reads_its_filter_and_mirror_flag_and_survives_the_importer() {
        let adjust = |o: &Op| match o {
            Op::Beh(Beh::BoardAdjust {
                angle_filter,
                negate_on_mirror,
                ..
            }) => Some((*angle_filter, *negate_on_mirror)),
            _ => None,
        };
        let bytes = graphs(&[("angleFilter", "angleRot90"), ("negateOnMirror", "false")]);
        let pack = Pack::parse(&bytes).unwrap();
        let typed: Vec<Vec<Op>> = pack
            .graphs
            .iter()
            .map(|g| crate::graph::ops::parse(&pack, g, None))
            .collect();
        assert_eq!(typed.iter().flatten().find_map(adjust), Some((6, false)));
        let code = crate::graph::opcode::encode(bytes.len(), &typed);
        assert_eq!(
            crate::graph::opcode::decode(&code, bytes.len()).as_ref(),
            Some(&typed)
        );
        // Default: no filter, negated when mirrored (Skate 3's constructor default).
        let bytes = graphs(&[]);
        let pack = Pack::parse(&bytes).unwrap();
        let ops: Vec<Op> = pack
            .graphs
            .iter()
            .flat_map(|g| crate::graph::ops::parse(&pack, g, None))
            .collect();
        assert_eq!(ops.iter().find_map(adjust), Some((0, true)));
    }

    #[test]
    fn deck_angles_keep_skate_3s_quadrants_and_pitch_projection() {
        // SK8 deck_angles_tests.rs.
        for (x, z, want) in [
            (1.0, 1.0, PI / 4.0),
            (-1.0, 1.0, -PI / 4.0),
            (1.0, -1.0, 3.0 * PI / 4.0),
            (-1.0, -1.0, -3.0 * PI / 4.0),
            (0.0, 0.0, PI / 2.0),
            (-0.0, -0.0, -PI / 2.0),
        ] {
            let [yaw, pitch] = deck_angles(v3(x, 0.5, z));
            assert!((yaw - want).abs() < 1e-5, "({x}, {z}): {yaw}");
            assert_eq!(pitch, 0.5);
        }
    }
}

/// The foot plant's conditions and FootPlantAbsorb, on a synthetic motion graph shaped like Skate 3's
/// one-foot air: `Footplant` (entered while the free foot will meet something in 0.1 to 0.5 s) with its
/// `Into` (held) and `Absorb` (while the physics swings over the planted foot, handing the swing's time
/// to the absorb clips).
mod foot_plant {
    use super::*;
    use crate::graph::host::Runner;
    use crate::graph::ops::{Beh, Cond, Op};

    fn c(name: &str) -> Element {
        Element::of("condition", &[("name", name), ("mask", "always")], vec![])
    }

    fn graphs() -> Vec<u8> {
        let action = st("Action", &[], vec![]);
        let motion = st(
            "Motion",
            &[],
            vec![
                st(
                    "Absorb",
                    &[],
                    vec![
                        when(vec![c("IsFootPlanting")]),
                        Element::of("behaviour", &[("name", "FootPlantAbsorb")], vec![]),
                    ],
                ),
                st(
                    "Footplant",
                    &[],
                    vec![when(vec![c("ShouldPrepareOneFootAirForFootplant")])],
                ),
                st("Idle", &[], vec![]),
            ],
        );
        compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    #[test]
    fn the_foot_plant_states_follow_the_manager_and_absorb_its_swing() {
        let bytes = graphs();
        let lib = crate::motion::tests::lib();
        let db = crate::motion::MotionDb::parse(&lib).unwrap();
        let pack = Pack::parse(&bytes).unwrap();
        let mut typed: Vec<Vec<Op>> = pack
            .graphs
            .iter()
            .map(|g| crate::graph::ops::parse(&pack, g, Some(&db)))
            .collect();
        let ops: Vec<&Op> = typed.iter().flatten().collect();
        assert!(ops.contains(&&Op::Cond(Cond::FootPlanting)));
        assert!(ops.contains(&&Op::Cond(Cond::PrepareFootplant)));
        assert!(ops.contains(&&Op::Beh(Beh::FootPlantAbsorb(None))));
        // The test library has no `absorblength`: hand the behaviour one of its names.
        let vel = db.name_id("VEL").unwrap();
        for o in typed.iter_mut().flatten() {
            if let Op::Beh(Beh::FootPlantAbsorb(n)) = o {
                *n = Some(vel);
            }
        }
        let code = crate::graph::opcode::encode(bytes.len(), &typed);
        assert_eq!(
            crate::graph::opcode::decode(&code, bytes.len()).as_ref(),
            Some(&typed)
        );
        let mut r = Runner::with_ops(Pack::parse(&bytes).unwrap(), Some(&db), Some(typed)).unwrap();
        let dt = 1.0 / 60.0;
        r.update(Some(&db), dt);
        assert_eq!(r.motion_state(), "Idle");
        // The free foot meets the wall in 0.6 s: not yet; in 0.3 s: prepare.
        r.s.world.footplant_contact = 0.6;
        r.update(Some(&db), dt);
        assert_eq!(r.motion_state(), "Idle");
        r.s.world.footplant_contact = 0.3;
        r.update(Some(&db), dt);
        assert_eq!(r.motion_state(), "Footplant");
        // Planted, but the swing's time not set yet: not planting.
        r.s.world.footplant_contact = 0.01;
        r.s.world.foot_planting = true;
        r.update(Some(&db), dt);
        assert_eq!(r.motion_state(), "Idle");
        r.s.world.footplant_length = 0.18;
        r.update(Some(&db), dt);
        assert_eq!(r.motion_state(), "Absorb");
        let param = r.s.player.params.iter().find(|p| p.name == vel);
        assert_eq!(param.map(|p| p.value), Some(0.18));
    }
}

/// Dropping in and hand plants: their conditions and behaviours on a synthetic motion graph shaped like
/// Skate 3's (`Grinding/DroppingIn` entered with `IsDroppingIn`; `HandPlantGround` with the ground hand
/// plant and its antic's time, `HandPlantAir/FS` and `/BS` with the plant, scoring by the tweak).
mod transition {
    use super::*;
    use crate::graph::host::Runner;
    use crate::graph::ops::{Beh, Cond, Op};

    fn c(name: &str, attrs: &[(&str, &str)]) -> Element {
        let mut a = vec![("name", name), ("mask", "always")];
        a.extend_from_slice(attrs);
        Element::of("condition", &a, vec![])
    }

    fn graphs() -> Vec<u8> {
        let action = st("Action", &[], vec![]);
        let motion = st(
            "Motion",
            &[],
            vec![
                st("DroppingIn", &[], vec![when(vec![c("IsDroppingIn", &[])])]),
                st(
                    "FlatDrop",
                    &[],
                    vec![when(vec![c("IsDropInWithoutTransition", &[])])],
                ),
                st(
                    "AirFS",
                    &[],
                    vec![
                        when(vec![c(
                            "IsHandPlanting",
                            &[("state", "air"), ("dir", "FS")],
                        )]),
                        Element::of(
                            "behaviour",
                            &[
                                ("name", "ScoringHandPlants"),
                                ("intentX", "HandPlantTweakX"),
                                ("intentY", "HandPlantTweakY"),
                                ("handplantname", "FS_handplant"),
                                ("up", "FS_handplant_up"),
                                ("left", "FS_handplant_left"),
                                ("down", "FS_handplant_down"),
                                ("right", "FS_handplant_right"),
                            ],
                            vec![],
                        ),
                    ],
                ),
                st(
                    "AirBS",
                    &[],
                    vec![when(vec![c(
                        "IsHandPlanting",
                        &[("state", "air"), ("dir", "BS")],
                    )])],
                ),
                st(
                    "NewPos",
                    &[],
                    vec![when(vec![c("HasNewHandPlantPos", &[])])],
                ),
                st(
                    "Antic",
                    &[],
                    vec![
                        when(vec![
                            c("IsHandPlanting", &[("state", "ground")]),
                            c("ShouldPlayHandPlantAnim", &[("anim", "antic")]),
                        ]),
                        Element::of("behaviour", &[("name", "SetHandPlantAnticLength")], vec![]),
                    ],
                ),
                st("Idle", &[], vec![]),
            ],
        );
        compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    #[test]
    fn drop_in_and_hand_plant_states_follow_the_physics() {
        let bytes = graphs();
        let lib = crate::motion::tests::lib();
        let db = crate::motion::MotionDb::parse(&lib).unwrap();
        let pack = Pack::parse(&bytes).unwrap();
        let typed: Vec<Vec<Op>> = pack
            .graphs
            .iter()
            .map(|g| crate::graph::ops::parse(&pack, g, Some(&db)))
            .collect();
        let ops: Vec<&Op> = typed.iter().flatten().collect();
        for want in [
            Cond::DroppingIn,
            Cond::DropInFlat,
            Cond::NewHandPlantPos,
            Cond::HandPlanting { air: true, dir: 1 },
            Cond::HandPlanting { air: true, dir: 0 },
            Cond::HandPlanting { air: false, dir: 2 },
            Cond::PlayHandPlant(2),
        ] {
            assert!(ops.contains(&&Op::Cond(want)), "{want:?}");
        }
        assert!(
            ops.iter()
                .any(|o| matches!(o, Op::Beh(Beh::ScoringHandPlants(_))))
        );
        // The test library has no antic clips: the antic length is left alone.
        assert!(ops.contains(&&Op::Beh(Beh::HandPlantAntic {
            attr: None,
            normal: 0.0,
            late: 0.0
        })));
        let code = crate::graph::opcode::encode(bytes.len(), &typed);
        assert_eq!(
            crate::graph::opcode::decode(&code, bytes.len()).as_ref(),
            Some(&typed)
        );
        let mut r = Runner::with_ops(Pack::parse(&bytes).unwrap(), Some(&db), Some(typed)).unwrap();
        let dt = 1.0 / 60.0;
        let step = |r: &mut Runner, f: &dyn Fn(&mut crate::graph::host::World)| {
            r.s.world = crate::graph::host::World::default();
            f(&mut r.s.world);
            r.update(Some(&db), dt);
            r.motion_state()
        };
        assert_eq!(step(&mut r, &|_| {}), "Idle");
        assert_eq!(step(&mut r, &|w| w.dropping_in = true), "DroppingIn");
        assert_eq!(step(&mut r, &|w| w.drop_in_flat = true), "FlatDrop");
        // Found on the ground: the antic plays once the apex is nearer than its threshold (0.75 s).
        let found = |w: &mut crate::graph::host::World, time: f32| {
            w.handplant.found = true;
            w.handplant.time = time;
        };
        assert_eq!(step(&mut r, &|w| found(w, 1.0)), "Idle");
        assert_eq!(step(&mut r, &|w| found(w, 0.6)), "Antic");
        assert_eq!(
            step(&mut r, &|w| {
                found(w, 0.6);
                w.handplant.new_pos = true;
            }),
            "NewPos"
        );
        // In the plant: by its side.
        let air = |w: &mut crate::graph::host::World, fs: bool| {
            w.handplant.air = true;
            w.handplant.frontside = fs;
        };
        assert_eq!(step(&mut r, &|w| air(w, false)), "AirBS");
        assert_eq!(step(&mut r, &|w| air(w, true)), "AirFS");
        assert_eq!(r.s.handplant_score, pack.find("FS_handplant").unwrap());
    }
}

#[test]
fn a_clips_jump_height_override_counts_only_where_the_physics_mode_allows_it() {
    // SK8 attribute finalization: `JumpHeightOverride` raises the jump only with the mode's
    // `JumpHeightOverrideEnabled` (on for Easy and Normal, off for Hardcore).
    use crate::tuning::{Difficulty, Tuning};
    let mut t = Tuning::BUILT_IN;
    let low = t.jump_clamp_low;
    assert_eq!(super::drive::jump_strength(low, 0.6, &t), 0.6);
    t.set_difficulty(Difficulty::Hardcore);
    assert!(!t.jump_height_override);
    assert_eq!(super::drive::jump_strength(low, 0.6, &t), 0.0);
    t.set_difficulty(Difficulty::Easy);
    assert_eq!(super::drive::jump_strength(low, 0.6, &t), 0.6);
}

/// MatchCadence and MatchAirTime (SK8 motion_offboard/cadence.rs, match_air_time.rs): each captures the
/// walker's step cadence on entering and publishes it as `CadenceStartPercent` on every update (the
/// run and sprint mounts expose `OB_TRAJ` from it; without it they never got on).
mod cadence {
    use super::*;
    use crate::graph::host::Runner;
    use crate::motion::MotionDb;
    use crate::motion::write::{AttrIn, ClipIn, write};

    fn lib() -> Vec<u8> {
        let scalar = |name: &str| AttrIn {
            name: name.into(),
            kind: crate::motion::KIND_SCALAR,
            begin: -1.0,
            end: -1.0,
            words: vec![0.0f32.to_bits()],
        };
        let clip = ClipIn {
            name: "MOUNT".into(),
            fps: 30.0,
            frames: 30.0,
            base_speed: 1.0,
            andale_flags: 0,
            has_pose: true,
            attrs: vec![scalar("CADENCESTARTPERCENT"), scalar("ANIMTIME")],
        };
        write(&[clip], &[])
    }

    fn graphs(behaviour: &str) -> Vec<u8> {
        let action = st("Action", &[], vec![]);
        let motion = st(
            "Motion",
            &[],
            vec![st(
                "Mount",
                &[],
                vec![
                    Element::of("behaviour", &[("name", behaviour)], vec![]),
                    Element::of(
                        "behaviour",
                        &[("name", "PlayAnimation"), ("anim", "MOUNT")],
                        vec![],
                    ),
                ],
            )],
        );
        compile(&[
            (crate::graph::host::ACTION, &action),
            (crate::graph::host::MOTION, &motion),
        ])
        .unwrap()
    }

    #[test]
    fn the_mounts_start_from_the_cadence_captured_on_entering() {
        let lib = lib();
        let db = MotionDb::parse(&lib).unwrap();
        let n = db.name_id("CADENCESTARTPERCENT").unwrap();
        for behaviour in ["MatchCadence", "MatchAirTime"] {
            let bytes = graphs(behaviour);
            let mut r = Runner::new(Pack::parse(&bytes).unwrap(), Some(&db)).unwrap();
            let param = |r: &Runner| {
                r.s.player
                    .params
                    .iter()
                    .find(|p| p.name == n)
                    .map(|p| p.value)
            };
            r.s.world.cadence = 0.4;
            r.update(Some(&db), 1.0 / 60.0);
            assert_eq!(r.motion_state(), "Mount", "{behaviour}");
            assert_eq!(param(&r), Some(0.4), "{behaviour}: published");
            // The walker steps on: still the cadence captured on entering.
            r.s.world.cadence = 0.7;
            for _ in 0..5 {
                r.update(Some(&db), 1.0 / 60.0);
            }
            assert_eq!(param(&r), Some(0.4), "{behaviour}: captured");
        }
    }
}
