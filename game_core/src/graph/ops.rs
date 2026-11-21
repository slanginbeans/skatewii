//! Typed graph operations: every behaviour, condition and hook of a [`Graph`] parsed once at load from its
//! name and attributes. Parameter meanings follow SK8-ENGINE/skate-3-rust-engine (GPL-3.0;
//! crates/skate-game/src/graph_host: motion_nodes.rs, motion_conditions.rs, condition_nodes.rs,
//! action_nodes.rs, motion_hooks.rs, motion_intent_filter.rs, motion_tricks.rs, motion_riding.rs).
//!
//! Names (intents, attributes, states) stay graph string ids ([`Str`]); animation names are resolved
//! against the motion library when it is present.

use alloc::vec::Vec;

use super::{Attr, Graph, NONE, OP_BEHAVIOUR, OP_CONDITION, OpId, Pack, StateId, Str};
use crate::motion::{MotionDb, NameId, Node};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmp {
    None,
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    GtAbs,
}

/// A numeric test authored as `greater="0.5"` and the like (`abs` compares the absolute value).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Num {
    pub cmp: Cmp,
    pub threshold: f32,
    pub abs: bool,
}

impl Num {
    pub const ANY: Num = Num {
        cmp: Cmp::None,
        threshold: 0.0,
        abs: false,
    };

    pub fn matches(self, v: f32) -> bool {
        let v = if self.abs { v.abs() } else { v };
        match self.cmp {
            Cmp::None => false,
            Cmp::Eq => v == self.threshold,
            Cmp::Ne => v != self.threshold,
            Cmp::Gt => v > self.threshold,
            Cmp::Lt => v < self.threshold,
            Cmp::Ge => v >= self.threshold,
            Cmp::Le => v <= self.threshold,
            Cmp::GtAbs => v.abs() > self.threshold,
        }
    }

    /// Present-and-matching, where no comparison means "present".
    pub fn test(self, v: Option<f32>) -> bool {
        v.is_some_and(|v| self.cmp == Cmp::None || self.matches(v))
    }
}

/// Skate 3's trick gesture groups (HasGestureIntent / CreateTrickIntentFromGesture).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Square,
    Nose,
    Tail,
    Nose90,
    Tail90,
    NoseN90,
    TailN90,
}

impl Group {
    pub fn parse(s: &str) -> Option<Group> {
        Some(match s {
            "Square" => Group::Square,
            "Nose" => Group::Nose,
            "Tail" => Group::Tail,
            "90Nose" => Group::Nose90,
            "90Tail" => Group::Tail90,
            "N90Nose" => Group::NoseN90,
            "N90Tail" => Group::TailN90,
            _ => return None,
        })
    }
}

