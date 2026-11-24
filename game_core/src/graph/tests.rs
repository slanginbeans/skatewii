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
    use crate::skater::{Mode, Skater};

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
        s.mode = Mode::Air { time: 0.2 };
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
        s.mode = Mode::Grind;
        assert!(d.world(&s, None, 0.0).trucks_or_deck);
        s.mode = Mode::Rolling;
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
