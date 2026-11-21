//! Which shot: Skate 3's own camera graph (`Default_cameragraph`, its low camera) run on the state-graph
//! runner (`crate::graph`) when the player's disc supplied it (`fg_import rules`: `rules/camera/graph`),
//! else flatground's own choice ([`built_in`]).
//!
//! The graph's conditions are the camera's own (SK8-ENGINE/skate-3-rust-engine, GPL-3.0,
//! crates/skate-game/src/camera/graph_conditions.rs): what the skater does (wiping out, off the board,
//! grinding, in a manual, preparing to jump), the predicted flight (air time, top height, landing
//! height, the top against the camera's distance at take-off, launch and landing slopes, reentry), how
//! long the steering has been off centre, the rail's slope, which shot is on. Its behaviour
//! `CameraChooseShot` picks a shot by name with optional transition times in and out.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use crate::graph::ops::{Cmp, Num};
use crate::graph::{Controller, Frame, Graph, Host, OP_BEHAVIOUR, OP_CONDITION, OpId, Pack};

use super::shot::{ShotSet, name_hash};

pub const ENTRY: &str = "rules/camera/graph";
/// The graph's name in the entry.
pub const GRAPH: &str = "Camera";

/// The values the conditions read this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Conditions {
    pub wiping_out: bool,
    pub off_board: bool,
    pub onboard_air: bool,
    pub preparing: bool,
    pub manual: bool,
    pub hippy_jump: bool,
    pub grinding: bool,
    pub was_grinding: bool,
    pub on_ground: bool,
    pub reentry: bool,
    pub ledge_left: bool,
    pub ledge_right: bool,
    /// Speed along the board (m/s, signed), upwards, acceleration.
    pub speed: f32,
    pub speed_y: f32,
    pub accel: f32,
    /// The predicted flight: air time (s, 0 without one), landing and top heights over the take-off,
    /// the top against the camera's distance at take-off.
    pub air_time: f32,
    pub landing_height: f32,
    pub max_height: f32,
    pub height_ratio: f32,
    /// Radians (only its sign is read).
    pub frontside: f32,
    /// Degrees.
    pub transfer: f32,
    pub launch_incline: f32,
    pub landing_incline: f32,
    pub grind_incline: f32,
    pub incline: f32,
    pub centred_time: f32,
    pub turning_time: f32,
    /// The current shot's key.
    pub shot: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Flag {
    WipingOut,
    Offboard,
    OnboardAir,
    Preparing,
    Manual,
    HippyJump,
    Grinding,
    WasGrinding,
    OnGround,
    Reentry,
    LedgeLeft,
    LedgeRight,
    /// Conditions about things flatground doesn't have (observer mode, skitching, plants, bonelesses,
    /// moving objects, painted regions, volumes, bad collision): false.
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Speed,
    SpeedY,
    Accel,
    AirTime,
    LandingHeight,
    MaxHeight,
    HeightRatio,
    Frontside,
    Transfer,
    LaunchIncline,
    LandingIncline,
    GrindIncline,
    Incline,
    AbsIncline,
    CentredTime,
    TurningTime,
    SinceInput,
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Flag(Flag),
    Num(Value, Num),
    /// `IsCameraTypeActive`: 0 is the low camera, flatground's.
    Type(u32),
    Previous(Vec<u64>),
    /// `IsWipeoutBodyTweak`: none of the named tweaks happen in flatground's wipeouts.
    Tweak(u32),
    Choose {
        name: String,
        transition_in: f32,
        transition_out: f32,
    },
    Ignore,
}

/// A shot the graph chose this tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub name: String,
    pub transition_in: f32,
    pub transition_out: f32,
}

/// Skate 3's camera graph, compiled by the importer, with its operations typed.
#[derive(Clone, Debug)]
pub struct CamGraph {
    graph: Graph,
    ops: Vec<Op>,
    names: Vec<String>,
    controller: Controller,
    /// Names the graph uses that flatground doesn't implement (logged once).
    pub unsupported: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Graph(crate::graph::Error),
    NoGraph,
}