/// Physics state categories (PhysFilteredState).
pub mod phys {
    pub const GROUND: u8 = 1;
    pub const AIR: u8 = 2;
    pub const GRIND: u8 = 3;
    pub const WIPEOUT: u8 = 4;
    pub const TELEPORT: u8 = 5;
    pub const OFFBOARD: u8 = 6;
    pub const OFFBOARD_AIR: u8 = 7;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cond {
    /// HasIntent (motion-graph intent).
    Intent(Str, Num),
    /// HasAGIntent (action-graph intent).
    AgIntent(Str, Num),
    /// HasFilteredMotionGraphIntent.
    Filtered(Str, Num),
    /// HasAnimAttribute: the playing clips carry the attribute (and it matches).
    AnimAttr(Option<NameId>, Num),
    WillExpire {
        in_time: f32,
        wait: bool,
    },
    ExpireInTime(Num),
    InTimeWindow {
        start: f32,
        length: f32,
    },
    /// InStateForTime / InParentStateForTime.
    StateTime(StateId, Num),
    LastState(StateId),
    CurrentState(StateId),
    PhysState(u8),
    Grinding(Option<Str>),
    Mirrored,
    RidingFakie,
    RidingGoofy,
    RidingSwitch,
    Speed {
        along: bool,
        num: Num,
    },
    SpeedAndSlope(Num),
    TimeSinceLastInput(Num),
    TimeToLand(Num),
    DisablePushBrake,
    Gesture(Group),
    GrabType(Str),
    HasTweak(Num),
    ManualOutTimer,
    WantsManualExit,
    WantsRunout,
    WantsWipeout,
    Dark,
    DarkCatchRequested,
    UnderflipRequested,
    Random,
    ComVel {
        axis: u8,
        num: Num,
    },
    BreakOutOfPush,
    SurfaceSlope(Num),
    SkateSlope(Num),
    GroundSlopeType(u8),
    AllowedToTrick,
    OkToTrickOnStairs,
    Tricking,
    LandingType(u8),
    CanLandOnBoard,
    CrouchedForGrab,
    ShouldLeaveSlide {
        right: bool,
    },
    CanEnterSlide {
        right: bool,
    },
    GrindTrickOutAllowed(Str),
    Blunting,
    TrucksOrDeck,
    Apex,
    DistToEdge(Num),
    AngleAdjustLess,
    BodyFlipping,
    /// The physics has finished the wipeout (the rider is back on the board).
    DoneWipingOut,
    /// Conditions of systems flatground doesn't have, with the value they take here.
    Fixed(bool),
    Unsupported(Str),
}

/// FilterMotionGraphIntent settings (SK8 intent_filter::Settings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterDef {
    pub intent: Str,
    pub filtered: Str,
    pub starting: f32,
    pub default: f32,
    pub scale: f32,
    pub filters: [u8; 4],
    pub ramp_time: Option<f32>,
    pub rising: f32,
    pub falling: f32,
    pub out: Option<f32>,
    pub clamp_vel: Option<f32>,
    pub clamp_acc: Option<f32>,
}

/// How a new animation joins the playing one (SK8 TransitionSettings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionDef {
    /// 1 play (cut), 2 blend, 3 channel blend, 4 sequence.
    pub kind: u8,
    pub seconds: f32,
    pub under: bool,
    /// 0 none, 1 blend with the outgoing frozen, 2 match phase, 3 match frame.
    pub matching: u8,
    pub channels_from_weights: bool,
}

