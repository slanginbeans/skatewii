//! The typed operations of both graphs, stored by the importer (`motion/graph/ops`) so loading reads them
//! back instead of parsing every operation's name and attributes and looking names up in the motion
//! library. Layout (big-endian): `"FGOP"`, u32 version, u32 length of the `motion/graph` entry they
//! were made from, u32 graph count, then per graph u32 operation count and the operations, each a tag
//! byte and its fields (u16 ids, f32, u8 flags; an option is a presence byte then the value).

use alloc::vec::Vec;

use super::ops::{
    Beh, Cmp, Cond, FilterDef, Flag, GrabScore, Group, Hook, Num, Op, ParamDef, PlayDef, PushOp,
    TransitionDef,
};
use crate::motion::Node;

/// The pack entry.
pub const ENTRY: &str = "motion/graph/ops";
pub const MAGIC: [u8; 4] = *b"FGOP";
pub const VERSION: u32 = 3;

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn b(&mut self, v: bool) {
        self.0.push(u8::from(v));
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn f(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_bits().to_be_bytes());
    }
    fn of(&mut self, v: Option<f32>) {
        self.b(v.is_some());
        if let Some(v) = v {
            self.f(v);
        }
    }
    fn o16(&mut self, v: Option<u16>) {
        self.b(v.is_some());
        if let Some(v) = v {
            self.u16(v);
        }
    }
    fn num(&mut self, n: Num) {
        self.u8(n.cmp as u8);
        self.f(n.threshold);
        self.b(n.abs);
    }
    fn node(&mut self, n: Option<Node>) {
        match n {
            None => self.u8(0),
            Some(Node::Clip(i)) => {
                self.u8(1);
                self.u16(i);
            }
            Some(Node::Tree(i)) => {
                self.u8(2);
                self.u16(i);
            }
            Some(Node::Missing(i)) => {
                self.u8(3);
                self.u16(i);
            }
        }
    }
    fn transition(&mut self, t: &TransitionDef) {
        self.u8(t.kind);
        self.f(t.seconds);
        self.b(t.under);
        self.u8(t.matching);
        self.b(t.channels_from_weights);
    }
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl R<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.at)?;
        self.at += 1;
        Some(v)
    }
    fn b(&mut self) -> Option<bool> {
        Some(self.u8()? != 0)
    }
    fn u16(&mut self) -> Option<u16> {
        let s = self.b.get(self.at..self.at + 2)?;
        self.at += 2;
        Some(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        let s = self.b.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.u32()?))
    }
    fn of(&mut self) -> Option<Option<f32>> {
        Some(if self.b()? { Some(self.f()?) } else { None })
    }
    fn o16(&mut self) -> Option<Option<u16>> {
        Some(if self.b()? { Some(self.u16()?) } else { None })
    }
    fn num(&mut self) -> Option<Num> {
        let cmp = match self.u8()? {
            0 => Cmp::None,
            1 => Cmp::Eq,
            2 => Cmp::Ne,
            3 => Cmp::Gt,
            4 => Cmp::Lt,
            5 => Cmp::Ge,
            6 => Cmp::Le,
            7 => Cmp::GtAbs,
            _ => return None,
        };
        Some(Num {
            cmp,
            threshold: self.f()?,
            abs: self.b()?,
        })
    }
    fn node(&mut self) -> Option<Option<Node>> {
        Some(match self.u8()? {
            0 => None,
            1 => Some(Node::Clip(self.u16()?)),
            2 => Some(Node::Tree(self.u16()?)),
            3 => Some(Node::Missing(self.u16()?)),
            _ => return None,
        })
    }
    fn transition(&mut self) -> Option<TransitionDef> {
        Some(TransitionDef {
            kind: self.u8()?,
            seconds: self.f()?,
            under: self.b()?,
            matching: self.u8()?,
            channels_from_weights: self.b()?,
        })
    }
    fn group(&mut self) -> Option<Group> {
        GROUPS.get(self.u8()? as usize).copied()
    }
}

const GROUPS: [Group; 7] = [
    Group::Square,
    Group::Nose,
    Group::Tail,
    Group::Nose90,
    Group::Tail90,
    Group::NoseN90,
    Group::TailN90,
];
const FLAGS: [Flag; 7] = [
    Flag::Trick,
    Flag::Manual,
    Flag::PowerSlide,
    Flag::Landing,
    Flag::Anticipating,
    Flag::SlideCandidate,
    Flag::NoPumping,
];