fn parse_num(p: &Pack, attrs: &[crate::graph::Attr]) -> Num {
    let cmp = match p.text(attrs, "comparison") {
        Some("NotEqual") => Cmp::Ne,
        Some("GreaterThan") => Cmp::Gt,
        Some("LessThan") => Cmp::Lt,
        Some("GreaterEqual") => Cmp::Ge,
        Some("LessEqual") => Cmp::Le,
        Some(_) => Cmp::Eq,
        None => Cmp::None,
    };
    Num {
        cmp,
        threshold: p.float(attrs, "value", 0.0),
        abs: false,
    }
}

fn shot_names(p: &Pack, attrs: &[crate::graph::Attr]) -> Vec<String> {
    let mut out = Vec::new();
    for k in ["shot1", "shot2", "shot3", "shot4"] {
        match p.text(attrs, k) {
            Some(n) => out.push(String::from(n)),
            None => break,
        }
    }
    out
}

impl CamGraph {
    pub fn load(entry: &[u8]) -> Result<CamGraph, Error> {
        let p = Pack::parse(entry).map_err(Error::Graph)?;
        let g = p
            .graph(GRAPH)
            .or_else(|| p.graphs.first())
            .ok_or(Error::NoGraph)?
            .clone();
        let mut unsupported = BTreeSet::new();
        let ops = (0..g.ops.len())
            .map(|i| {
                let o = &g.ops[i];
                let name = p.str(o.name);
                let a = g.op_attrs(i as OpId);
                match o.kind {
                    OP_CONDITION => {
                        let flag = match name {
                            "IsWipingOut" => Some(Flag::WipingOut),
                            "IsOffboard" => Some(Flag::Offboard),
                            "IsInOnBoardAir" => Some(Flag::OnboardAir),
                            "IsPreparingToJump" => Some(Flag::Preparing),
                            "IsInManual" => Some(Flag::Manual),
                            "SkaterIsPerformingHippyJump" => Some(Flag::HippyJump),
                            "SkaterIsGrinding" => Some(Flag::Grinding),
                            "SkaterWasGrinding" => Some(Flag::WasGrinding),
                            "IsPlayerSkateboardOnGround" => Some(Flag::OnGround),
                            "IsReentry" => Some(Flag::Reentry),
                            "IsGrindLedgeLeft" => Some(Flag::LedgeLeft),
                            "IsGrindLedgeRight" => Some(Flag::LedgeRight),
                            "IsInObserverMode"
                            | "IsBrokenBoneSlowMo"
                            | "IsAirOffboard"
                            | "IsFixedInAir"
                            | "SkaterIsDroppingIn"
                            | "SkaterIsMovingObject"
                            | "SkaterIsSkitching"
                            | "SkaterIsPerformingFootPlant"
                            | "SkaterIsPerformingHandPlant"
                            | "SkaterIsPerformingBoneless"
                            | "SkaterIsPerformingHippyHurdle"
                            | "SkaterIsOnRoad"
                            | "IsInWorldPainterRegion"
                            | "SkaterIsUnderThreat"
                            | "IsCameraInBadCollision"
                            | "IsInVolume" => Some(Flag::Never),
                            _ => None,
                        };
                        if let Some(f) = flag {
                            return Op::Flag(f);
                        }
                        let value = match name {
                            "CameraSubjectSpeed" => Some(Value::Speed),
                            "CameraSubjectSpeedY" => Some(Value::SpeedY),
                            "CameraSubjectAcceleration" => Some(Value::Accel),
                            "PredictedAirTime" => Some(Value::AirTime),
                            "PredictedLandingHeight" => Some(Value::LandingHeight),
                            "PredictedMaxHeight" => Some(Value::MaxHeight),
                            "PredictedHeightDistanceRatio" => Some(Value::HeightRatio),
                            "FrontsideAngle" => Some(Value::Frontside),
                            "TransferAngle" => Some(Value::Transfer),
                            "LaunchInclineAngle" => Some(Value::LaunchIncline),
                            "LandingInclineAngle" => Some(Value::LandingIncline),
                            "GrindInclineAngle" => Some(Value::GrindIncline),
                            "InclineAngle" => Some(Value::Incline),
                            "InclineAngleAbs" => Some(Value::AbsIncline),
                            "TurningCentredTime" => Some(Value::CentredTime),
                            "TurningTime" => Some(Value::TurningTime),
                            "TimeSinceLastPlayerInput" => Some(Value::SinceInput),
                            _ => None,
                        };
                        if let Some(v) = value {
                            return Op::Num(v, parse_num(&p, a));
                        }
                        match name {
                            "IsCameraTypeActive" => Op::Type(
                                p.text(a, "type")
                                    .and_then(|t| t.trim().parse().ok())
                                    .unwrap_or(u32::MAX),
                            ),
                            "IsPreviousShot" => Op::Previous(
                                shot_names(&p, a).iter().map(|n| name_hash(n)).collect(),
                            ),
                            "IsWipeoutBodyTweak" => Op::Tweak(match p.text(a, "tweak") {
                                Some("CannonBall") => 1,
                                Some("JudoKick") => 2,
                                Some("SwanDive") => 3,
                                Some("Torpedo") => 4,
                                _ => 0,
                            }),
                            _ => {
                                unsupported.insert(String::from(name));
                                Op::Flag(Flag::Never)
                            }
                        }
                    }
                    OP_BEHAVIOUR => match name {
                        "CameraChooseShot" => match shot_names(&p, a).into_iter().next() {
                            Some(n) => Op::Choose {
                                name: n,
                                transition_in: p.float(a, "transitionIn", -1.0),
                                transition_out: p.float(a, "transitionOut", -1.0),
                            },
                            None => Op::Ignore,
                        },
                        // A debug print, and Skate 3's slow motion on long airs (flatground keeps
                        // real time).
                        "PrintText2D" | "SlowMotionController" => Op::Ignore,
                        _ => {
                            unsupported.insert(String::from(name));
                            Op::Ignore
                        }
                    },
                    _ => Op::Ignore,
                }
            })
            .collect();
        let names = g
            .states
            .iter()
            .map(|s| String::from(p.str(s.name)))
            .collect();
        let controller = Controller::new(&g);
        Ok(CamGraph {
            graph: g,
            ops,
            names,
            controller,
            unsupported,
        })
    }

