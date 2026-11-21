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
        // A and B together is the hippie jump: no push.
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
        assert!(!r.s.ag.has(r.pack.find("RightPush").unwrap()));
    }
}