fn group_code(g: Group) -> u8 {
    GROUPS.iter().position(|&x| x == g).unwrap_or(0) as u8
}

/// Stores the operations of every graph (in pack order), for the `motion/graph` entry of `graph_len`
/// bytes.
pub fn encode(graph_len: usize, graphs: &[Vec<Op>]) -> Vec<u8> {
    let mut w = W(Vec::new());
    w.0.extend_from_slice(&MAGIC);
    w.u32(VERSION);
    w.u32(graph_len as u32);
    w.u32(graphs.len() as u32);
    for ops in graphs {
        w.u32(ops.len() as u32);
        for o in ops {
            op(&mut w, o);
        }
    }
    w.0
}

/// The operations of every graph, if `b` holds them for a `motion/graph` entry of `graph_len` bytes.
pub fn decode(b: &[u8], graph_len: usize) -> Option<Vec<Vec<Op>>> {
    let mut r = R { b, at: 0 };
    if b.get(..4)? != MAGIC || r.u32().is_none() {
        return None;
    }
    r.at = 4;
    if r.u32()? != VERSION || r.u32()? as usize != graph_len {
        return None;
    }
    let graphs = r.u32()? as usize;
    let mut out = Vec::with_capacity(graphs.min(8));
    for _ in 0..graphs {
        let n = r.u32()? as usize;
        let mut ops = Vec::with_capacity(n.min(1 << 16));
        for _ in 0..n {
            ops.push(read_op(&mut r)?);
        }
        out.push(ops);
    }
    (r.at == b.len()).then_some(out)
}

fn op(w: &mut W, o: &Op) {
    match o {
        Op::Cond(c) => {
            w.u8(0);
            cond(w, c);
        }
        Op::Beh(b) => {
            w.u8(1);
            beh(w, b);
        }
        Op::Hook(h) => {
            w.u8(2);
            match *h {
                Hook::Override(t) => {
                    w.u8(0);
                    w.transition(&t);
                }
                Hook::GrabSlide { right } => {
                    w.u8(1);
                    w.b(right);
                }
                Hook::MongoPushToAntic => w.u8(2),
                Hook::Unsupported(s) => {
                    w.u8(3);
                    w.u16(s);
                }
            }
        }
    }
}

fn read_op(r: &mut R) -> Option<Op> {
    Some(match r.u8()? {
        0 => Op::Cond(read_cond(r)?),
        1 => Op::Beh(read_beh(r)?),
        2 => Op::Hook(match r.u8()? {
            0 => Hook::Override(r.transition()?),
            1 => Hook::GrabSlide { right: r.b()? },
            2 => Hook::MongoPushToAntic,
            3 => Hook::Unsupported(r.u16()?),
            _ => return None,
        }),
        _ => return None,
    })
}

/// Conditions without fields, by tag.
const PLAIN_CONDS: [Cond; 27] = [
    Cond::Mirrored,
    Cond::RidingFakie,
    Cond::RidingGoofy,
    Cond::RidingSwitch,
    Cond::DisablePushBrake,
    Cond::ManualOutTimer,
    Cond::WantsManualExit,
    Cond::WantsRunout,
    Cond::WantsWipeout,
    Cond::Dark,
    Cond::DarkCatchRequested,
    Cond::UnderflipRequested,
    Cond::Random,
    Cond::BreakOutOfPush,
    Cond::AllowedToTrick,
    Cond::OkToTrickOnStairs,
    Cond::Tricking,
    Cond::CanLandOnBoard,
    Cond::CrouchedForGrab,
    Cond::Blunting,
    Cond::TrucksOrDeck,
    Cond::Apex,
    Cond::AngleAdjustLess,
    Cond::BodyFlipping,
    Cond::DoneWipingOut,
    Cond::Fixed(false),
    Cond::Fixed(true),
];