    /// Every shot name the graph can choose.
    pub fn shot_names(&self) -> impl Iterator<Item = &str> {
        self.ops.iter().filter_map(|o| match o {
            Op::Choose { name, .. } => Some(name.as_str()),
            _ => None,
        })
    }

    /// One tick: the shots chosen (in the order their states were entered).
    pub fn update(&mut self, c: &Conditions, dt: f32) -> Vec<Choice> {
        let mut host = CamHost {
            ops: &self.ops,
            c,
            chosen: Vec::new(),
        };
        self.controller.update(&self.graph, dt, &mut host);
        host.chosen
    }

    /// The current state's path (for the log).
    pub fn state(&self) -> String {
        let mut parts = Vec::new();
        let mut s = self.controller.frame.current;
        while let Some(id) = s {
            parts.push(self.names[id as usize].as_str());
            s = self.graph.parent(id);
        }
        parts.reverse();
        parts.iter().skip(1).copied().collect::<Vec<_>>().join("/")
    }

    pub fn restart(&mut self) {
        self.controller = Controller::new(&self.graph);
    }
}

struct CamHost<'a> {
    ops: &'a [Op],
    c: &'a Conditions,
    chosen: Vec<Choice>,
}

impl Host for CamHost<'_> {
    fn condition(&mut self, op: OpId, _frame: &Frame) -> bool {
        let c = self.c;
        match &self.ops[op as usize] {
            Op::Flag(f) => match f {
                Flag::WipingOut => c.wiping_out,
                Flag::Offboard => c.off_board,
                Flag::OnboardAir => c.onboard_air,
                Flag::Preparing => c.preparing,
                Flag::Manual => c.manual,
                Flag::HippyJump => c.hippy_jump,
                Flag::Grinding => c.grinding,
                Flag::WasGrinding => c.was_grinding,
                Flag::OnGround => c.on_ground,
                Flag::Reentry => c.reentry,
                Flag::LedgeLeft => c.ledge_left,
                Flag::LedgeRight => c.ledge_right,
                Flag::Never => false,
            },
            Op::Num(v, n) => n.matches(match v {
                Value::Speed => c.speed,
                Value::SpeedY => c.speed_y,
                Value::Accel => c.accel,
                Value::AirTime => c.air_time,
                Value::LandingHeight => c.landing_height,
                Value::MaxHeight => c.max_height,
                Value::HeightRatio => c.height_ratio,
                Value::Frontside => c.frontside,
                Value::Transfer => c.transfer,
                Value::LaunchIncline => c.launch_incline,
                Value::LandingIncline => c.landing_incline,
                Value::GrindIncline => c.grind_incline,
                Value::Incline => c.incline,
                Value::AbsIncline => c.incline.abs(),
                Value::CentredTime => c.centred_time,
                Value::TurningTime => c.turning_time,
                Value::SinceInput => 0.0,
            }),
            Op::Type(t) => *t == 0,
            Op::Previous(keys) => c.shot.is_some_and(|k| keys.contains(&k)),
            Op::Tweak(t) => *t == 0,
            _ => false,
        }
    }

    fn begin(&mut self, op: OpId, _frame: &Frame) {
        if let Op::Choose {
            name,
            transition_in,
            transition_out,
        } = &self.ops[op as usize]
        {
            self.chosen.push(Choice {
                name: name.clone(),
                transition_in: *transition_in,
                transition_out: *transition_out,
            });
        }
    }

    fn update(&mut self, _op: OpId, _frame: &Frame) {}
    fn end(&mut self, _op: OpId, _frame: &Frame) {}
    fn hook(&mut self, _op: OpId, _frame: &Frame) {}
}