/// A tree parameter source of PlayAnimation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamDef {
    /// 0 motion intent, 1 filtered intent, 2 last animation's attribute.
    pub from: u8,
    pub source: Str,
    /// The tree parameter set (motion library name).
    pub target: Option<NameId>,
    /// The source as a motion-library name (for `lastAnim`).
    pub source_name: Option<NameId>,
    pub default: Option<f32>,
    pub normalized: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayDef {
    pub anim: Option<Node>,
    pub switch_anim: Option<Node>,
    pub mirror_anim: Option<Node>,
    pub anim_name: Str,
    pub speed: f32,
    pub transition: TransitionDef,
    pub params: Vec<ParamDef>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Beh {
    Play(PlayDef),
    /// CreateAttribute: a value for the physics packet on begin/update/end (`set`: also a tree parameter).
    CreateAttribute {
        name: Str,
        tree: Option<NameId>,
        values: [Option<f32>; 3],
        set: bool,
    },
    AttachIntent {
        intent: Str,
        attr: Str,
    },
    Filter(FilterDef),
    SetTrickHeight {
        from: Option<NameId>,
        rename: Option<NameId>,
        value: Option<f32>,
        manual: bool,
        grind: bool,
    },
    JumpInto,
    ScoringTrick(Str),
    SetTrickAttr(Str),
    ClearTrickAttr,
    /// Behaviours that only mark a mode while they run (IsDoingTrick, IsManualing, ...).
    Flag(Flag),
    SetDark,
    MonitorUnderflip,
    ChooseRandomLanding(u8),
    KickTurnSteering {
        spin_scale: f32,
        max_height: f32,
    },
    SetManualOutTimer(f32),
    UpdateManualOutTimer,
    DisableTricks(f32),
    UpdateRidingFakie,
    ResetTimeSinceKickturn,
    UpdateTimeSinceKickturn,
    SetGrabType(Str),
    ScoringGrabs,
    Push(PushOp),
    CreateSlide {
        right: bool,
    },
    SetSpeed {
        attr: Option<NameId>,
        value: Option<f32>,
    },
    SetTurning,
    // Action graph:
    MgFromAg {
        mg: Str,
        ag: Str,
        default: Option<f32>,
        scale: f32,
        filters: [u8; 4],
        on_update: bool,
    },
    MgConst {
        mg: Str,
        value: f32,
        on_update: bool,
    },
    MgTime {
        mg: Str,
        ag: Str,
    },
    TrickFromGesture(Group),
    BoardAdjust {
        mag: Str,
        angle: Str,
    },
    BodyFlippingSignal,
    /// FakieHeadChannel: the fakie head and torso channel.
    FakieHead,
    /// Behaviours with nothing to do in flatground (hand services, debug text, off-board helpers).
    Nothing,
    Unsupported(Str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Trick,
    Manual,
    PowerSlide,
    Landing,
    Anticipating,
    SlideCandidate,
    NoPumping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushOp {
    Init,
    /// PushCycle: `foot` is its `newPushName` intent (`RightPush` / `LeftPush`).
    Cycle {
        foot: Str,
    },
    Out {
        right: bool,
    },
    FirstStrength,
    /// SetPushCoefs (`onFirstUpdateOnly`).
    Coefs {
        first_only: bool,
    },
    /// ComputeRepushDeadline / ComputeTargetCoefsFromSpeedAndStrength (`regularAttributes`).
    RepushDeadline {
        regular: bool,
    },
    TargetCoefs {
        regular: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hook {
    Override(TransitionDef),
    GrabSlide { right: bool },
    MongoPushToAntic,
    Unsupported(Str),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Cond(Cond),
    Beh(Beh),
    Hook(Hook),
}

/// Reads one operation's attributes.
struct A<'p, 'g> {
    p: &'p Pack<'p>,
    attrs: &'g [Attr],
}

impl A<'_, '_> {
    fn get(&self, k: &str) -> Option<&Attr> {
        self.p.attr(self.attrs, k)
    }
    fn text(&self, k: &str) -> Option<&str> {
        self.p.text(self.attrs, k)
    }
    fn str(&self, k: &str) -> Str {
        self.get(k).map_or(NONE, |a| a.text)
    }
    fn f(&self, k: &str, d: f32) -> f32 {
        self.p.float(self.attrs, k, d)
    }
    fn of(&self, k: &str) -> Option<f32> {
        self.get(k).map(|a| a.float)
    }
    fn b(&self, k: &str, d: bool) -> bool {
        self.p.flag(self.attrs, k, d)
    }
    fn num(&self) -> Num {
        if let Some(c) = self.text("comparison") {
            let cmp = match c {
                "NotEqual" => Cmp::Ne,
                "GreaterThan" => Cmp::Gt,
                "LessThan" => Cmp::Lt,
                "GreaterEqual" => Cmp::Ge,
                "LessEqual" => Cmp::Le,
                _ => Cmp::Eq,
            };
            return Num {
                cmp,
                threshold: self.f("value", 0.0),
                abs: self.b("abs", false),
            };
        }
        for (cmp, k) in [
            (Cmp::Eq, "equal"),
            (Cmp::Ne, "notEqual"),
            (Cmp::Gt, "greater"),
            (Cmp::GtAbs, "greaterThanAbs"),
            (Cmp::Lt, "less"),
            (Cmp::Ge, "greaterEqual"),
            (Cmp::Le, "lessEqual"),
        ] {
            if let Some(a) = self.get(k) {
                return Num {
                    cmp,
                    threshold: a.float,
                    abs: self.b("abs", false),
                };
            }
        }
        Num {
            abs: self.b("abs", false),
            ..Num::ANY
        }
    }
}

fn filter(s: Option<&str>) -> u8 {
    match s {
        Some("negate") => 1,
        Some("abs") => 2,
        Some("oneMinus") => 3,
        Some("clamp") => 4,
        Some("angleFlip") => 5,
        Some("angleRot90") => 6,
        Some("angleRotN90") => 7,
        _ => 0,
    }
}

fn transition(a: &A) -> TransitionDef {
    TransitionDef {
        kind: match a.text("transType").unwrap_or("") {
            "play" => 1,
            "sequence" => 4,
            "channelblend" => 3,
            _ => 2,
        },
        seconds: a.f("time", 0.2),
        under: a.b("transitionUnder", false),
        matching: if a.b("blendWithCurrentFrame", false) {
            1
        } else if a.b("blendMatchPhase", false) {
            2
        } else if a.b("blendMatchFrame", false) {
            3
        } else {
            0
        },
        channels_from_weights: a.b("useChannelFromWeights", false),
    }
}

/// Conditions that belong to systems flatground leaves out (off-board walking, skitching, hand plants,
/// water, moving objects, debug), with the value that keeps the graph on the board.
const FIXED_FALSE: &[&str] = &[
    "IsDroppingIn",
    "IsDropInWithoutTransition",
    "IsFootPlanting",
    "IsGrabbingObject",
    "IsHandPlanting",
    "IsInLocomotion",
    "IsLandingOnBoard",
    "PhysicsRequestsDismount",
    "IsEnteringSkitch",
    "IsSkitching",
    "IsSkitchShimmying",
    "IsSkitchingWithAbsorb",
    "SkitchingPosition",
    "IsInWater",
    "IsMovingObject",
    "IsStandingOnMovingObject",
    "IsInDebugAnimationsMode",
    "IsHoldingSkateboard",
    "IsDroppingSkateboard",
    "IsRetrievingSkateboard",
    "IsHippyHurdling",
    "IsInBipedAir",
    "IsBipedGroundThin",
    "IsBipedCommittedToMotion",
    "CanBipedLand",
    "HasNewHandPlantPos",
    "ShouldPlayHandPlantAnim",
    "ShouldPrepareOneFootAirForFootplant",
    "IsArmChannelPlaying",
    "LocoState",
    "OBTimeToLand",
    "OBTrajTime",
    "MongoPushFootToFar",
    "IsBumped",
    "IsProSkater",
    "IsLandingIntoGrind",
    "WipeoutTimeSinceContact",
    "WipeoutTimeToLand",
    "IsGrindApproach",
    "HasTiltToLargeForPreland",
    "IsPushOffEnabled",
    "DisableDismount",
    "IsDeckFree",
];
const FIXED_TRUE: &[&str] = &["EnoughDistToObstacle"];

fn cond(a: &A, name: &str, bound: StateId, db: Option<&MotionDb>) -> Cond {
    let n = || a.num();
    let lib = |k: &str| a.text(k).and_then(|t| db.and_then(|d| d.name_id(t)));
    match name {
        "HasIntent" => Cond::Intent(a.str("intent"), n()),
        "HasAGIntent" => Cond::AgIntent(a.str("intent"), n()),
        "HasFilteredMotionGraphIntent" => Cond::Filtered(a.str("intent"), n()),
        "HasAnimAttribute" => Cond::AnimAttr(lib("attribute"), n()),
        "WillExpire" => Cond::WillExpire {
            in_time: a.f("InTime", 0.0),
            wait: a.b("waitForTransitions", true),
        },
        "ExpireInTime" => Cond::ExpireInTime(n()),
        "InTimeWindow" => Cond::InTimeWindow {
            start: a.f("StartTime", 0.0),
            length: a.f("WindowFrameLength", 0.0),
        },
        "InStateForTime" | "InParentStateForTime" => Cond::StateTime(bound, n()),
        "LastState" => Cond::LastState(bound),
        "CurrentState" => Cond::CurrentState(bound),
        "PhysFilteredState" => Cond::PhysState(match a.text("state").unwrap_or("") {
            "ground" => phys::GROUND,
            "air" => phys::AIR,
            "grind" => phys::GRIND,
            "wipeout" => phys::WIPEOUT,
            "teleport" => phys::TELEPORT,
            "offboard" => phys::OFFBOARD,
            "offboardair" => phys::OFFBOARD_AIR,
            _ => 0,
        }),
        "IsGrinding" => Cond::Grinding(a.get("grindName").map(|x| x.text)),
        "IsMirrored" => Cond::Mirrored,
        "IsRidingFakie" => Cond::RidingFakie,
        "IsRidingGoofy" => Cond::RidingGoofy,
        "IsRidingSwitch" => Cond::RidingSwitch,
        "PhysicsSpeedCompare" => Cond::Speed {
            along: a.b("alongSkateZ", false),
            num: n(),
        },
        "PhysicsSpeedAndSlopeCompare" => Cond::SpeedAndSlope(n()),
        "TimeSinceLastInput" => Cond::TimeSinceLastInput(n()),
        "TimeToLand" => Cond::TimeToLand(n()),
        "DisablePushBrake" => Cond::DisablePushBrake,
        "HasGestureIntent" => match Group::parse(a.text("group").unwrap_or("")) {
            Some(g) => Cond::Gesture(g),
            None => Cond::Fixed(false),
        },
        "CurrentGrabType" => Cond::GrabType(a.str("grab")),
        "HasTweak" => Cond::HasTweak(n()),
        "ManualOutTimerIsActive" => Cond::ManualOutTimer,
        "PhysicsWantsManualExit" => Cond::WantsManualExit,
        "PhysicsWantsRunout" => Cond::WantsRunout,
        "PhysicsWantsWipeOut" | "IsPhysicsWiping" => Cond::WantsWipeout,
        "IsDark" => Cond::Dark,
        "IsDarkCatchRequested" => Cond::DarkCatchRequested,
        "IsUnderflipRequested" => Cond::UnderflipRequested,
        "RandomCond" => Cond::Random,
        "ComVelCompare" => Cond::ComVel {
            axis: match a.text("axis").unwrap_or("y") {
                "x" | "X" => 0,
                "z" | "Z" => 2,
                _ => 1,
            },
            num: n(),
        },
        "BreakOutOfPush" => Cond::BreakOutOfPush,
        "SurfaceSlope" => Cond::SurfaceSlope(n()),
        "SkateSlope" => Cond::SkateSlope(n()),
        "GroundSlopeType" => Cond::GroundSlopeType(match a.text("slopetype").unwrap_or("") {
            "flat" | "Flat" => 0,
            "up" | "Up" | "uphill" => 1,
            "down" | "Down" | "downhill" => 2,
            _ => 3,
        }),
        "AllowedToTrick" => Cond::AllowedToTrick,
        "OkToDoTrickOnStairs" => Cond::OkToTrickOnStairs,
        "IsTricking" => Cond::Tricking,
        "HasLandingType" => Cond::LandingType(match a.text("landingType").unwrap_or("") {
            "sketchy" | "Sketchy" => 1,
            "spin" | "Spin" => 2,
            _ => 0,
        }),
        "CanLandOnBoard" => Cond::CanLandOnBoard,
        "IsCrouchedEnoughForBlendToGrabCycle" => Cond::CrouchedForGrab,
        "ShouldLeaveSlide" => Cond::ShouldLeaveSlide {
            right: a.b("right", true),
        },
        "CanEnterSlide" => Cond::CanEnterSlide {
            right: a.b("right", true),
        },
        "GrindTrickOutTypeAllowed" => Cond::GrindTrickOutAllowed(a.str("type")),
        "IsGrindBluntingBackslash" => Cond::Blunting,
        "TrucksOrDeckInContact" => Cond::TrucksOrDeck,
        "ApexReached" => Cond::Apex,
        "DistToEdge" => Cond::DistToEdge(n()),
        "AngleAdjustByAnimationEndLessThan" => Cond::AngleAdjustLess,
        "IsBodyFlipping" => Cond::BodyFlipping,
        "IsDoneWipingOut" => Cond::DoneWipingOut,
        "IsTimeToLand" => Cond::TimeToLand(n()),
        _ if FIXED_FALSE.contains(&name) => Cond::Fixed(false),
        _ if FIXED_TRUE.contains(&name) => Cond::Fixed(true),
        _ => Cond::Unsupported(NONE),
    }
}

/// Behaviours that need nothing from flatground (hand and arm services, debug, scoring augmentations,
/// off-board helpers, physics-only rigs we do differently).
const NOTHING: &[&str] = &[
    "PrintText2D",
    "JuiceHook",
    "HandBusy",
    "EndGesture",
    "CharacterGesture",
    "SetScoreAugmentation",
    "ControlAirLegExtension",
    "MatchTwistAndLean",
    "ApplyingBodyTilt",
    "SettingBodyTilt",
    "Crouching",
    "Pumping",
    "BipedCadence",
    "MatchCadence",
    "MatchAirTime",
    "OffboardBodyTweakBlend",
    "SetDistComToBoard",
    "ForcePhysics",
    "UpdateTimeSinceTeleport",
    "GrindControlFade",
    "ControlGrindCrouch",
    "CreateGrindAttributes",
    "SetDeckPitchAndYaw",
    "SetBumpCoefficients",
    "TweakProject",
    "UpdateIsWeightOnNose",
    "FootPlantAbsorb",
    "SetHandPlantAnticLength",
    "ScoringHandPlants",
    "BodySpin",
    "AirDismounting",
    "InitMovingObjects",
    "MovingObject",
    "UpdateStandingOnCar",
    "MaintainShove",
    "Shove",
    "HippyJumpAntic",
    "LandOnBoard",
    "StoreLandingData",
    "SetLandingData",
    "AddRunoutAttribs",
    "Wipeout",
    "EnableWipeoutGestures",
    "ToggleBoard",
    "ResetSkaterAnimation",
    "ResetToGivenStance",
    "EndShimmy",
    "FingerFlipOut",
    "PowerSlideDecel",
    "PowerSlideManualAtt",
    "PowerSlideSpin",
    "SetManualAngle",
    "SetDeckPitch",
];
/// Off-board and NPC behaviours flatground doesn't run (logged once as unsupported when reached).
const UNSUPPORTED: &[&str] = &[
    "EnterSkitchingBehaviour",
    "SkitchingBehaviour",
    "SkitchShimmyingBehaviour",
];

fn beh(a: &A, p: &Pack, g: &Graph, op: OpId, name: &str, db: Option<&MotionDb>) -> Beh {
    let lib = |t: &str| db.and_then(|d| d.name_id(t));
    let node = |k: &str| a.text(k).and_then(|t| db.and_then(|d| d.find(t)));
    match name {
        "PlayAnimation" => {
            let mut params = Vec::new();
            for attrs in g.op_params(op) {
                let pa = A { p, attrs };
                let (from, source) = match pa.text("from").unwrap_or("") {
                    "intent" => (0, pa.str("intent")),
                    "lastAnim" => (2, pa.str("attribute")),
                    _ => (1, pa.str("filteredIntent")),
                };
                let src = if source == NONE { "" } else { p.str(source) };
                let rename = pa.text("rename").filter(|r| !r.is_empty());
                params.push(ParamDef {
                    from,
                    source,
                    target: lib(rename.unwrap_or(src)),
                    source_name: lib(src),
                    default: pa.of("defaultValue"),
                    normalized: from != 2 && pa.b("normalize", false),
                });
            }
            Beh::Play(PlayDef {
                anim: node("anim"),
                switch_anim: node("switchAnim"),
                mirror_anim: node("mirrorAnim"),
                anim_name: a.str("anim"),
                speed: a.f("playBackSpeed", 1.0),
                transition: transition(a),
                params,
            })
        }
        "CreateAttribute" => {
            let (values, set) = if let Some(v) = a.of("always") {
                ([Some(v), Some(v), Some(0.0)], false)
            } else if ["begin", "update", "end"]
                .iter()
                .all(|k| a.get(k).is_none())
            {
                ([None, Some(0.0), None], false)
            } else {
                (
                    [a.of("begin"), a.of("update"), a.of("end")],
                    a.b("set", false),
                )
            };
            Beh::CreateAttribute {
                name: a.str("attName"),
                tree: a.text("attName").and_then(lib),
                values,
                set,
            }
        }
        "AttachIntent" => Beh::AttachIntent {
            intent: a.str("intent"),
            attr: a.str("attr"),
        },
        "FilterMotionGraphIntent" => {
            let blend = a.f("blend", 1.0);
            Beh::Filter(FilterDef {
                intent: a.str("intent"),
                filtered: a.str("filteredIntent"),
                starting: a.f("startingValue", 0.0),
                default: a.f("defaultValue", 0.0),
                scale: a.f("scale", 1.0),
                filters: [
                    filter(a.text("filter1").or_else(|| a.text("filter"))),
                    filter(a.text("filter2")),
                    filter(a.text("fakieFilter")),
                    filter(a.text("mirrorFilter")),
                ],
                ramp_time: a.of("rampTime"),
                rising: a.f("blendRising", blend),
                falling: a.f("blendFalling", blend),
                out: a.of("blendOut"),
                clamp_vel: a.of("clampVel"),
                clamp_acc: a.of("clampAcc"),
            })
        }
        "SetTrickHeight" => Beh::SetTrickHeight {
            from: a.text("from").and_then(lib),
            rename: a.text("rename").and_then(lib),
            value: a.of("setToValue"),
            manual: a.get("trickFromManual").is_some(),
            grind: a.get("trickFromGrind").is_some(),
        },
        "JumpInto" => Beh::JumpInto,
        "ScoringTrick" => Beh::ScoringTrick(a.str("trick")),
        "SetTrickAttr" => Beh::SetTrickAttr(a.str("trick")),
        "ClearTrickAttr" => Beh::ClearTrickAttr,
        "IsDoingTrick" => Beh::Flag(Flag::Trick),
        "IsManualing" => Beh::Flag(Flag::Manual),
        "IsPowerSliding" | "PowerSliding" => Beh::Flag(Flag::PowerSlide),
        "IsLanding" => Beh::Flag(Flag::Landing),
        "IsAnticipating" => Beh::Flag(Flag::Anticipating),
        "InCandidateSlidingState" => Beh::Flag(Flag::SlideCandidate),
        "DisallowPumping" => Beh::Flag(Flag::NoPumping),
        "SetDark" => Beh::SetDark,
        "MonitorUnderflip" => Beh::MonitorUnderflip,
        "ChooseRandomLanding" => Beh::ChooseRandomLanding(a.f("numlandings", 1.0).max(1.0) as u8),
        "KickTurnSteering" => Beh::KickTurnSteering {
            spin_scale: a.f("spinScale", 1.0),
            max_height: a.f("maxHeight", 0.0),
        },
        "SetManualOutTimer" => Beh::SetManualOutTimer(a.f("length", 0.1)),
        "UpdateManualOutTimer" => Beh::UpdateManualOutTimer,
        "DisableTricks" => Beh::DisableTricks(a.f("length", 0.3)),
        "UpdateRidingFakie" => Beh::UpdateRidingFakie,
        "ResetTimeSinceKickturn" => Beh::ResetTimeSinceKickturn,
        "UpdateTimeSinceKickturn" => Beh::UpdateTimeSinceKickturn,
        "SetGrabType" => Beh::SetGrabType(a.str("grab")),
        "ScoringGrabs" => Beh::ScoringGrabs,
        "InitPush" => Beh::Push(PushOp::Init),
        "PushCycle" => Beh::Push(PushOp::Cycle {
            foot: match a.get("newPushName") {
                Some(_) => a.str("newPushName"),
                None => p.find("LeftPush").unwrap_or(NONE),
            },
        }),
        "PushOut" => Beh::Push(PushOp::Out {
            right: a.b("isRightFoot", true),
        }),
        "ComputeFirstPushStrength" => Beh::Push(PushOp::FirstStrength),
        "SetPushCoefs" => Beh::Push(PushOp::Coefs {
            first_only: a.b("onFirstUpdateOnly", false),
        }),
        "ComputeRepushDeadline" => Beh::Push(PushOp::RepushDeadline {
            regular: a.b("regularAttributes", true),
        }),
        "ComputeTargetCoefsFromSpeedAndStrength" => Beh::Push(PushOp::TargetCoefs {
            regular: a.b("regularAttributes", true),
        }),
        "CreateSlide" => Beh::CreateSlide {
            right: a.b("right", true),
        },
        "SetSpeed" => Beh::SetSpeed {
            attr: a.text("attribute").and_then(lib),
            value: a.of("setToValue"),
        },
        "SetTurning" => Beh::SetTurning,
        "CreateMGIntentFromAGIntent" => Beh::MgFromAg {
            mg: a.str("MGIntent"),
            ag: a.str("AGIntent"),
            default: a.of("defaultValue"),
            scale: a.f("scale", 1.0),
            filters: [
                filter(a.text("filter1").or_else(|| a.text("filter"))),
                filter(a.text("filter2")),
                filter(a.text("fakieFilter")),
                filter(a.text("mirrorFilter")),
            ],
            on_update: a.b("onUpdate", true),
        },
        "CreateConstMGIntent" => Beh::MgConst {
            mg: a.str("MGIntent"),
            value: a.f("value", 0.0),
            on_update: a.b("onUpdate", true),
        },
        "CreateMGTimeIntentFromAGIntent" => Beh::MgTime {
            mg: a.str("MGIntent"),
            ag: a.str("AGIntent"),
        },
        "CreateTrickIntentFromGesture" => match Group::parse(a.text("group").unwrap_or("")) {
            Some(g) => Beh::TrickFromGesture(g),
            None => Beh::Nothing,
        },
        "BoardAdjust" => Beh::BoardAdjust {
            mag: a.str("MGIntentMag"),
            angle: a.str("MGIntentAngle"),
        },
        "BodyFlippingSignal" => Beh::BodyFlippingSignal,
        "FakieHeadChannel" => Beh::FakieHead,
        _ if NOTHING.contains(&name) => Beh::Nothing,
        _ if UNSUPPORTED.contains(&name) => Beh::Unsupported(NONE),
        _ => Beh::Unsupported(NONE),
    }
}

/// Parses every operation of a graph. `db`: the motion library, to resolve animation and attribute names.
pub fn parse(p: &Pack, g: &Graph, db: Option<&MotionDb>) -> Vec<Op> {
    (0..g.ops.len() as u16)
        .map(|op| {
            let o = &g.ops[op as usize];
            let a = A {
                p,
                attrs: g.op_attrs(op),
            };
            let name = p.str(o.name);
            match o.kind {
                OP_CONDITION => Op::Cond(match cond(&a, name, o.bound, db) {
                    Cond::Unsupported(_) => Cond::Unsupported(o.name),
                    c => c,
                }),
                OP_BEHAVIOUR => Op::Beh(match beh(&a, p, g, op, name, db) {
                    Beh::Unsupported(_) => Beh::Unsupported(o.name),
                    b => b,
                }),
                _ => Op::Hook(match name {
                    "OverideNextAnimTransitionHook" => Hook::Override(transition(&a)),
                    "GrabSlide" => Hook::GrabSlide {
                        right: a.b("right", true),
                    },
                    "MongoPushToAntic" => Hook::MongoPushToAntic,
                    _ => Hook::Unsupported(o.name),
                }),
            }
        })
        .collect()
}

/// (implemented, stubbed) counts and the names of unsupported operations, for the log.
pub fn coverage<'a>(p: &Pack<'a>, ops: &[Op]) -> (usize, usize, Vec<&'a str>) {
    let mut names: Vec<&str> = Vec::new();
    let mut fixed = 0;
    for o in ops {
        match o {
            Op::Cond(Cond::Unsupported(n))
            | Op::Beh(Beh::Unsupported(n))
            | Op::Hook(Hook::Unsupported(n)) => {
                let s = p.str(*n);
                if !names.contains(&s) {
                    names.push(s);
                }
            }
            Op::Cond(Cond::Fixed(_)) | Op::Beh(Beh::Nothing) => fixed += 1,
            _ => {}
        }
    }
    names.sort_unstable();
    let unsupported = ops
        .iter()
        .filter(|o| {
            matches!(
                o,
                Op::Cond(Cond::Unsupported(_))
                    | Op::Beh(Beh::Unsupported(_))
                    | Op::Hook(Hook::Unsupported(_))
            )
        })
        .count();
    (ops.len() - unsupported - fixed, fixed, names)
}