fn cond(w: &mut W, c: &Cond) {
    if let Some(i) = PLAIN_CONDS.iter().position(|p| p == c) {
        w.u8(100 + i as u8);
        return;
    }
    match *c {
        Cond::Intent(s, n) => {
            w.u8(0);
            w.u16(s);
            w.num(n);
        }
        Cond::AgIntent(s, n) => {
            w.u8(1);
            w.u16(s);
            w.num(n);
        }
        Cond::Filtered(s, n) => {
            w.u8(2);
            w.u16(s);
            w.num(n);
        }
        Cond::AnimAttr(a, n) => {
            w.u8(3);
            w.o16(a);
            w.num(n);
        }
        Cond::WillExpire { in_time, wait } => {
            w.u8(4);
            w.f(in_time);
            w.b(wait);
        }
        Cond::ExpireInTime(n) => {
            w.u8(5);
            w.num(n);
        }
        Cond::InTimeWindow { start, length } => {
            w.u8(6);
            w.f(start);
            w.f(length);
        }
        Cond::StateTime(s, n) => {
            w.u8(7);
            w.u16(s);
            w.num(n);
        }
        Cond::LastState(s) => {
            w.u8(8);
            w.u16(s);
        }
        Cond::CurrentState(s) => {
            w.u8(9);
            w.u16(s);
        }
        Cond::PhysState(k) => {
            w.u8(10);
            w.u8(k);
        }
        Cond::Grinding(g) => {
            w.u8(11);
            w.o16(g);
        }
        Cond::Speed { along, num } => {
            w.u8(12);
            w.b(along);
            w.num(num);
        }
        Cond::SpeedAndSlope(n) => {
            w.u8(13);
            w.num(n);
        }
        Cond::TimeSinceLastInput(n) => {
            w.u8(14);
            w.num(n);
        }
        Cond::TimeToLand(n) => {
            w.u8(15);
            w.num(n);
        }
        Cond::Gesture(g) => {
            w.u8(16);
            w.u8(group_code(g));
        }
        Cond::GrabType(s) => {
            w.u8(17);
            w.u16(s);
        }
        Cond::HasTweak(n) => {
            w.u8(18);
            w.num(n);
        }
        Cond::ComVel { axis, num } => {
            w.u8(19);
            w.u8(axis);
            w.num(num);
        }
        Cond::SurfaceSlope(n) => {
            w.u8(20);
            w.num(n);
        }
        Cond::SkateSlope(n) => {
            w.u8(21);
            w.num(n);
        }
        Cond::GroundSlopeType(t) => {
            w.u8(22);
            w.u8(t);
        }
        Cond::LandingType(t) => {
            w.u8(23);
            w.u8(t);
        }
        Cond::ShouldLeaveSlide { right } => {
            w.u8(24);
            w.b(right);
        }
        Cond::CanEnterSlide { right } => {
            w.u8(25);
            w.b(right);
        }
        Cond::GrindTrickOutAllowed(s) => {
            w.u8(26);
            w.u16(s);
        }
        Cond::DistToEdge(n) => {
            w.u8(27);
            w.num(n);
        }
        Cond::Unsupported(s) => {
            w.u8(28);
            w.u16(s);
        }
        // Every field-less condition was written above.
        _ => w.u8(255),
    }
}

fn read_cond(r: &mut R) -> Option<Cond> {
    let tag = r.u8()?;
    if (100..127).contains(&tag) {
        return Some(PLAIN_CONDS[(tag - 100) as usize]);
    }
    Some(match tag {
        0 => Cond::Intent(r.u16()?, r.num()?),
        1 => Cond::AgIntent(r.u16()?, r.num()?),
        2 => Cond::Filtered(r.u16()?, r.num()?),
        3 => Cond::AnimAttr(r.o16()?, r.num()?),
        4 => Cond::WillExpire {
            in_time: r.f()?,
            wait: r.b()?,
        },
        5 => Cond::ExpireInTime(r.num()?),
        6 => Cond::InTimeWindow {
            start: r.f()?,
            length: r.f()?,
        },
        7 => Cond::StateTime(r.u16()?, r.num()?),
        8 => Cond::LastState(r.u16()?),
        9 => Cond::CurrentState(r.u16()?),
        10 => Cond::PhysState(r.u8()?),
        11 => Cond::Grinding(r.o16()?),
        12 => Cond::Speed {
            along: r.b()?,
            num: r.num()?,
        },
        13 => Cond::SpeedAndSlope(r.num()?),
        14 => Cond::TimeSinceLastInput(r.num()?),
        15 => Cond::TimeToLand(r.num()?),
        16 => Cond::Gesture(r.group()?),
        17 => Cond::GrabType(r.u16()?),
        18 => Cond::HasTweak(r.num()?),
        19 => Cond::ComVel {
            axis: r.u8()?,
            num: r.num()?,
        },
        20 => Cond::SurfaceSlope(r.num()?),
        21 => Cond::SkateSlope(r.num()?),
        22 => Cond::GroundSlopeType(r.u8()?),
        23 => Cond::LandingType(r.u8()?),
        24 => Cond::ShouldLeaveSlide { right: r.b()? },
        25 => Cond::CanEnterSlide { right: r.b()? },
        26 => Cond::GrindTrickOutAllowed(r.u16()?),
        27 => Cond::DistToEdge(r.num()?),
        28 => Cond::Unsupported(r.u16()?),
        _ => return None,
    })
}