/// flatground's own shot choice (without Skate 3's graph): wipeouts and run-outs from high up, grinds
/// beside the rail, small airs keep the shot, long airs pull back, manuals low, hard turns swing, else
/// the riding shot. Thresholds are ours.
pub fn built_in(c: &Conditions, set: &ShotSet) -> Choice {
    let pick = |name: &str, tin: f32, tout: f32| Choice {
        name: String::from(if set.find(name).is_some() {
            name
        } else {
            "bl_chase"
        }),
        transition_in: tin,
        transition_out: tout,
    };
    let flying = !c.on_ground && !c.grinding && c.air_time > 0.12;
    if c.wiping_out {
        pick("wipeout", 0.15, 0.8)
    } else if c.off_board {
        pick("offboard_ground", 0.8, 0.5)
    } else if c.was_grinding && flying {
        pick("grind_exit", 0.6, 0.3)
    } else if c.grinding {
        if c.ledge_left {
            pick("bl_grind_ledge_left", 1.0, 0.3)
        } else if c.ledge_right {
            pick("bl_grind_ledge_right", 1.0, 0.3)
        } else {
            pick("bl_grind", 0.5, 0.3)
        }
    } else if flying {
        if c.air_time > 1.3 || c.max_height > 1.3 {
            pick("bl_air", -1.0, 0.5)
        } else {
            pick("low_ollie", 0.8, 0.4)
        }
    } else if c.manual {
        pick("bl_manual", 1.5, -1.0)
    } else if !c.preparing && c.centred_time < 0.25 && c.speed > 1.0 {
        pick("bl_turning", -1.0, -1.0)
    } else {
        pick("bl_chase", -1.0, -1.0)
    }
}