/// Behaviours without fields, by tag.
const PLAIN_BEHS: [Beh; 16] = [
    Beh::JumpInto,
    Beh::ClearTrickAttr,
    Beh::SetDark,
    Beh::MonitorUnderflip,
    Beh::UpdateManualOutTimer,
    Beh::UpdateRidingFakie,
    Beh::ResetTimeSinceKickturn,
    Beh::UpdateTimeSinceKickturn,
    Beh::BodyFlippingSignal,
    Beh::BodySpin,
    Beh::FakieHead,
    Beh::Nothing,
    Beh::ApplyingBodyTilt,
    Beh::SetLandingData,
    Beh::StoreLandingData,
    Beh::SetManualAngle,
];

fn beh(w: &mut W, b: &Beh) {
    if let Some(i) = PLAIN_BEHS.iter().position(|p| p == b) {
        w.u8(100 + i as u8);
        return;
    }
    match b {
        Beh::Play(p) => {
            w.u8(0);
            w.node(p.anim);
            w.node(p.switch_anim);
            w.node(p.mirror_anim);
            w.u16(p.anim_name);
            w.f(p.speed);
            w.transition(&p.transition);
            w.u16(p.params.len() as u16);
            for q in &p.params {
                w.u8(q.from);
                w.u16(q.source);
                w.o16(q.target);
                w.o16(q.source_name);
                w.of(q.default);
                w.b(q.normalized);
            }
        }
        Beh::CreateAttribute {
            name,
            tree,
            values,
            set,
        } => {
            w.u8(1);
            w.u16(*name);
            w.o16(*tree);
            for v in values {
                w.of(*v);
            }
            w.b(*set);
        }
        Beh::AttachIntent { intent, attr } => {
            w.u8(2);
            w.u16(*intent);
            w.u16(*attr);
        }
        Beh::Filter(f) => {
            w.u8(3);
            w.u16(f.intent);
            w.u16(f.filtered);
            w.f(f.starting);
            w.f(f.default);
            w.f(f.scale);
            for k in f.filters {
                w.u8(k);
            }
            w.of(f.ramp_time);
            w.f(f.rising);
            w.f(f.falling);
            w.of(f.out);
            w.of(f.clamp_vel);
            w.of(f.clamp_acc);
        }
        Beh::SetTrickHeight {
            from,
            rename,
            value,
            manual,
            grind,
        } => {
            w.u8(4);
            w.o16(*from);
            w.o16(*rename);
            w.of(*value);
            w.b(*manual);
            w.b(*grind);
        }
        Beh::ScoringTrick(s) => {
            w.u8(5);
            w.u16(*s);
        }
        Beh::SetTrickAttr(s) => {
            w.u8(6);
            w.u16(*s);
        }
        Beh::Flag(f) => {
            w.u8(7);
            w.u8(FLAGS.iter().position(|x| x == f).unwrap_or(0) as u8);
        }
        Beh::ChooseRandomLanding(n) => {
            w.u8(8);
            w.u8(*n);
        }
        Beh::KickTurnSteering {
            spin_scale,
            max_height,
        } => {
            w.u8(9);
            w.f(*spin_scale);
            w.f(*max_height);
        }
        Beh::SetManualOutTimer(v) => {
            w.u8(10);
            w.f(*v);
        }
        Beh::DisableTricks(v) => {
            w.u8(11);
            w.f(*v);
        }
        Beh::SetGrabType(s) => {
            w.u8(12);
            w.u16(*s);
        }
        Beh::Push(p) => {
            w.u8(13);
            match *p {
                PushOp::Init => w.u8(0),
                PushOp::Cycle { foot } => {
                    w.u8(1);
                    w.u16(foot);
                }
                PushOp::Out { right } => {
                    w.u8(2);
                    w.b(right);
                }
                PushOp::FirstStrength => w.u8(3),
                PushOp::Coefs { first_only } => {
                    w.u8(4);
                    w.b(first_only);
                }
                PushOp::RepushDeadline { regular } => {
                    w.u8(5);
                    w.b(regular);
                }
                PushOp::TargetCoefs { regular } => {
                    w.u8(6);
                    w.b(regular);
                }
            }
        }
        Beh::CreateSlide { right } => {
            w.u8(14);
            w.b(*right);
        }
        Beh::SetSpeed { attr, value } => {
            w.u8(15);
            w.o16(*attr);
            w.of(*value);
        }
        Beh::MgFromAg {
            mg,
            ag,
            default,
            scale,
            filters,
            on_update,
        } => {
            w.u8(16);
            w.u16(*mg);
            w.u16(*ag);
            w.of(*default);
            w.f(*scale);
            for k in filters {
                w.u8(*k);
            }
            w.b(*on_update);
        }
        Beh::MgConst {
            mg,
            value,
            on_update,
        } => {
            w.u8(17);
            w.u16(*mg);
            w.f(*value);
            w.b(*on_update);
        }
        Beh::MgTime { mg, ag } => {
            w.u8(18);
            w.u16(*mg);
            w.u16(*ag);
        }
        Beh::TrickFromGesture(g) => {
            w.u8(19);
            w.u8(group_code(*g));
        }
        Beh::BoardAdjust { mag, angle } => {
            w.u8(20);
            w.u16(*mag);
            w.u16(*angle);
        }
        Beh::Unsupported(s) => {
            w.u8(21);
            w.u16(*s);
        }
        Beh::SetTurning { names } => {
            w.u8(22);
            for n in names {
                w.o16(*n);
            }
        }
        Beh::SettingBodyTilt(n) => {
            w.u8(23);
            w.o16(*n);
        }
        Beh::Crouching(n) => {
            w.u8(24);
            w.o16(*n);
        }
        Beh::SetDistComToBoard {
            attr,
            manually,
            adjust,
        } => {
            w.u8(25);
            w.o16(*attr);
            w.b(*manually);
            w.b(*adjust);
        }
        Beh::ScoringGrabs(g) => {
            w.u8(26);
            w.u16(g.base);
            for d in g.dirs {
                w.u16(d);
            }
            w.u16(g.x);
            w.u16(g.y);
            w.b(g.invert_y);
            w.b(g.polar);
        }
        Beh::TweakProject { x, y } => {
            w.u8(27);
            w.o16(*x);
            w.o16(*y);
        }
        Beh::AirLegExtension(n) => {
            w.u8(28);
            w.o16(*n);
        }
        Beh::DeckPitchAndYaw { yaw, pitch } => {
            w.u8(29);
            w.o16(*yaw);
            w.o16(*pitch);
        }
        // Every field-less behaviour was written above.
        _ => w.u8(255),
    }
}

fn read_beh(r: &mut R) -> Option<Beh> {
    let tag = r.u8()?;
    if (100..100 + PLAIN_BEHS.len() as u8).contains(&tag) {
        return Some(PLAIN_BEHS[(tag - 100) as usize].clone());
    }
    Some(match tag {
        0 => {
            let anim = r.node()?;
            let switch_anim = r.node()?;
            let mirror_anim = r.node()?;
            let anim_name = r.u16()?;
            let speed = r.f()?;
            let transition = r.transition()?;
            let n = r.u16()? as usize;
            let mut params = Vec::with_capacity(n);
            for _ in 0..n {
                params.push(ParamDef {
                    from: r.u8()?,
                    source: r.u16()?,
                    target: r.o16()?,
                    source_name: r.o16()?,
                    default: r.of()?,
                    normalized: r.b()?,
                });
            }
            Beh::Play(PlayDef {
                anim,
                switch_anim,
                mirror_anim,
                anim_name,
                speed,
                transition,
                params,
            })
        }
        1 => Beh::CreateAttribute {
            name: r.u16()?,
            tree: r.o16()?,
            values: [r.of()?, r.of()?, r.of()?],
            set: r.b()?,
        },
        2 => Beh::AttachIntent {
            intent: r.u16()?,
            attr: r.u16()?,
        },
        3 => Beh::Filter(FilterDef {
            intent: r.u16()?,
            filtered: r.u16()?,
            starting: r.f()?,
            default: r.f()?,
            scale: r.f()?,
            filters: [r.u8()?, r.u8()?, r.u8()?, r.u8()?],
            ramp_time: r.of()?,
            rising: r.f()?,
            falling: r.f()?,
            out: r.of()?,
            clamp_vel: r.of()?,
            clamp_acc: r.of()?,
        }),
        4 => Beh::SetTrickHeight {
            from: r.o16()?,
            rename: r.o16()?,
            value: r.of()?,
            manual: r.b()?,
            grind: r.b()?,
        },
        5 => Beh::ScoringTrick(r.u16()?),
        6 => Beh::SetTrickAttr(r.u16()?),
        7 => Beh::Flag(*FLAGS.get(r.u8()? as usize)?),
        8 => Beh::ChooseRandomLanding(r.u8()?),
        9 => Beh::KickTurnSteering {
            spin_scale: r.f()?,
            max_height: r.f()?,
        },
        10 => Beh::SetManualOutTimer(r.f()?),
        11 => Beh::DisableTricks(r.f()?),
        12 => Beh::SetGrabType(r.u16()?),
        13 => Beh::Push(match r.u8()? {
            0 => PushOp::Init,
            1 => PushOp::Cycle { foot: r.u16()? },
            2 => PushOp::Out { right: r.b()? },
            3 => PushOp::FirstStrength,
            4 => PushOp::Coefs { first_only: r.b()? },
            5 => PushOp::RepushDeadline { regular: r.b()? },
            6 => PushOp::TargetCoefs { regular: r.b()? },
            _ => return None,
        }),
        14 => Beh::CreateSlide { right: r.b()? },
        15 => Beh::SetSpeed {
            attr: r.o16()?,
            value: r.of()?,
        },
        16 => Beh::MgFromAg {
            mg: r.u16()?,
            ag: r.u16()?,
            default: r.of()?,
            scale: r.f()?,
            filters: [r.u8()?, r.u8()?, r.u8()?, r.u8()?],
            on_update: r.b()?,
        },
        17 => Beh::MgConst {
            mg: r.u16()?,
            value: r.f()?,
            on_update: r.b()?,
        },
        18 => Beh::MgTime {
            mg: r.u16()?,
            ag: r.u16()?,
        },
        19 => Beh::TrickFromGesture(r.group()?),
        20 => Beh::BoardAdjust {
            mag: r.u16()?,
            angle: r.u16()?,
        },
        21 => Beh::Unsupported(r.u16()?),
        22 => Beh::SetTurning {
            names: [r.o16()?, r.o16()?, r.o16()?, r.o16()?, r.o16()?],
        },
        23 => Beh::SettingBodyTilt(r.o16()?),
        24 => Beh::Crouching(r.o16()?),
        25 => Beh::SetDistComToBoard {
            attr: r.o16()?,
            manually: r.b()?,
            adjust: r.b()?,
        },
        26 => Beh::ScoringGrabs(GrabScore {
            base: r.u16()?,
            dirs: [r.u16()?, r.u16()?, r.u16()?, r.u16()?],
            x: r.u16()?,
            y: r.u16()?,
            invert_y: r.b()?,
            polar: r.b()?,
        }),
        27 => Beh::TweakProject {
            x: r.o16()?,
            y: r.o16()?,
        },
        28 => Beh::AirLegExtension(r.o16()?),
        29 => Beh::DeckPitchAndYaw {
            yaw: r.o16()?,
            pitch: r.o16()?,
        },
        _ => return None,
    })
}
