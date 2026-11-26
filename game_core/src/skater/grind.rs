//! Grinds, as Skate 3 does them (after SK8-ENGINE/skate-3-rust-engine, GPL-3.0:
//! `physics/grind_contact{,/admission,/entry,/families,/investigator,/control,/manager,/balance}.rs`,
//! `physics/grind_forces{,/launch,/orientation,/support,/release}.rs`, `physics/grind_air/*`,
//! skate-game `physics/grind/contact.rs` and `physics/grind_chromosome{,/pose}.rs`).
//!
//! - **Contact** ([`Skater::investigate`]): the rail must cross one of the board's boxes: a box under
//!   each truck, one under the deck between the trucks, one past each truck under the tips, one over an
//!   upside-down deck. Which box it crosses picks the family: both trucks a 50-50, one truck a 5-0 (front
//!   or back), a tip a tip slide (a blunt when the board lies over a ledge's high side), the deck a
//!   boardslide (board flat on the rail, trucks clear of it), the upside-down deck a darkslide. The
//!   family the board is in is tried first.
//! - **Admission**: from the air any contact is taken; riding into a rail it depends on the approach:
//!   from above (within 17 degrees of the rail), from below or into coping (the speed across the rail
//!   under a limit that grows with how vertical the approach is), or dropping in (slow, on a tip or a
//!   truck). The velocity across the rail is turned along it (partly, by family, from the air; by
//!   `VertEngagementHelpVsVelY` riding into it). Too sharp a change, too fast across a thin rail or towards
//!   a ledge's high side, or a rail more than 60 degrees off the deck from the air, wipes out.
//! - **On the rail**: the board slides along the rail under gravity and friction (by family and rail
//!   class, growing over the grind: `FrictionVsTime`), and across it under Skate 3's balance forces
//!   ([`super::balance`]: the pins, the slides' and tip slides' stick pushes, the exit lean), and comes
//!   off the rail when the rail leaves its box (or wipes out on the closing speed); riding into coping
//!   holds the board up for a while (`TimeOfGravityRelief...`),
//!   the stick's pitch tips a 50-50 into a 5-0 or nosegrind and lifts or drops the free end, the sticks'
//!   x turn a 5-0 across the rail (crooked, overcrook ...); tip slides lean 20 degrees, blunts 42; on
//!   ledges (and tip slides on anything but a thin rail) the board leans off after a while
//!   (`ExitLeanAngleVsTime`) and is pushed off the low side (`ExitAssistVsLeanAngle`).
//! - **Leaving**: off the rail's end (grinds get a little lift), leaning off, too slow, or popped: the
//!   launch leans with the family (tip slides outwards, blunts inwards), the stick steers it sideways, and
//!   each pop costs energy that comes back slowly; another grind needs a moment in the air.
//! - **Names** ([`Naming`]): FS or BS from where the rider was at take-off (or half a second ago on the
//!   ground) against the rail and which way the chest faced, the truck or tip on the rail, straight or
//!   angled, low (the free end dipped), which way the rider faces, and the family give Skate 3's grind
//!   names (the state graph's `IsGrinding` values, `BF_` variants included); a new name counts for the
//!   score once it has held 12 ticks.
//!
//! Where Skate 3's value is a force on its rigid board, flatground turns it into an acceleration with an
//! effective mass of its own ([`GRIND_MASS`]).

use super::balance;
use super::*;
use crate::rails::{Rail, RailClass, upright};

/// The effective mass (kg, ours) Skate 3's grind forces are divided by. Chosen so a thin-rail 50-50 at
/// entry slows by about 0.8 m/s^2 (flatground's grind friction before), with every family and rail class
/// keeping Skate 3's ratios.
pub const GRIND_MASS: f32 = 25.0;
/// SK8 grind_forces.rs: friction = 0.48 x the rail's up y x strength x `FrictionVsTime` x material.
const FRICTION_SCALE: f32 = 0.48;
/// SK8 `material_multiplier`'s default (unflagged concrete and metal).
const MATERIAL: f32 = 1.3;
/// Friction strengths [thin, fat, ledge] per family (SK8 skate-game grind/contact.rs).
const STRENGTH_GRIND: [f32; 3] = [50.0, 40.0, 40.0];
const STRENGTH_SLIDE: [f32; 3] = [55.0, 55.0, 60.0];
const STRENGTH_TIP: [f32; 3] = [60.0, 60.0, 60.0];
const STRENGTH_BLUNT: [f32; 3] = [70.0, 80.0, 80.0];
/// Tip slides lean 20 degrees about the rail, blunts 42 (SK8 orientation.rs).
const TIP_LEAN: f32 = 0.349_066;
const BLUNT_LEAN: f32 = 0.733_038;
/// The board's frame turns this share of the way to its target each tick (SK8 `blend`, 0.1).
const ORIENT_BLEND: f32 = 0.1;
/// A tip's box reaches this far past the truck, and that much deeper at its far end (SK8 families.rs).
const TIP_LENGTH: f32 = 0.215;
const TIP_RISE: f32 = 0.03;
/// The upside-down deck's box is this long each way (SK8 `inverted_contact`).
const DARK_HALF: f32 = 0.3;
/// Truck boxes start this far under the deck and reach this far down (SK8 `truck_contacts`).
const TRUCK_TOP: f32 = 0.02;
const BOX_DEPTH: f32 = 0.2;
/// A 50-50 needs the sticks' x under 0.9. Skate 3 also wants the rail within a few centimetres of the
/// deck (its rigid deck comes to rest on the rail); flatground's boxes alone bound the depth, as the
/// board is put on the rail once it locks on.
const FIFTY_BALANCE: f32 = 0.9;
/// 5-0: the rail not too far across or up the deck.
const FIVE_RIGHT: f32 = 0.906;
const FIVE_UP: f32 = 0.46;
/// Dropping in (slowly riding onto a rail), the deck's centre within this far of it along the deck's
/// up (m), SK8 families.rs's depth windows: a 5-0 (0.13; `0x3c8a71de` squared), and a tip (0.06;
/// `0x3b6bedfa`). Rolling slowly at a coping on the deck it sits on, the deck is a deck's height
/// above it: the board rolls off into the transition rather than stopping on it, as in Skate 3. (Other
/// entries keep the boxes alone: flatground's board doesn't pivot onto a rail as Skate 3's rigid one
/// does, so its deck wouldn't come within the windows.)
const FIVE_DEPTH: f32 = 0.13;
const TIP_DEPTH: f32 = 0.06;
/// Boardslides: the rail's up against the deck's (cosine), and the trucks clear of the rail by more
/// than a truck's half width; darkslides: the upside-down deck against it.
const BOARD_UP: f32 = 0.65;
const DARK_UP: f32 = 0.45;
/// Riding onto a rail: from above within this many degrees of it; slower than this is dropping in.
const FROM_ABOVE_DEG: f32 = 17.0;
const DROP_IN_SPEED: f32 = 1.45;
/// Dropping in onto a tip needs the board moving at least this fast towards it (m/s, ours).
const DROP_IN_MIN: f32 = 0.3;
/// The approach's slope sine above which it is riding into coping, or from below.
const INTO_COPING: f32 = 0.96;
const FROM_BELOW: f32 = 0.25;
/// Cross-rail speed limit riding into a rail: base, plus this much more the more of the speed is along it.
const CROSS_LIMIT: f32 = 2.8;
const CROSS_LIMIT_ALONG: f32 = 2.4;
/// Changing family on the rail stays within 24 degrees of it.
const CHANGE_DEG: f32 = 24.0;
/// A rail more than 60 degrees off the deck (its up against the rail, cosine 0.5) isn't taken.
const ENTRY_STEEP: f32 = 0.5;
/// Crossing a thin rail (or towards a ledge's high side) faster than this wipes out (m/s, SK8 49
/// squared).
const CROSS_WIPEOUT: f32 = 7.0;
/// On a fat rail or ledge, moving towards the low side faster than this across it doesn't take
/// (m/s, SK8 12.25 squared).
const LOW_SIDE_SPEED: f32 = 3.5;
/// Coping hold starts when the approach is this vertical (SK8 manager.rs).
const RELIEF_SLOPE: f32 = 0.85;
/// The stick's pitch: smoothed (0.91 kept a tick, 0.76 when the 5-0 rule changes it), a 50-50 tips over
/// past 0.12, the pitch turns the board 0.68 rad per unit (SK8 control.rs), applied as a turn of 0.1 of
/// that a tick (the frame blend).
const PITCH_KEEP: f32 = 0.91;
const PITCH_KEEP_MODIFIED: f32 = 0.76;
const PITCH_TIPS: f32 = 0.12;
const PITCH_RATE: f32 = 0.68 * ORIENT_BLEND;
/// 5-0s: the free end is held up at 0.71 of the stick with the stick let go; it doesn't lift past 0.26
/// rad, and is pushed back up below -0.25 (SK8 control.rs).
const FREE_END_HOLD: f32 = 0.71;
const FREE_END_MAX: f32 = 0.26;
const FREE_END_MIN: f32 = -0.25;
/// 5-0s: the sticks' x turn the board (dead zone 0.25, 0.85 of the stick plus 0.79 of the main stick,
/// cubed, smoothed by 0.91 a tick).
const YAW_DEAD: f32 = 0.25;
const YAW_STICKS: f32 = 0.85;
const YAW_NUDGE: f32 = 0.79;
/// A 5-0 within this of level (rad) and turned less than `FIFTY_TWIST` off the rail, with the other
/// truck over the rail, has both trucks on again.
const FIFTY_AGAIN: f32 = 0.15;
const FIFTY_TWIST: f32 = 0.2;
/// How far a 5-0 can turn off the rail (rad) before it isn't one (SK8 `FIVE_RIGHT`, 65 degrees).
const TWIST_MAX: f32 = 1.13;
/// The most a 5-0's free end tips (rad, ours: the deck meets the rail).
const TILT_MAX: f32 = 0.5;
/// A name is scored once it has held this many ticks (SK8 grind_chromosome.rs).
const NAME_STABLE: u32 = 12;
/// Lean-off: the delay before it starts (s): 2.0, 0.8 entered from below, 0.1 riding into coping; forced
/// off past 28 degrees, or past 15 with no ground under the low side (SK8 balance.rs).
const LEAN_DELAY: f32 = 2.0;
const LEAN_DELAY_BELOW: f32 = 0.8;
const LEAN_DELAY_COPING: f32 = 0.1;
const LEAN_FORCED: f32 = 28.0;
const LEAN_FORCED_NO_GROUND: f32 = 15.0;
/// Leaving by leaning off or stopping: at least this much speed off the low side (m/s, ours).
const EXIT_MIN_PUSH: f32 = 0.6;
/// Slower than this along the rail for this long (and not held on coping) steps off (ours).
const STOP_SPEED: f32 = 0.2;
const STOP_TIME: f32 = 0.4;
/// Off the rail's end a grind (not a slide) lifts a little (m/s, ours: SK8's release nudge strength
/// comes from its caller).
const RELEASE_LIFT: f32 = 0.4;
/// Popping out (SK8 launch.rs): the jump x 1.03; tip slides lean the launch 0.291 outwards, blunts
/// 0.1093 inwards; sideways up to 1.8 m/s from the stick, minus the high side's push; each pop costs 0.6
/// energy, which comes back by 0.0035 a tick; under 0.31 the pop goes off to a side (0.42) and grind
/// targeting waits 10 ticks.
const LAUNCH_JUMP: f32 = 1.03;
const LAUNCH_TIP: f32 = 0.291;
const LAUNCH_BLUNT: f32 = -0.1093;
const LAUNCH_SIDE: f32 = 1.8;
const POP_ENERGY: f32 = 0.6;
const ENERGY_BACK: f32 = 0.0035;
const LOW_ENERGY: f32 = 0.31;
const LOW_ENERGY_SIDE: f32 = 0.42;
const LOW_ENERGY_TICKS: u32 = 10;
/// Another grind needs this long in the air and more than this many ticks since the last grind jump
/// (any rail; SK8 selector/air.rs).
const REGRIND_AIR: f32 = 0.08;
const REGRIND_TICKS: u32 = 10;
/// The rail just left without a pop (off its end, leaned off) can't be taken again for this long (s,
/// ours).
const LEAN_OFF_BLOCK: f32 = 0.5;
/// Riding samples kept for the approach (ticks): the pose half a second before a grind from riding.
const HISTORY: usize = 30;
/// Straight: the deck within 20 degrees of the rail (cosine); low: the free end this far under the
/// rail's plane (SK8 pose.rs).
const STRAIGHT_COS: f32 = 0.9397;
const LOW_DOT: f32 = -0.1;
/// A grind shorter than this isn't scored.
const GRIND_MIN_TIME: f32 = 0.15;
/// A grind grab is named with the grind once held this long (s: the score's 12 ticks, ours).
const GRAB_MIN_TIME: f32 = 0.2;
/// Rails near the board looked at for contact (more than this many near at once are rare).
const NEAR_RAILS: usize = 16;

/// Skate 3's grind families (SK8 `Family`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    FiftyFifty,
    Boardslide,
    Tipslide,
    FiveO,
    /// A tip slide over a ledge's high side: a blunt.
    Backslash,
    Darkslide,
}

impl Family {
    /// The deck (not the trucks) is on the rail.
    pub fn is_slide(self) -> bool {
        !matches!(self, Family::FiftyFifty | Family::FiveO)
    }

    fn strengths(self) -> [f32; 3] {
        match self {
            Family::FiftyFifty | Family::FiveO => STRENGTH_GRIND,
            Family::Boardslide | Family::Darkslide => STRENGTH_SLIDE,
            Family::Tipslide => STRENGTH_TIP,
            Family::Backslash => STRENGTH_BLUNT,
        }
    }

    /// How much of the cross-rail speed an air entry takes out (SK8 `airborne_velocity`).
    fn air_entry(self) -> f32 {
        match self {
            Family::FiftyFifty | Family::FiveO => 0.4,
            Family::Boardslide | Family::Darkslide => 0.2,
            Family::Tipslide | Family::Backslash => 0.8,
        }
    }

    /// The pop's speed along the launch normal for a jump of `strength` (0..1; m/s, before the launch's
    /// 1.03): the physics mode's low and high ends for the family's group, blended, unclamped (SK8
    /// grind/substate/settings.rs `vertical`: the 50-50s, 5-0s, backslashes and darkslides, the
    /// boardslides, the nose and tail slides).
    fn pop(self, t: &crate::tuning::grind::GrindTuning, strength: f32) -> f32 {
        let group = match self {
            Family::Boardslide => 1,
            Family::Tipslide => 2,
            _ => 0,
        };
        let [low, high] = t.pop[group];
        (1.0 - strength) * low + high * strength
    }

    /// The pop's sideways push off a ledge's high side (SK8 substate settings `side`).
    fn side_jump(self, class: RailClass) -> f32 {
        match self {
            Family::Boardslide | Family::Darkslide if class.is_ledge() => 0.9,
            Family::Boardslide | Family::Darkslide => 0.0,
            Family::Backslash => 0.9,
            _ => 0.5,
        }
    }
}

/// How the board came onto the rail (SK8 `EntryKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    RideFromAbove,
    RideFromBelow,
    RideIntoCoping,
    StayInGrind,
    ChangeGrind,
    AirToGrind,
    DropIn,
}

/// Where the board is (SK8's physics categories: 100 ground, 200 air, 400 grind).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Ground,
    Air,
    Grind,
}

/// The board's frame: right (`up x fwd`), up, nose, and the deck's middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub right: Vec3,
    pub up: Vec3,
    pub fwd: Vec3,
    pub deck: Vec3,
}

/// Where the rail crosses one of the board's boxes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hit {
    rail: usize,
    seg: usize,
    point: Vec3,
}

/// A contact the investigation accepted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub family: Family,
    pub rail: usize,
    pub seg: usize,
    pub point: Vec3,
    /// The segment's direction (towards its end).
    pub dir: Vec3,
    /// 5-0: the front truck; tip slides and blunts: the nose.
    pub front: bool,
    pub entry: Entry,
}

/// The lean-off (SK8 `BalanceState`): seconds since it started, the delay, the angle (degrees), the side
/// it leans away from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lean {
    pub elapsed: f32,
    pub delay: f32,
    pub angle: f32,
}

/// A grind in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Grind {
    /// Index into the terrain's rails.
    pub rail: usize,
    /// Distance along the rail, and the point there.
    pub s: f32,
    pub point: Vec3,
    /// +1 travelling towards the rail's end, -1 towards its start.
    pub dir: f32,
    /// Speed along the travel (m/s, 0 or more).
    pub speed: f32,
    pub family: Family,
    /// 5-0s: on the front truck (nosegrinds, crooks, willys); tips: on the nose.
    pub front: bool,
    pub entry: Entry,
    pub class: RailClass,
    /// The rail's high side (horizontal unit; meaningful on ledges).
    pub high: Vec3,
    /// Where along the board the rail is (m from the middle, + towards the nose).
    pub along: f32,
    pub time: f32,
    /// The trick that took the skater onto the rail (named and scored with the grind).
    pub into: Option<(String, crate::scoring::Air)>,
    /// The stick's smoothed pitch and the sticks' yaw (SK8 `Control`).
    pub pitch: f32,
    pub yaw: f32,
    /// The board's tip (rad, + nose down) and turn off the rail (rad), for 5-0s.
    pub tilt: f32,
    pub twist: f32,
    /// Seconds of coping hold left.
    pub relief: f32,
    pub lean: Lean,
    /// Across the rail (SK8's rigid board, [`super::balance`]): how far the family's reference point
    /// (the deck's middle, the truck on the rail, the tip) is off the rail along the segment's across
    /// axis (`dir x up`, m), and how fast it moves (m/s).
    pub lateral: f32,
    pub lateral_vel: f32,
    /// Ticks left of the tip slide's pull in after riding fast onto the tip.
    pub tip_hold: u32,
    /// A pop was wound up on a slide (sticky for the grind, SK8 `preparing_jump`).
    pub preparing: bool,
    /// The closing speed the rail stopped this tick (m/s, across the rider's ground plane).
    pub closing: f32,
    /// The grind grab held now (`FSGRAB`, `NOSEGRAB` ...) and for how long, and the longest one held
    /// in this grind (scored with it).
    pub grab: Option<(&'static str, f32)>,
    pub grabbed: Option<(&'static str, f32)>,
    /// Seconds slower than the stop speed.
    pub slow: f32,
    pub naming: Naming,
    /// The approach was backside (the chest away from the rail) and the rider is turned round from it.
    pub backside: bool,
    pub reversed: bool,
    /// The rail is a ramp's coping (`skater::stall::is_coping`), and the fastest the board went along it
    /// (m/s): held still on a coping, the grind is a stall.
    pub coping: bool,
    pub top_speed: f32,
}

impl Grind {
    /// The name the state graph's `IsGrinding` knows (`FS_50_50`, `BF_BS_CROOK` ...).
    pub fn graph_name(&self) -> &'static str {
        self.naming.anim.unwrap_or("BS_50_50")
    }

    /// The name scored (after it held [`NAME_STABLE`] ticks), as shown: `FS 50-50`, `BS CROOKED GRIND`;
    /// with the grind grab held longest, `FS 5-0 NOSEGRAB`.
    pub fn display(&self) -> String {
        let name = display_name(
            self.naming
                .scoring
                .or(self.naming.anim)
                .unwrap_or("BS_50_50"),
        );
        match self.grabbed {
            Some((grab, t)) if t >= GRAB_MIN_TIME => {
                format!("{name} {}", balance::grab_display(grab))
            }
            _ => name,
        }
    }

    /// Skate 3's trick id for the grind as scored (`fs_50_50`), with the grind grab held longest when it
    /// has one (`fs_5_o_nosegrab`).
    pub fn scorable(&self) -> Option<u16> {
        use crate::scoring::catalog::{id, id_of};
        let name = self
            .naming
            .scoring
            .or(self.naming.anim)
            .unwrap_or("BS_50_50");
        let base = id(name).or_else(|| name.strip_prefix("BF_").and_then(id))?;
        let grab = self
            .grabbed
            .filter(|(_, t)| *t >= GRAB_MIN_TIME)
            .and_then(|(grab, _)| id_of(&[crate::scoring::catalog::name(base), "_", grab]));
        Some(grab.unwrap_or(base))
    }

    /// The graph name's grind without side or `BF_` (`5_O`, `CROOK` ...).
    pub fn base(&self) -> &'static str {
        let n = self.graph_name();
        let n = n.strip_prefix("BF_").unwrap_or(n);
        n.get(3..).unwrap_or(n)
    }

    /// Frontside, from the name.
    pub fn frontside(&self) -> bool {
        self.graph_name()
            .trim_start_matches("BF_")
            .starts_with("FS_")
    }

    /// The name's grind, without side.
    pub fn kind(&self) -> GrindKind {
        GrindKind::of(self.graph_name())
    }

    /// The gesture group a pop out of it reads flicks with (the state graphs' groups: the back truck's
    /// grinds and tail slides the tail, the front truck's and nose slides the nose, the rest square).
    pub fn group(&self) -> Group {
        match self.family {
            Family::FiveO | Family::Tipslide | Family::Backslash if self.front => Group::Nose,
            Family::FiveO | Family::Tipslide | Family::Backslash => Group::Tail,
            _ => Group::Square,
        }
    }
}

/// The grind's look, for the built-in rider and the score (the graph name without its side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrindKind {
    FiftyFifty,
    FiveO,
    Nosegrind,
    Crooked,
    Overcrook,
    Willy,
    Overwilly,
    Salad,
    Feeble,
    Smith,
    Boardslide,
    Lipslide,
    Noseslide,
    Tailslide,
    Noseblunt,
    Bluntslide,
    Darkslide,
}

impl GrindKind {
    pub fn of(graph_name: &str) -> GrindKind {
        let base = graph_name
            .trim_start_matches("BF_")
            .trim_start_matches("FS_")
            .trim_start_matches("BS_");
        match base {
            "5_O" => GrindKind::FiveO,
            "NOSEGRIND" => GrindKind::Nosegrind,
            "CROOK" => GrindKind::Crooked,
            "OVERCROOK" => GrindKind::Overcrook,
            "WILLY" => GrindKind::Willy,
            "OVERWILLY" => GrindKind::Overwilly,
            "SALAD" => GrindKind::Salad,
            "FEEBLE" => GrindKind::Feeble,
            "SMITH" => GrindKind::Smith,
            "BOARD" => GrindKind::Boardslide,
            "LIP" => GrindKind::Lipslide,
            "NOSESLIDE" => GrindKind::Noseslide,
            "TAILSLIDE" => GrindKind::Tailslide,
            "NOSEBLUNT" => GrindKind::Noseblunt,
            "BLUNT" => GrindKind::Bluntslide,
            "DARKSLIDE" => GrindKind::Darkslide,
            _ => GrindKind::FiftyFifty,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            GrindKind::FiftyFifty => "50-50",
            GrindKind::FiveO => "5-0",
            GrindKind::Nosegrind => "NOSEGRIND",
            GrindKind::Crooked => "CROOKED GRIND",
            GrindKind::Overcrook => "OVERCROOK",
            GrindKind::Willy => "WILLY GRIND",
            GrindKind::Overwilly => "OVERWILLY",
            GrindKind::Salad => "SALAD GRIND",
            GrindKind::Feeble => "FEEBLE GRIND",
            GrindKind::Smith => "SMITH GRIND",
            GrindKind::Boardslide => "BOARDSLIDE",
            GrindKind::Lipslide => "LIPSLIDE",
            GrindKind::Noseslide => "NOSESLIDE",
            GrindKind::Tailslide => "TAILSLIDE",
            GrindKind::Noseblunt => "NOSEBLUNT",
            GrindKind::Bluntslide => "BLUNTSLIDE",
            GrindKind::Darkslide => "DARKSLIDE",
        }
    }

    /// The deck (not the trucks) is on the rail.
    pub fn is_slide(self) -> bool {
        matches!(
            self,
            GrindKind::Boardslide
                | GrindKind::Lipslide
                | GrindKind::Noseslide
                | GrindKind::Tailslide
                | GrindKind::Noseblunt
                | GrindKind::Bluntslide
                | GrindKind::Darkslide
        )
    }
}

/// Every grind name the physics publishes (the state graph's `IsGrinding` values).
pub const GRAPH_NAMES: &[&str] = &[
    "FS_50_50",
    "BS_50_50",
    "BF_FS_50_50",
    "BF_BS_50_50",
    "FS_BOARD",
    "BS_BOARD",
    "FS_LIP",
    "BS_LIP",
    "FS_NOSESLIDE",
    "BS_NOSESLIDE",
    "FS_TAILSLIDE",
    "BS_TAILSLIDE",
    "FS_NOSEBLUNT",
    "BS_NOSEBLUNT",
    "FS_BLUNT",
    "BS_BLUNT",
    "FS_DARKSLIDE",
    "BS_DARKSLIDE",
    "FS_5_O",
    "BS_5_O",
    "BF_FS_5_O",
    "BF_BS_5_O",
    "FS_NOSEGRIND",
    "BS_NOSEGRIND",
    "BF_FS_NOSEGRIND",
    "BF_BS_NOSEGRIND",
    "FS_CROOK",
    "BS_CROOK",
    "BF_FS_CROOK",
    "BF_BS_CROOK",
    "FS_OVERCROOK",
    "BS_OVERCROOK",
    "BF_FS_OVERCROOK",
    "BF_BS_OVERCROOK",
    "FS_WILLY",
    "BS_WILLY",
    "BF_FS_WILLY",
    "BF_BS_WILLY",
    "FS_OVERWILLY",
    "BS_OVERWILLY",
    "BF_FS_OVERWILLY",
    "BF_BS_OVERWILLY",
    "FS_SALAD",
    "BS_SALAD",
    "BF_FS_SALAD",
    "BF_BS_SALAD",
    "FS_SMITH",
    "BS_SMITH",
    "BF_FS_SMITH",
    "BF_BS_SMITH",
    "FS_FEEBLE",
    "BS_FEEBLE",
    "BF_FS_FEEBLE",
    "BF_BS_FEEBLE",
];

/// The graph name with this side, `BF_` variant and base (one of [`GRAPH_NAMES`]).
fn graph_name(bf: bool, backside: bool, base: &str) -> &'static str {
    let side = if backside { "BS_" } else { "FS_" };
    GRAPH_NAMES
        .iter()
        .find(|n| {
            let rest = if bf {
                n.strip_prefix("BF_")
            } else if n.starts_with("BF_") {
                None
            } else {
                Some(**n)
            };
            rest.and_then(|r| r.strip_prefix(side)) == Some(base)
        })
        .copied()
        .unwrap_or("BS_50_50")
}

/// What a grind's six features name (SK8 grind_names, as flatground reads its pattern): the approach
/// (backside), the truck or tip on the rail (back), straight, low, the rider turned round from the
/// approach (reversed) and facing along the travel (forward), and the family.
pub fn name_of(
    backside: bool,
    back: bool,
    straight: bool,
    low: bool,
    reversed: bool,
    forward: bool,
    family: Family,
) -> &'static str {
    // The side and the facing flip together: one of them alone turns the name over.
    let turned = backside != forward;
    match family {
        Family::FiftyFifty => graph_name(reversed, backside, "50_50"),
        Family::Boardslide => graph_name(false, backside, if turned { "LIP" } else { "BOARD" }),
        Family::Tipslide => {
            let base = match (back, turned) {
                (false, false) => "NOSESLIDE",
                (false, true) => "NOSEBLUNT",
                (true, false) => "BLUNT",
                (true, true) => "TAILSLIDE",
            };
            graph_name(false, backside, base)
        }
        Family::Backslash => {
            // Always a blunt; the side comes from which way the rider faces.
            let base = if back { "BLUNT" } else { "NOSEBLUNT" };
            graph_name(false, back == forward, base)
        }
        Family::Darkslide => graph_name(false, back == forward, "DARKSLIDE"),
        Family::FiveO => {
            let base = match (back, straight, low) {
                (false, true, false) => "NOSEGRIND",
                (false, _, true) => {
                    if turned {
                        "OVERWILLY"
                    } else {
                        "WILLY"
                    }
                }
                (false, false, false) => {
                    if turned {
                        "OVERCROOK"
                    } else {
                        "CROOK"
                    }
                }
                (true, true, false) => "5_O",
                (true, _, true) => {
                    if turned {
                        "SMITH"
                    } else {
                        "FEEBLE"
                    }
                }
                (true, false, false) => {
                    if turned {
                        "5_O"
                    } else {
                        "SALAD"
                    }
                }
            };
            graph_name(reversed, backside, base)
        }
    }
}

/// How a graph name is shown: `FS 50-50`, `BS CROOKED GRIND` (the `BF_` variants show as their grind).
pub fn display_name(graph: &str) -> String {
    let side = if graph.trim_start_matches("BF_").starts_with("FS_") {
        "FS"
    } else {
        "BS"
    };
    format!("{side} {}", GrindKind::of(graph).name())
}

/// The grind's names as they settle (SK8 grind_chromosome.rs): the latest, what the animation shows
/// (from the next tick) and what scores (once held 12 ticks).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Naming {
    pending: Option<&'static str>,
    frames: u32,
    pub anim: Option<&'static str>,
    pub scoring: Option<&'static str>,
}

impl Naming {
    fn update(&mut self, name: &'static str, first: bool) {
        if first {
            // Entering: both at once.
            self.pending = Some(name);
            self.frames = NAME_STABLE + 1;
        } else if self.pending == Some(name) {
            self.frames += 1;
        } else {
            self.pending = Some(name);
            self.frames = 0;
        }
        if self.frames > 0 {
            self.anim = self.pending;
        }
        if self.frames > NAME_STABLE {
            self.scoring = self.pending;
        }
    }
}

/// A pose remembered for the approach: where the board was, which way the chest faced, riding fakie.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ApproachPose {
    pub pos: Vec3,
    pub chest: Vec3,
    pub fakie: bool,
}

/// The approach's memory (SK8 chromosome history): the pose at take-off, the last half second of riding,
/// and how long since the last grind.
#[derive(Clone, Debug, PartialEq)]
pub struct Approach {
    saved: ApproachPose,
    history: [ApproachPose; HISTORY],
    count: usize,
    at: usize,
    /// Ticks since the board was last on a rail.
    pub away: u32,
}

impl Default for Approach {
    fn default() -> Self {
        Approach {
            saved: ApproachPose::default(),
            history: [ApproachPose::default(); HISTORY],
            count: 0,
            at: 0,
            away: 1000,
        }
    }
}

impl Approach {
    /// Leaving the ground: the take-off pose is the reference, the riding history starts over.
    pub fn take_off(&mut self, pose: ApproachPose) {
        self.saved = pose;
        self.count = 0;
    }

    /// A tick of riding.
    pub fn ride(&mut self, pose: ApproachPose) {
        self.history[self.at] = pose;
        self.at = (self.at + 1) % HISTORY;
        self.count = (self.count + 1).min(HISTORY);
    }

    /// The pose to name the approach from: half a second of riding ago, else the take-off.
    pub fn reference(&self) -> ApproachPose {
        if self.count == HISTORY {
            self.history[self.at]
        } else {
            self.saved
        }
    }
}

/// The grind-out energy and cooldown (SK8 `Jumper`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Jumper {
    pub energy: f32,
    pub cooldown: u32,
    /// Ticks since the last pop out of a grind.
    pub since_jump: u32,
}

impl Default for Jumper {
    fn default() -> Self {
        Jumper {
            energy: 1.0,
            cooldown: 0,
            since_jump: 1000,
        }
    }
}

impl Jumper {
    pub fn tick(&mut self) {
        self.cooldown = self.cooldown.saturating_sub(1);
        self.energy = (self.energy + ENERGY_BACK).min(1.0);
        self.since_jump = self.since_jump.saturating_add(1);
    }
}

/// Where segment `p0..p1` crosses the parallelogram at `corner` spanned by `e1` and `e2`, if it does.
pub fn segment_box(p0: Vec3, p1: Vec3, corner: Vec3, e1: Vec3, e2: Vec3) -> Option<Vec3> {
    let n = e1.cross(e2);
    let d = p1 - p0;
    let denom = d.dot(n);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = (corner - p0).dot(n) / denom;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let q = p0 + d * t;
    let w = q - corner;
    let (a, b, c) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
    let det = a * c - b * b;
    if det.abs() < 1e-12 {
        return None;
    }
    let (r1, r2) = (w.dot(e1), w.dot(e2));
    let u = (r1 * c - r2 * b) / det;
    let v = (r2 * a - r1 * b) / det;
    ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some(q)
}

/// The slope sine of an approach (SK8 `approach_slope_sine`): the vertical part of the unit velocity
/// across the rail.
fn slope_sine(dir: Vec3, v: Vec3, dead: f32) -> f32 {
    let across = v - dir * v.dot(dir);
    let len = across.length();
    if len <= dead { 0.0 } else { across.y / len }
}

/// The velocity's direction within `deg` degrees of the rail (either way), seen in the plane across the
/// rail's up (SK8 `within_approach_angle`).
fn within(dir: Vec3, v: Vec3, deg: f32) -> bool {
    let n = upright(dir);
    let p = v - n * v.dot(n);
    let len = p.length();
    len <= 0.001 || (p * (1.0 / len)).dot(dir).abs() > cosf(deg * PI / 180.0)
}

impl Skater {
    /// The board's frame: right, up, nose and the deck's middle. In the air the deck is turned and tipped
    /// against the body by the board adjust (`skater::adjust`) about its middle: the boxes the rails are
    /// looked for in, and the grind air assist's points, are the adjusted deck's (Skate 3's physical
    /// board follows its posed one).
    pub fn frame(&self) -> Frame {
        let body_up = self.normal.normalize_or(Vec3::UP);
        let body_fwd = tangent(self.fwd, body_up);
        let (fwd, up) = if self.deck_adjust.is_none() {
            (body_fwd, body_up)
        } else {
            self.deck_adjust.apply(body_fwd, body_up)
        };
        Frame {
            right: up.cross(fwd),
            up,
            fwd,
            // In the air the legs hold the deck up off the flight's path.
            deck: self.pos + body_up * DECK_HEIGHT + Vec3::UP * self.air_lift,
        }
    }

    /// The direction the rider's chest faces (the toe side).
    pub fn chest(&self) -> Vec3 {
        let f = self.frame();
        if self.mirrored() { -f.right } else { f.right }
    }

    /// The pose the approach is named from.
    pub(super) fn approach_pose(&self) -> ApproachPose {
        ApproachPose {
            pos: self.pos,
            chest: self.chest(),
            fakie: self.fakie,
        }
    }

    /// Admission (SK8 admission.rs `test`): how the board comes onto the rail and whether it may.
    fn admit(
        &self,
        category: Category,
        current: Option<Family>,
        requested: Family,
        dir: Vec3,
        drop_in: bool,
    ) -> Option<Entry> {
        let v = self.vel;
        let speed = v.length();
        let t = &self.tuning.grind;
        let kind = match category {
            Category::Grind => {
                let same = current == Some(requested)
                    || (requested == Family::Tipslide && current == Some(Family::Backslash));
                if same {
                    Entry::StayInGrind
                } else {
                    Entry::ChangeGrind
                }
            }
            Category::Ground if drop_in && speed < DROP_IN_SPEED => Entry::DropIn,
            Category::Ground => {
                let slope = slope_sine(dir, v, 0.01);
                if slope > INTO_COPING {
                    Entry::RideIntoCoping
                } else if slope > FROM_BELOW {
                    Entry::RideFromBelow
                } else {
                    Entry::RideFromAbove
                }
            }
            Category::Air => Entry::AirToGrind,
        };
        let ok = match kind {
            Entry::RideFromAbove => within(dir, v, FROM_ABOVE_DEG),
            Entry::StayInGrind => true,
            Entry::ChangeGrind => within(dir, v, CHANGE_DEG),
            Entry::AirToGrind | Entry::DropIn => true,
            Entry::RideFromBelow | Entry::RideIntoCoping => {
                let sq = v.dot(v);
                let across = dir * v.dot(dir) - v;
                let ratio = if sq <= 0.002 {
                    1.0
                } else {
                    sqrtf(across.dot(across) / sq)
                };
                let slope = slope_sine(dir, v, 0.01);
                let limit = (CROSS_LIMIT + CROSS_LIMIT_ALONG * (1.0 - ratio))
                    * t.entry_vs_slope.eval(slope);
                across.length() < limit
            }
        };
        ok.then_some(kind)
    }

    /// Looks for a rail crossing the board's boxes and picks the family as Skate 3 does (SK8
    /// investigator.rs, families.rs, manager.rs `tweak_geometry`). `current`: the family on the rail now
    /// (tried first).
    pub fn investigate(
        &self,
        park: &dyn Terrain,
        category: Category,
        current: Option<Family>,
        sticks_x: f32,
    ) -> Option<Contact> {
        let t = self.tuning.grind;
        let f = self.frame();
        let rails = park.rails();
        // No allocation: this runs every tick in the air and riding.
        let mut near_buf = [0usize; NEAR_RAILS];
        let mut count = 0;
        for (i, r) in rails.iter().enumerate() {
            if count < NEAR_RAILS && r.near(f.deck, 0.8) {
                near_buf[count] = i;
                count += 1;
            }
        }
        if count == 0 {
            return None;
        }
        let near = &near_buf[..count];
        let td = t.deck_to_truck;
        // The nearest crossing of a box (to `reference`) over every nearby segment.
        let cross_box = |corner: Vec3, e1: Vec3, e2: Vec3, reference: Vec3| {
            let mut best: Option<(f32, Hit)> = None;
            for &ri in near {
                let r = &rails[ri];
                for (si, w) in r.points.windows(2).enumerate() {
                    if r.classes
                        .get(si)
                        .is_some_and(|c| c.class == RailClass::Blocked)
                    {
                        continue;
                    }
                    if let Some(p) = segment_box(w[0], w[1], corner, e1, e2) {
                        let d = (p - reference).length();
                        if best.is_none_or(|b| d < b.0) {
                            best = Some((
                                d,
                                Hit {
                                    rail: ri,
                                    seg: si,
                                    point: p,
                                },
                            ));
                        }
                    }
                }
            }
            best.map(|b| b.1)
        };
        let dark = self.dark;
        // Truck boxes: across the board under each truck.
        let trucks: [Option<Hit>; 2] = if dark {
            [None, None]
        } else {
            [1.0f32, -1.0].map(|s| {
                let c = f.deck - f.up * TRUCK_TOP + f.fwd * (s * td);
                let side = f.right * t.truck_to_wheel;
                cross_box(c + side, side * -2.0, f.up * -BOX_DEPTH, c)
            })
        };
        // Tip boxes: along the board past each truck.
        let tips: [Option<Hit>; 2] = if dark {
            [None, None]
        } else {
            [1.0f32, -1.0].map(|s| {
                let c = f.deck + f.up * TRUCK_TOP;
                let a = c + f.fwd * (s * td);
                let b = c + f.fwd * (s * (td + TIP_LENGTH)) + f.up * TIP_RISE;
                cross_box(a, b - a, f.up * -BOX_DEPTH, (a + b) * 0.5)
            })
        };
        // The deck's box: along the board between the trucks.
        let deck = if dark {
            None
        } else {
            let c = f.deck + f.up * t.test_above;
            let along = f.fwd * td;
            cross_box(
                c + along,
                along * -2.0,
                f.up * -(t.test_above + t.test_below),
                f.deck,
            )
        };
        let inverted = if dark {
            let c = f.deck - f.up * t.test_above;
            let along = f.fwd * DARK_HALF;
            cross_box(
                c + along,
                along * -2.0,
                f.up * (t.test_above + t.test_below),
                f.deck,
            )
        } else {
            None
        };
        let seg_dir = |h: &Hit| {
            let r: &Rail = &rails[h.rail];
            (r.points[h.seg + 1] - r.points[h.seg]).normalize_or(v3(0.0, 0.0, 1.0))
        };
        let v = self.vel;
        let ground = category == Category::Ground;
        let contact = |h: &Hit, family: Family, front: bool, entry: Entry| Contact {
            family,
            rail: h.rail,
            seg: h.seg,
            point: h.point,
            dir: seg_dir(h),
            front,
            entry,
        };
        let fifty = || {
            let (Some(a), Some(b)) = (trucks[0], trucks[1]) else {
                return None;
            };
            if sticks_x.abs() >= FIFTY_BALANCE {
                return None;
            }
            if (a.rail, a.seg) != (b.rail, b.seg) {
                // Two segments of a rail: their hits must lie along it.
                let d = seg_dir(&a);
                let across = upright(d).cross(d);
                if a.rail != b.rail || (a.point - b.point).dot(across) > 0.1 {
                    return None;
                }
            }
            let d = seg_dir(&a);
            let entry = self.admit(category, current, Family::FiftyFifty, d, false)?;
            let mid = Hit {
                point: (a.point + b.point) * 0.5,
                ..a
            };
            Some(contact(&mid, Family::FiftyFifty, false, entry))
        };
        let five = || {
            let valid = trucks.map(|h| {
                let h = h?;
                let d = seg_dir(&h);
                let entry = self.admit(category, current, Family::FiveO, d, true)?;
                if entry == Entry::DropIn && (f.deck - h.point).dot(f.up).abs() >= FIVE_DEPTH {
                    return None;
                }
                if f.right.dot(d).abs() >= FIVE_RIGHT || f.up.dot(d).abs() >= FIVE_UP {
                    return None;
                }
                Some((h, entry))
            });
            let forwards = f.fwd.dot(v) > 0.0;
            let (lead, trail) = if forwards { (0, 1) } else { (1, 0) };
            // Both trucks: the stick's pitch (or the truck already on it) picks; the leading one alone
            // only from the air or just after riding onto it.
            let nose = self.grind_pitch_input > 0.35 || current_front(self, current) == Some(true);
            let pick = match (valid[lead], valid[trail]) {
                (Some(_), Some(_)) => {
                    if nose {
                        0
                    } else {
                        1
                    }
                }
                (Some(_), None) if !ground || self.riding_ticks <= 30 || nose == (lead == 0) => {
                    lead
                }
                (_, Some(_)) => trail,
                _ => return None,
            };
            let (h, entry) = valid[pick]?;
            Some(contact(&h, Family::FiveO, pick == 0, entry))
        };
        let tip = || {
            let front = tips[0].is_some();
            let h = if front { tips[0]? } else { tips[1]? };
            let d = seg_dir(&h);
            let entry = self.admit(category, current, Family::Tipslide, d, true)?;
            let descending = v.y < 0.0 && category != Category::Grind;
            if (entry == Entry::DropIn || descending) && f.deck.y <= h.point.y {
                return None;
            }
            if entry == Entry::DropIn && (f.deck - h.point).dot(f.up).abs() >= TIP_DEPTH {
                return None;
            }
            // Riding: moving (standing still beside an edge isn't a drop-in).
            if ground && v.length() < DROP_IN_MIN {
                return None;
            }
            // Under 3 m/s across the rail (SK8 families.rs `tipslide`): in a manual, only a rail on
            // the ground side of the deck (`balance_2720`, against the wheels' normal); riding over
            // a second, only towards the rail with the weight forwards and away from it without
            // (`flags_2472` bit 12: the animation's weight on the nose, turned round riding fakie).
            let across = v - d * v.dot(d);
            if across.dot(across) < 9.0 {
                if self.manual.is_some() {
                    if (h.point - f.deck).dot(self.contact_n) >= 0.0 {
                        return None;
                    }
                } else if self.riding_ticks > 30 {
                    let toward = (h.point - f.deck).dot(v) > 0.0;
                    let on_nose = self.graph.is_some_and(|g| g.weight_on_nose);
                    let forwards = if self.fakie { !on_nose } else { on_nose };
                    if forwards != toward {
                        return None;
                    }
                }
            }
            Some(contact(&h, Family::Tipslide, front, entry))
        };
        let slide = || {
            let h = deck?;
            let d = seg_dir(&h);
            if upright(d).dot(f.up) <= BOARD_UP {
                return None;
            }
            let entry = self.admit(category, current, Family::Boardslide, d, false)?;
            if ground && v.dot(d).abs() <= 0.75 && self.riding_ticks <= 10 {
                return None;
            }
            // The trucks must clear the rail.
            let horizontal = (d - f.up * d.dot(f.up)).normalize_or(d);
            let alignment = horizontal.dot(f.fwd).abs();
            let from = h.point - f.deck;
            let across = from - f.up * from.dot(f.up);
            let remaining = td - across.length();
            let clearance = if alignment > 0.0 {
                remaining / alignment * sqrtf((1.0 - alignment * alignment).max(0.0))
            } else {
                999.0
            };
            if clearance <= t.truck_to_wheel {
                return None;
            }
            Some(contact(&h, Family::Boardslide, false, entry))
        };
        let darkslide = || {
            let h = inverted?;
            let d = seg_dir(&h);
            if upright(d).dot(-f.up) <= DARK_UP {
                return None;
            }
            let entry = self.admit(category, current, Family::Boardslide, d, false)?;
            Some(contact(&h, Family::Darkslide, false, entry))
        };
        let retained = match current {
            Some(Family::FiftyFifty | Family::FiveO) => fifty().or_else(five),
            Some(Family::Tipslide | Family::Backslash) => tip(),
            Some(Family::Boardslide) => slide(),
            Some(Family::Darkslide) => darkslide(),
            None => None,
        };
        let mut c = retained
            .or_else(fifty)
            .or_else(five)
            .or_else(tip)
            .or_else(slide)
            .or_else(darkslide)?;
        // The rail's class: blocked rails were skipped; a tip over a ledge's high side is a blunt; fast
        // towards the low side of anything but a thin rail doesn't take.
        let r = &rails[c.rail];
        let s = r.segment_start(c.seg) + (c.point - r.points[c.seg]).length();
        let (class, high) = r.class_at(s);
        if c.family == Family::Tipslide && class.is_ledge() && current != Some(Family::Tipslide) {
            let over_high = (f.deck - c.point).dot(high) > 0.0;
            if over_high {
                c.family = Family::Backslash;
            }
        }
        if class != RailClass::Thin {
            let mut across = c.dir * c.dir.dot(v) - v;
            across.y = 0.0;
            if across.length() > LOW_SIDE_SPEED && v.dot(high) < 0.0 {
                return None;
            }
        }
        Some(c)
    }
}

/// Where the family's reference point (the deck's middle for a 50-50 and the slides, the truck on the
/// rail for a 5-0, the rail's place on the deck at entry for the slides) is across the rail from `p`
/// (m along `across`), for a deck at `deck` with its nose along `fwd`: the inverse of the board's
/// placement in `slide_rail`.
#[allow(clippy::too_many_arguments)]
fn ref_lateral(
    family: Family,
    front: bool,
    along: f32,
    deck_to_truck: f32,
    deck: Vec3,
    fwd: Vec3,
    p: Vec3,
    across: Vec3,
) -> f32 {
    let off = deck - p;
    match family {
        Family::FiftyFifty => off.dot(across),
        Family::FiveO => {
            let truck = if front { deck_to_truck } else { -deck_to_truck };
            (off + fwd * truck).dot(across)
        }
        _ => off.dot(across) + along * fwd.dot(across),
    }
}

/// The truck the board is on now, in a 5-0.
fn current_front(s: &Skater, current: Option<Family>) -> Option<bool> {
    match (current, &s.grind) {
        (Some(Family::FiveO), Some(g)) => Some(g.front),
        _ => None,
    }
}

/// The SK8 `deadzone`: 0 within 0.25, the rest stretched to the full range.
fn dead(v: f32) -> f32 {
    if v > YAW_DEAD {
        (v - YAW_DEAD) / (1.0 - YAW_DEAD)
    } else if v < -YAW_DEAD {
        (v + YAW_DEAD) / (1.0 - YAW_DEAD)
    } else {
        0.0
    }
}

impl Skater {
    /// Tries to lock onto a rail this tick (riding, or in the air). Returns whether the board is now
    /// grinding (or wiped out trying).
    pub(crate) fn try_grind(
        &mut self,
        park: &dyn Terrain,
        category: Category,
        events: &mut Vec<Event>,
    ) -> bool {
        let free = match self.graph {
            Some(g) => !g.danger,
            None => self.trick.is_none_or(|t| t.progress() >= 1.0),
        };
        if self.grab.is_some() || !free {
            return false;
        }
        match category {
            Category::Air => {
                let air = if self.is_airborne() {
                    self.air_time
                } else {
                    0.0
                };
                if air < REGRIND_AIR || self.jumper.since_jump <= REGRIND_TICKS {
                    return false;
                }
            }
            Category::Ground => {
                if self.approach.away <= REGRIND_TICKS {
                    return false;
                }
            }
            Category::Grind => return false,
        }
        let sticks = self.grind_sticks_x;
        let Some(c) = self.investigate(park, category, None, sticks) else {
            return false;
        };
        if self
            .regrind_block
            .is_some_and(|(r, t)| r == c.rail && t > 0.0)
        {
            return false;
        }
        // Onto the rail only as Skate 3's state selector takes the contact (its grind states).
        if !self.grind_taken(c.family as u32) {
            return false;
        }
        // Onto the rail as the deck was lined up (the family was picked from the adjusted deck).
        self.bake_deck_adjust();
        self.engage(park, c, category, events)
    }

    /// Takes a contact (SK8 entry.rs): the entry velocity, the entry wipeouts, the grind's start.
    pub(super) fn engage(
        &mut self,
        park: &dyn Terrain,
        c: Contact,
        category: Category,
        events: &mut Vec<Event>,
    ) -> bool {
        let t = self.tuning;
        let Some(rail) = park.rails().get(c.rail) else {
            return false;
        };
        let f = self.frame();
        let s0 = rail.segment_start(c.seg) + (c.point - rail.points[c.seg]).length();
        let (class, high) = rail.class_at(s0);
        let v = self.vel;
        if f.up.dot(c.dir).abs() > ENTRY_STEEP {
            if category == Category::Air {
                self.pos = c.point;
                self.vel = v * 0.5;
                self.bail("came onto the rail at a bad angle", events);
                return true;
            }
            return false;
        }
        let n = upright(c.dir);
        let entry_v = match category {
            Category::Ground => {
                let base = match c.family {
                    Family::Boardslide => 0.1,
                    Family::Tipslide | Family::Backslash
                        if sticks_nonzero(self.grind_sticks_x) && v.length() < DROP_IN_SPEED =>
                    {
                        1.0
                    }
                    Family::Tipslide => 0.4,
                    _ => 0.2,
                };
                let help = t.grind.vert_help.eval(slope_sine(c.dir, v, 0.001));
                let amount = (help + (1.0 - help) * base).min(1.0);
                v * (1.0 - amount) + c.dir * (v.dot(c.dir) * amount)
            }
            _ => {
                let across = n.cross(c.dir).normalize_or(Vec3::ZERO);
                let removed = across * v.dot(across);
                let k = c.family.air_entry();
                (v - removed) * k + v * (1.0 - k)
            }
        };
        let impact = (entry_v - v).length();
        // Skate 3's bad landing onto a rail (SK8 wipeout/air.rs): the mode's check and scale.
        if t.bad_landing && impact > t.land_max_grind * t.bad_landing_scale {
            self.pos = c.point;
            self.vel = v * 0.5;
            self.bail("came onto the rail too hard", events);
            return true;
        }
        let mut across = c.dir * v.dot(c.dir) - v;
        across.y = 0.0;
        if across.length() > CROSS_WIPEOUT && (class == RailClass::Thin || v.dot(high) > 0.0) {
            self.pos = c.point;
            self.vel = v * 0.5;
            self.bail("crossed the rail too fast", events);
            return true;
        }
        // The closing speed (SK8 post.rs, the board's contact with the rail): the speed into the rail it
        // stops, across the rider's ground plane. Square to the rail that is straight down the rider's
        // up; a board rolled or tipped against the rail takes part of it sideways or along.
        let removed = n * entry_v.dot(n).min(0.0);
        if balance::closing_xz(removed, f.up, f.fwd) > t.grind.wipeout_xz {
            self.pos = c.point;
            self.vel = v * 0.5;
            self.bail("slammed into the rail", events);
            return true;
        }
        // Onto the rail: the speed along it is the entry velocity's (3-D, no minimum); the speed across
        // it is what the entry left (the balance forces take it from there).
        let along = entry_v.dot(c.dir);
        let across_s = c.dir.cross(n).normalize_or(Vec3::ZERO);
        let dir = if along >= 0.0 { 1.0 } else { -1.0 };
        let into = self.trick_label().map(|(n, trick)| {
            (
                format!("{}{n}", self.stance_prefix()),
                crate::scoring::Air {
                    trick,
                    ..Default::default()
                },
            )
        });
        // Where along the board the rail is.
        let along_board = (c.point - f.deck).dot(f.fwd);
        // The approach (SK8 chromosome): from where the rider was against the rail, and the chest then.
        let reference = self.approach.reference();
        let travel = c.dir * dir;
        let side = upright(c.dir).cross(c.dir).normalize_or(Vec3::ZERO);
        let to_ref = if side.dot(reference.pos - c.point) > 0.0 {
            side
        } else {
            -side
        };
        let backside = reference.chest.dot(to_ref) > 0.0;
        let reversed = (self.chest().dot(reference.chest) < 0.0) != reference.fakie;
        // Coping hold: riding steeply into it, for a time set by the vertical and the along-rail speed.
        let relief = if category == Category::Ground && slope_sine(c.dir, v, 0.001) > RELIEF_SLOPE {
            t.grind.relief_vs_vy.eval(v.y) * t.grind.relief_vs_speed.eval(along.abs())
        } else {
            0.0
        };
        let delay = match c.entry {
            Entry::RideFromBelow => LEAN_DELAY_BELOW,
            Entry::RideIntoCoping => LEAN_DELAY_COPING,
            _ => LEAN_DELAY,
        };
        // A 5-0's turn off the rail and its tip, as it came in.
        let fh = (f.fwd - n * f.fwd.dot(n)).normalize_or(f.fwd);
        let aligned = if fh.dot(travel) >= 0.0 {
            travel
        } else {
            -travel
        };
        let twist = air::signed_angle(aligned, fh, n).clamp(-TWIST_MAX, TWIST_MAX);
        let tilt = libm::asinf((-f.fwd.dot(n)).clamp(-1.0, 1.0)).clamp(-TILT_MAX, TILT_MAX);
        self.grind = Some(Grind {
            rail: c.rail,
            s: s0,
            point: c.point,
            dir,
            speed: along.abs(),
            family: c.family,
            front: c.front,
            entry: c.entry,
            class,
            high,
            along: along_board.clamp(-0.42, 0.42),
            time: 0.0,
            into,
            pitch: 0.0,
            yaw: 0.0,
            tilt: if c.family == Family::FiveO { tilt } else { 0.0 },
            twist: if c.family == Family::FiveO {
                twist
            } else {
                0.0
            },
            relief,
            lean: Lean {
                elapsed: 0.0,
                delay,
                angle: 0.0,
            },
            lateral: ref_lateral(
                c.family,
                c.front,
                along_board.clamp(-0.42, 0.42),
                t.grind.deck_to_truck,
                f.deck,
                f.fwd,
                c.point,
                across_s,
            ),
            lateral_vel: entry_v.dot(across_s),
            tip_hold: if c.family == Family::Tipslide
                && c.entry == Entry::DropIn
                && v.length() > balance::TIP_HOLD_SPEED
            {
                balance::TIP_HOLD_TICKS
            } else {
                0
            },
            preparing: false,
            closing: 0.0,
            grab: None,
            grabbed: None,
            slow: 0.0,
            naming: Naming::default(),
            backside,
            reversed,
            coping: class.is_ledge() && super::stall::is_coping(park, c.point, high),
            top_speed: along.abs(),
        });
        self.grind_exit = None;
        self.name_grind(c.dir, upright(c.dir), true);
        self.trick = None;
        self.grabbed = None;
        self.takeoff.hippy = false;
        self.spin_rate = 0.0;
        self.manual = None;
        self.slide = None;
        self.push = None;
        self.brake = None;
        self.revert = None;
        self.kickturn = None;
        self.kick_rate = 0.0;
        self.wallride = None;
        self.landing = None;
        self.plan = None;
        self.air_assist = None;
        self.vn = 0.0;
        self.approach.away = 0;
        true
    }

    /// Names the grind this tick (SK8 `Chromosome::update`).
    fn name_grind(&mut self, d: Vec3, n: Vec3, first: bool) {
        let chest = self.chest();
        let f = self.frame();
        let Some(g) = &mut self.grind else { return };
        let rail_dir = d * g.dir;
        let forward = chest.dot(rail_dir) > 0.0;
        // The truck or tip on the rail: behind the middle is the back one.
        let back = !g.front;
        let axis = f.fwd - n * f.fwd.dot(n);
        let straight =
            axis.length() <= 0.01 || (axis.normalize_or(f.fwd)).dot(rail_dir).abs() > STRAIGHT_COS;
        // Low: the board's middle side dips under the rail's plane.
        let to_middle = if g.front { -f.fwd } else { f.fwd };
        let low = n.dot(to_middle) <= LOW_DOT;
        let name = name_of(
            g.backside, back, straight, low, g.reversed, forward, g.family,
        );
        g.naming.update(name, first);
    }

    /// Grind grabs (Skate 3's `OnBoard/Grinding/<grind>/<grind>_<grab>` states, entered from the
    /// action graph's `LeftGroundGrab` / `RightGroundGrab`): a trigger fully in grabs with that hand, the
    /// left one the frontside grab and the right one the backside grab (the other way round for the
    /// mirrored rider), each as this grind has it; both hands the frontside one where there is one. With
    /// the state graphs, the grab their grind-grab states score.
    fn grind_grab(&self, g: &mut Grind, pad: &Pad, dt: f32) {
        let now = match self.graph {
            Some(gc) => gc.grind_grab,
            None => {
                let full = crate::graph::input::TRIGGER_FULL;
                let (l, r) = (pad.trig_l >= full, pad.trig_r >= full);
                let (fs, bs) = if self.mirrored() { (r, l) } else { (l, r) };
                let [fs_grab, bs_grab] = balance::grind_grabs(g.base());
                match (fs, bs) {
                    (true, true) => fs_grab.or(bs_grab),
                    (true, false) => fs_grab,
                    (false, true) => bs_grab,
                    (false, false) => None,
                }
            }
        };
        g.grab = match (g.grab, now) {
            (Some((a, t)), Some(b)) if a == b => Some((a, t + dt)),
            (_, Some(b)) => Some((b, 0.0)),
            (_, None) => None,
        };
        if let Some((a, t)) = g.grab
            && g.grabbed.is_none_or(|(_, longest)| t > longest)
        {
            g.grabbed = Some((a, t));
        }
    }

    /// One tick on the rail: the controls, gravity and friction along it, the board's pose, the balance
    /// across it ([`super::balance`]), the lean-off, and leaving it.
    pub(super) fn slide_rail(
        &mut self,
        park: &dyn Terrain,
        pad: &Pad,
        flick: &FlickOut,
        dt: f32,
        events: &mut Vec<Event>,
    ) {
        let t = self.tuning;
        // Taken out for the tick (no clone: the grind owns a string) and put back before naming.
        let Some(mut g) = self.grind.take() else {
            self.take_off(None);
            return;
        };
        let Some(rail) = park.rails().get(g.rail) else {
            // The rail went away (a park reload): just drop.
            self.take_off(None);
            return;
        };
        let (_, d) = rail.at(g.s);
        let travel = d * g.dir;
        let n = upright(d);
        let ticks = dt * 60.0;
        let td = t.grind.deck_to_truck;
        let last = self.frame();

        // Grind grabs, and the 5-0's free end they hold up (`PhysGrindGrabMinHeight`).
        self.grind_grab(&mut g, pad, dt);
        let grab_min = match self.graph {
            Some(gc) => gc.grind_grab_min_height,
            None if g.grab.is_some() && g.family == Family::FiveO => balance::GRAB_MIN_HEIGHT,
            None => 0.0,
        };

        // Controls (SK8 control.rs): the stick's pitch, the sticks' yaw.
        let up_down = pad.flick[1];
        let translation = clamp(pad.stick[0] + pad.flick[0], -1.0, 1.0);
        let nudge = pad.stick[0];
        let (yaw_in, pitch_in, modified) = match g.family {
            Family::FiftyFifty => (0.0, up_down * up_down * up_down, false),
            Family::FiveO => {
                // The free end's height (rad, + up): the tail on a nosegrind, the nose on a 5-0.
                let height = if g.front { g.tilt } else { -g.tilt };
                let lift_sign = if g.front { 1.0 } else { -1.0 };
                let mut input = up_down;
                if height > 0.0 && up_down.abs() <= 0.01 {
                    input = FREE_END_HOLD * lift_sign;
                }
                let mut target = input;
                if height > FREE_END_MAX {
                    target = if g.front {
                        input.min(0.0)
                    } else {
                        input.max(0.0)
                    };
                }
                if height < FREE_END_MIN {
                    target = if g.front {
                        input.max(0.0)
                    } else {
                        input.min(0.0)
                    };
                }
                if let Some(v) = balance::grab_lift(grab_min, height, lift_sign) {
                    target = v;
                }
                let yaw = (dead(translation) * YAW_STICKS + dead(nudge) * YAW_NUDGE)
                    .clamp(-YAW_STICKS, YAW_STICKS);
                (yaw * yaw * yaw, target * target * target, target != input)
            }
            _ => (0.0, 0.0, false),
        };
        let keep = if modified {
            PITCH_KEEP_MODIFIED
        } else {
            PITCH_KEEP
        };
        g.yaw = g.yaw * PITCH_KEEP + yaw_in * (1.0 - PITCH_KEEP);
        g.pitch = g.pitch * keep + pitch_in * (1.0 - keep);
        if g.yaw.abs() < 1e-4 {
            g.yaw = 0.0;
        }
        if g.pitch.abs() < 1e-4 {
            g.pitch = 0.0;
        }
        // 50-50 into 5-0 or nosegrind (ChangeGrind within 24 degrees): tipped by the stick's pitch, or
        // the sticks pushed fully to the side (a 50-50 needs them under 0.9; then the back truck, or the
        // front one with the C-stick up, as SK8's investigator picks it), and back once level.
        let family_was = g.family;
        match g.family {
            Family::FiftyFifty
                if (g.pitch.abs() > PITCH_TIPS || translation.abs() >= FIFTY_BALANCE)
                    && within(d, self.vel, CHANGE_DEG) =>
            {
                g.family = Family::FiveO;
                g.front = if g.pitch.abs() > PITCH_TIPS {
                    g.pitch > 0.0
                } else {
                    up_down > 0.35
                };
                g.tilt = 0.0;
                g.twist = 0.0;
            }
            // Both trucks on the rail again (SK8 tries the 50-50 first every tick): the other truck is
            // over the rail and the board near level and along it.
            Family::FiveO
                if g.tilt.abs() < FIFTY_AGAIN
                    && g.pitch.abs() <= PITCH_TIPS
                    && g.twist.abs() < FIFTY_TWIST
                    && translation.abs() < FIFTY_BALANCE
                    && {
                        let sign = if g.front { -1.0 } else { 1.0 };
                        let other = g.s + self.fwd.dot(d) * sign * 2.0 * td;
                        (0.0..=rail.length()).contains(&other)
                    } =>
            {
                g.family = Family::FiftyFifty;
                g.tilt = 0.0;
                g.twist = 0.0;
            }
            _ => {}
        }
        if g.family != family_was {
            // The reference point moves (the deck's middle, the truck): where the board is stays.
            let across = d.cross(n).normalize_or(Vec3::ZERO);
            g.lateral = ref_lateral(
                g.family, g.front, g.along, td, last.deck, last.fwd, g.point, across,
            );
        }
        if g.family == Family::FiveO {
            g.tilt = (g.tilt + g.pitch * PITCH_RATE * ticks).clamp(-TILT_MAX, TILT_MAX);
            g.twist = (g.twist - g.yaw * PITCH_RATE * ticks).clamp(-TWIST_MAX, TWIST_MAX);
        }

        // Along the rail: gravity, and friction by family and class growing over time, which damps
        // the whole velocity in the rail's plane, the speed across it too (SK8 `friction`).
        let friction = friction_decel(&t, g.family, g.class, n.y, g.time);
        let gravity = -t.gravity * travel.y;
        g.speed += gravity * dt;
        let planar = sqrtf(g.speed * g.speed + g.lateral_vel * g.lateral_vel);
        if planar > 0.001 {
            let k = (friction * dt / planar).min(1.0);
            g.speed -= g.speed * k;
            g.lateral_vel -= g.lateral_vel * k;
        }
        if g.speed < 0.0 {
            // Sliding back down the rail.
            g.speed = -g.speed;
            g.dir = -g.dir;
        }
        let travel_was = d * g.dir;
        let across_was = d.cross(n).normalize_or(Vec3::ZERO);
        let v_was = travel_was * g.speed + across_was * g.lateral_vel;
        g.s += g.dir * g.speed * dt;
        g.time += dt;
        g.relief = (g.relief - dt).max(0.0);
        let (p, d) = rail.at(g.s);
        g.point = p;
        let travel = d * g.dir;
        let n = upright(d);
        let across_s = d.cross(n).normalize_or(Vec3::ZERO);
        // Round a bend the rail takes the board with it: the velocity into the rail's new plane is
        // stopped (its closing speed), the rest goes on along and across it; over a crest the board
        // follows the rail, keeping its speed.
        let into = v_was.dot(n);
        let along = v_was.dot(travel).max(0.0);
        g.lateral_vel = v_was.dot(across_s);
        g.speed = if into >= 0.0 {
            sqrtf(along * along + into * into)
        } else {
            along
        };
        g.closing = balance::closing_xz(n * into.min(0.0), last.up, last.fwd);
        let (class, high) = rail.class_at(g.s);
        g.class = class;
        if class.is_ledge() {
            g.high = high;
        }

        // Lean-off (SK8 balance.rs): on ledges, and tip slides on anything but a thin rail.
        let droppable = match g.family {
            Family::Tipslide | Family::Backslash => class != RailClass::Thin,
            _ => class.is_ledge(),
        };
        if droppable || g.lean.elapsed > 0.0 {
            g.lean.elapsed += dt;
        }
        let late = g.lean.elapsed - g.lean.delay;
        g.lean.angle = if late > 0.0 {
            t.grind.exit_lean.eval(late)
        } else {
            0.0
        };
        // The low side: away from the high side (on a thin rail, the side the board hangs over).
        let low_side = if class.is_ledge() {
            -g.high
        } else {
            let side = n.cross(d).normalize_or(Vec3::ZERO);
            if side.dot(self.pos - p) >= 0.0 {
                side
            } else {
                -side
            }
        };
        let forced = g.lean.angle > LEAN_FORCED
            || (g.lean.angle > LEAN_FORCED_NO_GROUND && {
                let start = p + low_side * 0.1;
                match park.raycast(start, -Vec3::UP, 5.0) {
                    Some(h) => h.normal.dot(-low_side) > -0.1,
                    None => true,
                }
            });

        // The board's pose: the target frame for the family, eased towards (SK8 orientation.rs).
        let frame = last;
        let (target_up, target_fwd) = match g.family {
            Family::FiftyFifty => {
                let fwd = if frame.fwd.dot(travel) >= 0.0 {
                    travel
                } else {
                    -travel
                };
                (n, fwd)
            }
            Family::FiveO => {
                let base = if frame.fwd.dot(travel) >= 0.0 {
                    travel
                } else {
                    -travel
                };
                let fwd = rotate(base, n, g.twist);
                let right = n.cross(fwd).normalize_or(Vec3::ZERO);
                let fwd2 = rotate(fwd, right, g.tilt);
                (fwd2.cross(right).normalize_or(n), fwd2)
            }
            Family::Boardslide | Family::Darkslide | Family::Tipslide | Family::Backslash => {
                let up = match g.family {
                    Family::Boardslide => n,
                    Family::Darkslide => -n,
                    _ => {
                        let across = d.cross(n);
                        let axis = if across.dot(frame.deck - p) > 0.0 {
                            -d
                        } else {
                            d
                        };
                        let angle = if g.family == Family::Backslash {
                            BLUNT_LEAN
                        } else {
                            TIP_LEAN
                        };
                        rotate(n, axis, angle)
                    }
                };
                // Across the rail, the end on it towards it (the nose for a nose slide).
                let across = d.cross(up).normalize_or(frame.fwd);
                let towards = if g.along.abs() > 0.1 {
                    (p - frame.deck) * g.along.signum()
                } else {
                    frame.fwd
                };
                (
                    up,
                    if across.dot(towards) >= 0.0 {
                        across
                    } else {
                        -across
                    },
                )
            }
        };
        // Leaning off: the board (and the grind's normal) rolls away from the high side.
        let (target_up, target_fwd, grind_n) = if g.lean.angle > 0.0 {
            let axis = if d.cross(-low_side).y > 0.0 { d } else { -d };
            let a = g.lean.angle * PI / 180.0;
            (
                rotate(target_up, axis, a),
                rotate(target_fwd, axis, a),
                rotate(n, axis, a),
            )
        } else {
            (target_up, target_fwd, n)
        };
        let k = 1.0 - libm::powf(1.0 - ORIENT_BLEND, ticks);
        let up = frame.up.lerp(target_up, k).normalize_or(target_up);
        let fwd = tangent(frame.fwd.lerp(target_fwd, k), up);
        // The deck slammed too far from where the rail holds it: a wipeout (SK8 post.rs).
        let error = libm::acosf(up.dot(target_up).clamp(-1.0, 1.0));

        // Across the rail (SK8's rigid board): each family's balance forces on its reference point,
        // through the effective mass. Forces are along the segment's across axis; the sticks push
        // along the travel's (its right, as the rider sees the rail ahead).
        let fwd_a = fwd.dot(across_s);
        let to_travel = g.dir;
        let slope = balance::pin_slope(&t.grind, grind_n.dot(n), g.relief);
        g.preparing |= match self.graph {
            Some(gc) => gc.anticipating,
            None => flick.crouch > 0.0,
        };
        let mut force = 0.0;
        let mut stop = false;
        match g.family {
            Family::FiftyFifty => {
                force += balance::pin(balance::PIN_FIFTY, -g.lateral, g.lateral_vel, slope);
            }
            Family::FiveO => {
                force += balance::pin(balance::PIN_FIVE, -g.lateral, g.lateral_vel, slope);
            }
            Family::Backslash => {
                let reach = balance::BLUNT_REACH * g.along.signum();
                let error = -g.lateral + (g.along - reach) * fwd_a;
                force += balance::pin(balance::PIN_BLUNT, error, g.lateral_vel, slope);
            }
            Family::Boardslide | Family::Darkslide => {
                let offset = (g.lateral - g.along * fwd_a) * to_travel;
                let (f, stopped) = balance::slide(
                    g.family == Family::Darkslide,
                    offset,
                    g.lateral_vel * to_travel,
                    translation,
                    class.is_ledge(),
                    g.preparing,
                );
                force += f * to_travel;
                stop = stopped;
            }
            Family::Tipslide => {
                let offset = (g.lateral - g.along * fwd_a) * to_travel;
                let inward = if offset > 0.0 { -1.0 } else { 1.0 };
                force += balance::tip(nudge, g.tip_hold > 0, inward) * to_travel;
            }
        }
        g.tip_hold = g.tip_hold.saturating_sub(1);
        force += low_side.dot(across_s) * balance::exit_lean(&t.grind, g.lean.angle);
        if stop {
            g.lateral_vel = 0.0;
        }
        g.lateral_vel += force / GRIND_MASS * dt;
        g.lateral += g.lateral_vel * dt;

        // Still on the rail: it must stay in the box it is in (the truck's width, the deck between the
        // trucks, the tip); a tip slide pushed in past its truck is a boardslide.
        let q = g.along - g.lateral * fwd_a;
        let fell = match g.family {
            Family::FiftyFifty | Family::FiveO => {
                g.lateral.abs() > t.grind.truck_to_wheel * up.cross(fwd).dot(across_s).abs()
            }
            Family::Boardslide => q.abs() > td,
            Family::Darkslide => q.abs() > DARK_HALF,
            Family::Tipslide | Family::Backslash => {
                let on_tip = q * g.along.signum();
                if on_tip < td {
                    g.family = Family::Boardslide;
                }
                on_tip > td + TIP_LENGTH
            }
        };

        self.normal = up;
        self.contact_n = up;
        self.fwd = fwd;
        // Where the board sits so the truck, tip or deck on the rail stays on it, moved across the rail
        // by the balance.
        let offset = match g.family {
            Family::FiftyFifty => 0.0,
            Family::FiveO => {
                if g.front {
                    td
                } else {
                    -td
                }
            }
            _ => g.along,
        };
        // The trucks' hangers (grinds), the deck's underside (slides) or its grip (a darkslide, the deck
        // upside down) on the rail.
        let sink = match g.family {
            Family::FiftyFifty | Family::FiveO => DECK_HEIGHT - HANGER_DROP,
            Family::Darkslide => DECK_HEIGHT + DECK_UNDERSIDE,
            _ => DECK_HEIGHT - DECK_UNDERSIDE,
        };
        self.pos = p + across_s * g.lateral - fwd * offset - up * sink;
        self.speed = g.speed * if fwd.dot(travel) >= 0.0 { 1.0 } else { -1.0 };
        self.vel = travel * g.speed + across_s * g.lateral_vel;
        self.grind = Some(g);
        self.name_grind(d, grind_n, false);
        let Some(mut g) = self.grind.take() else {
            return;
        };
        // Wipeouts (SK8 post.rs): the deck slammed too far from where the rail holds it, or the board
        // hit the rail too hard (its closing speed across the rider's ground plane).
        let slammed = if error > t.grind.wipeout_angle && g.time > 0.1 {
            Some("the deck slammed off the rail")
        } else if g.closing > t.grind.wipeout_xz {
            Some("slammed into the rail")
        } else {
            None
        };
        if let Some(reason) = slammed {
            self.vel = travel * g.speed + across_s * g.lateral_vel;
            self.bail(reason, events);
            return;
        }

        // The pop: the state graph's takeoff (its jump), else the flick, read with the grind's group.
        let pop = match self.graph {
            Some(gc) => gc.pop,
            None => flick.flick.map(|f| {
                (
                    gesture::select(f, g.group(), self.mirrored()),
                    flick.strength,
                )
            }),
        };
        let off_end = g.s <= 0.0 || g.s >= rail.length();
        g.top_speed = g.top_speed.max(g.speed);
        let stopped = g.speed < STOP_SPEED && g.relief <= 0.0;
        g.slow = if stopped { g.slow + dt } else { 0.0 };
        let stalled = g.slow > STOP_TIME;
        let leaving = pop.is_some() || off_end || forced || stalled || fell;
        if !leaving {
            self.grind = Some(g);
            return;
        }
        // A board or dark slide that stalls on a rail (not a ledge) hangs up on it: Skate 3's slides
        // wipe out there (SK8 involuntary.rs, wipeout 10), where the grinds and ledges step off.
        if pop.is_none()
            && stalled
            && !forced
            && !off_end
            && matches!(g.family, Family::Boardslide | Family::Darkslide)
            && !class.is_ledge()
        {
            self.vel = travel * g.speed;
            self.bail("the slide hung up on the rail", events);
            return;
        }
        let leaned = forced || (fell && g.lean.angle > 0.0);
        self.grind_exit = Some(if pop.is_some() {
            "popped"
        } else if leaned {
            "leaned off"
        } else if off_end {
            "off the end"
        } else if fell {
            "fell off"
        } else {
            "stopped"
        });
        // Held still on a coping and falling back in: a stall (`skater::stall`), named for the grind it
        // is; the main stick to the side turns a rock round.
        let stall = (g.coping && pop.is_none() && g.top_speed < super::stall::STALL_SPEED)
            .then(|| {
                let turned = pad.stick[0].abs() > super::stall::ROLL_STICK;
                super::stall::Stall::of(g.family, g.front, self.fwd, g.high, turned)
            })
            .flatten();
        // Leaving the rail: score it, square the board back up along the rail and fly off.
        if g.time >= GRIND_MIN_TIME {
            let display = match stall {
                Some(st) => st.display(g.frontside()),
                None => g.display(),
            };
            let name = match &g.into {
                Some((tr, _)) => format!("{tr} {display}"),
                None => display,
            };
            let base = g.scorable();
            events.push(Event::Landed {
                name,
                points: 0,
                score: crate::scoring::Scored::Grind {
                    into: g.into.as_ref().map(|t| t.1),
                    id: stall.and_then(|st| st.id(g.frontside())).or(base),
                    base,
                    held: g.time,
                },
            });
        }
        self.stalled = stall;
        let backwards = self.fwd.dot(travel) < 0.0;
        let square = if backwards { -travel } else { travel };
        let lean_off = leaned || (stalled && pop.is_none());
        self.approach.away = 0;
        let mut vel = travel * g.speed + across_s * g.lateral_vel;
        self.pos = p + across_s * g.lateral + Vec3::UP * 0.02;
        if pop.is_none() {
            // Not popped: this rail can't be taken again straight away (the boxes still touch it).
            self.regrind_block = Some((g.rail, LEAN_OFF_BLOCK));
        }
        // Leaned (or stepped) off: pushed off the low side, at least `EXIT_MIN_PUSH`.
        let off_low_now = (across_s * g.lateral_vel).dot(low_side);
        let off_low = off_low_now.max(EXIT_MIN_PUSH);
        if lean_off {
            vel += low_side * (off_low - off_low_now);
            self.pos += low_side * 0.05;
        } else if off_end && !g.family.is_slide() && pop.is_none() {
            // A grind off the end lifts a little (SK8 release nudge, not for slides).
            vel += n * (n.y.max(0.0) * RELEASE_LIFT);
        }
        // Squared back up along the rail, nose first or tail first, whichever is nearer; leaning off
        // slower along the rail than off it (a rock on coping), the board stays as it is and turns to
        // the landing in the air.
        if !(lean_off && g.speed < off_low) {
            self.fwd = square;
            self.normal = Vec3::UP;
            self.contact_n = Vec3::UP;
            self.fakie = backwards;
        }
        // Out of a truck stall or a rock and roll the rider turns back down the ramp.
        if stall.is_some_and(|st| st.turns_down()) {
            let down = v3(low_side.x, 0.0, low_side.z).normalize_or(self.fwd);
            self.fwd = down;
            self.normal = Vec3::UP;
            self.contact_n = Vec3::UP;
            self.fakie = false;
        }
        match pop {
            Some((f, strength)) => {
                let jump = g.family.pop(&t.grind, strength);
                self.vel = self.launch(&g, vel, p, d, jump, pad.stick[0]);
                self.take_off(Some(f));
                events.push(Event::Popped(f));
            }
            None => {
                self.vel = vel;
                self.take_off(None);
            }
        }
    }

    /// The pop out of a grind (SK8 launch.rs): the speed along the rail kept, the jump along the
    /// family's launch normal, the stick's sideways part (less the high side's push), the energy cost.
    fn launch(
        &mut self,
        g: &Grind,
        vel: Vec3,
        point: Vec3,
        d: Vec3,
        jump: f32,
        stick_x: f32,
    ) -> Vec3 {
        let n = upright(d);
        let candidate = match g.family {
            Family::Tipslide | Family::Backslash => {
                let across = n.cross(d);
                let out = if across.dot(self.pos - point) > 0.0 {
                    across
                } else {
                    -across
                };
                let k = if g.family == Family::Backslash {
                    LAUNCH_BLUNT
                } else {
                    LAUNCH_TIP
                };
                n + out * k
            }
            _ => (n + self.normal) * 0.5,
        };
        let normal = if candidate.length() > 0.1 {
            candidate.normalize_or(n)
        } else {
            n
        };
        let planar = vel - normal * vel.dot(normal);
        let base = planar + normal * (jump * LAUNCH_JUMP);
        let across = d.cross(normal).normalize_or(Vec3::ZERO);
        let mut balance = stick_x;
        let low = self.jumper.energy < LOW_ENERGY;
        if low {
            balance = if balance < 0.0 {
                balance.min(-LOW_ENERGY_SIDE)
            } else if balance > 0.0 {
                balance.max(LOW_ENERGY_SIDE)
            } else if (self.pos - point).dot(across) > 0.0 {
                LOW_ENERGY_SIDE
            } else {
                -LOW_ENERGY_SIDE
            };
            self.jumper.cooldown = LOW_ENERGY_TICKS;
        }
        let side = if g.class == RailClass::Thin {
            0.0
        } else {
            g.family.side_jump(g.class)
        };
        let mut lateral = across * (balance * LAUNCH_SIDE) - g.high * side;
        let len = lateral.length();
        if len > LAUNCH_SIDE {
            lateral = lateral * (LAUNCH_SIDE / len);
        }
        self.jumper.energy = (self.jumper.energy - POP_ENERGY).max(0.0);
        self.jumper.since_jump = 0;
        base + lateral
    }
}

/// The lock-on point each air-assist kind aims (deck middle, nose tip, tail tip, deck middle, front
/// truck, front truck, back truck) and the board's angle to the rail it wants (degrees: across for the
/// slides, along for the trucks); kind 0 is the upside-down deck (SK8 grind_air targets.rs).
const ASSIST_POINT: [usize; 7] = [0, 1, 4, 0, 2, 2, 3];
const ASSIST_ANGLE: [f32; 7] = [90.0, 90.0, 90.0, 90.0, 0.0, 0.0, 0.0];

/// The grind air assist (SK8 `GrindAir`): how far it has moved the board towards the rail (m, at most
/// `MaxOffsetDist`) and which kind of lock-on it is going for.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Assist {
    pub offset: Vec3,
    pub kind: Option<usize>,
    /// What it did this tick: moved (m) and turned (rad).
    pub moved: f32,
    pub turned: f32,
}

impl Skater {
    /// One tick of the grind air assist towards the targeted rail (SK8 grind_air.rs): the board's five
    /// lock-on points (deck middle, nose and tail tips, both trucks) are followed `NumFramesToTest` ticks
    /// ahead to where they cross the rail's plane; the kind whose point crosses nearest the rail within
    /// its reach (`MaxDist*` times the deck-to-truck distance) and whose angle to the rail is within its
    /// window (`AngleRange*`) is aimed for: the board moves towards the rail by at most
    /// `MaxOffsetDeltaPerFrame` a tick (`MaxOffsetDist` in all) and turns towards along or across it by
    /// at most `MaxAngleDeltaPerFrame` a tick (times `AngleYAssist*`). Skate 3 moves its animated board
    /// this way; flatground moves the board itself (its rails have no collision to land the trucks on).
    pub(super) fn grind_air_assist(
        &mut self,
        park: &dyn Terrain,
        target: air::GrindTarget,
        dt: f32,
    ) {
        let t = self.tuning.grind;
        let Some(rail) = park.rails().get(target.rail) else {
            return;
        };
        let Some(w) = rail.points.get(target.seg..target.seg + 2) else {
            return;
        };
        let origin = w[0];
        let rail_dir = (w[1] - w[0]).normalize_or(target.dir);
        let n = upright(rail_dir);
        let frames = (t.frames as usize).clamp(2, 12);
        let pts = t.points();
        let mut f = self.frame();
        let mut pos = self.pos;
        let mut vel = self.vel;
        let spin = self.spin_rate;
        let mut samples = [[Vec3::ZERO; 5]; 12];
        let mut headings = [0.0f32; 12];
        let heading = |fwd: Vec3| {
            let p = (fwd - n * fwd.dot(n)).normalize_or(rail_dir);
            let aligned = if p.dot(rail_dir) < 0.0 { -p } else { p };
            air::signed_angle(rail_dir, aligned, n) * (180.0 / PI)
        };
        for k in 0..frames {
            let deck = pos + f.up * DECK_HEIGHT;
            for (i, p) in pts.iter().enumerate() {
                samples[k][i] = deck + f.right * p[0] + f.up * p[1] + f.fwd * p[2];
            }
            headings[k] = heading(samples[k][1] - samples[k][4]);
            f.fwd = rotate(f.fwd, f.up, spin * dt);
            f.right = f.up.cross(f.fwd);
            vel += f.up * (t.stomp * dt) - Vec3::UP * (self.tuning.gravity * dt);
            pos += vel * dt;
        }
        // Where each point first crosses the rail's plane: when (in ticks), how far from the rail, and
        // the move that would put it on the rail.
        let mut times = [-1.0f32; 5];
        let mut dist = [0.0f32; 5];
        let mut corr = [Vec3::ZERO; 5];
        let mut earliest = frames as f32;
        for p in 0..5 {
            for i in 1..frames {
                let a = n.dot(samples[i - 1][p] - origin);
                let b = n.dot(samples[i][p] - origin);
                if a * b < 0.0 {
                    let k = (a / (a - b)).abs();
                    let at = samples[i - 1][p].lerp(samples[i][p], k);
                    let on = origin + rail_dir * rail_dir.dot(at - origin);
                    times[p] = (i - 1) as f32 + k;
                    corr[p] = on - at;
                    dist[p] = corr[p].length();
                    earliest = earliest.min(times[p]);
                    break;
                }
            }
        }
        let z = t.truck_distance();
        let reach = [
            z,
            z * t.reach_tip,
            z * t.reach_tip,
            z * t.reach_board,
            z * t.reach_50,
            z * t.reach_5o,
            z * t.reach_5o,
        ];
        let range = [
            90.0,
            t.angle_tip,
            t.angle_tip,
            t.angle_board,
            t.angle_50,
            t.angle_5o,
            t.angle_5o,
        ];
        let yaw = [
            t.yaw_board,
            t.yaw_tip,
            t.yaw_tip,
            t.yaw_board,
            t.yaw_50,
            t.yaw_5o,
            t.yaw_5o,
        ];
        let valid = |kind: usize| {
            let p = ASSIST_POINT[kind];
            let at = times[p];
            if kind == 4 {
                // Both trucks reach the rail together.
                if times[2].min(times[3]) < 4.0 && (times[2] - times[3]).abs() > 2.0 {
                    return false;
                }
            } else if earliest < 5.0 && (earliest - at).abs() > 3.0 {
                return false;
            }
            let h = headings[((at + 0.5) as usize).min(frames - 1)];
            at > 0.0 && dist[p] < reach[kind] && (ASSIST_ANGLE[kind] - h.abs()).abs() < range[kind]
        };
        let mut chosen: Option<usize> = None;
        for kind in 0..7 {
            if self.dark != (kind == 0) || !valid(kind) {
                continue;
            }
            let mut kind = kind;
            if kind == 3
                && let Some(old @ (1 | 2)) = self.air_assist.and_then(|a| a.kind)
                && valid(old)
            {
                kind = old;
            }
            if chosen.is_none_or(|c| dist[ASSIST_POINT[kind]] < dist[ASSIST_POINT[c]]) {
                chosen = Some(kind);
            }
            if chosen.is_some_and(|c| c < 5) {
                break;
            }
        }
        let mut a = self.air_assist.unwrap_or_default();
        a.moved = 0.0;
        a.turned = 0.0;
        let Some(kind) = chosen else {
            self.air_assist = Some(a);
            return;
        };
        a.kind = Some(kind);
        let p = ASSIST_POINT[kind];
        let time = times[p].max(1.0);
        // Towards the rail, capped a tick and in all.
        let mut step = corr[p] * (1.0 / time);
        if step.length() > t.max_offset_step {
            step = step * (t.max_offset_step / step.length());
        }
        let mut offset = a.offset + step;
        if offset.length() > t.max_offset {
            offset = offset * (t.max_offset / offset.length());
        }
        a.moved = (offset - a.offset).length();
        a.offset = offset;
        // Turned towards along or across the rail: the deck as the rider lined it up (board adjust),
        // turned with the body.
        let h = heading(self.frame().fwd);
        let want = if ASSIST_ANGLE[kind] == 0.0 {
            0.0
        } else if h < 0.0 {
            -90.0
        } else {
            90.0
        };
        let max = t.max_angle_step;
        let turn = ((want - h) / time * yaw[kind]).clamp(-max, max) * (PI / 180.0);
        self.fwd = rotate(self.fwd, self.normal, turn);
        a.turned = turn;
        self.air_assist = Some(a);
    }
}

/// A grind's friction as a deceleration (m/s^2): Skate 3's force (0.48 x the rail's up y x the
/// family's strength on this class of rail x `FrictionVsTime` x the material) over [`GRIND_MASS`].
pub fn friction_decel(t: &Tuning, family: Family, class: RailClass, up_y: f32, time: f32) -> f32 {
    FRICTION_SCALE
        * up_y.max(0.0)
        * family.strengths()[class.column()]
        * t.grind.friction_vs_time.eval(time)
        * MATERIAL
        / GRIND_MASS
}

fn sticks_nonzero(x: f32) -> bool {
    x != 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_crosses_a_box_only_inside_it() {
        let corner = v3(0.0, 0.0, 0.0);
        let (e1, e2) = (v3(1.0, 0.0, 0.0), v3(0.0, -1.0, 0.0));
        let hit = segment_box(v3(0.5, -0.5, -1.0), v3(0.5, -0.5, 1.0), corner, e1, e2);
        assert!((hit.unwrap() - v3(0.5, -0.5, 0.0)).length() < 1e-5);
        assert!(segment_box(v3(1.5, -0.5, -1.0), v3(1.5, -0.5, 1.0), corner, e1, e2).is_none());
        assert!(segment_box(v3(0.5, -0.5, 0.2), v3(0.5, -0.5, 1.0), corner, e1, e2).is_none());
        // Parallel to the box: no crossing.
        assert!(segment_box(v3(-1.0, -0.5, 0.0), v3(2.0, -0.5, 0.0), corner, e1, e2).is_none());
    }

    #[test]
    fn grind_pops_are_the_physics_modes_per_family() {
        // SK8 grind/substate/settings.rs `vertical`: (1 - strength) x low + high x strength, from the
        // mode's pair for the family's group; Skate 3's Hardcore pops weakly at low strength.
        use crate::tuning::{Difficulty, Tuning};
        let mut t = Tuning::BUILT_IN;
        for (f, n) in [
            (Family::FiftyFifty, 2.5),
            (Family::FiveO, 2.5),
            (Family::Backslash, 2.5),
            (Family::Darkslide, 2.5),
            (Family::Boardslide, 2.61),
            (Family::Tipslide, 2.61),
        ] {
            assert_eq!(f.pop(&t.grind, 0.0), n);
            assert_eq!(f.pop(&t.grind, 1.0), n);
        }
        t.set_difficulty(Difficulty::Hardcore);
        let g = &t.grind;
        assert_eq!(Family::FiftyFifty.pop(g, 0.0), 0.5);
        assert_eq!(Family::Darkslide.pop(g, 1.0), 2.0);
        assert!((Family::Boardslide.pop(g, 0.5) - 1.805).abs() < 1e-5);
        assert_eq!(Family::Tipslide.pop(g, 0.0), 1.0);
        assert_eq!(Family::Tipslide.pop(g, 1.0), 2.0);
    }

    #[test]
    fn names_follow_the_grinds_features() {
        use Family::*;
        // 50-50: the side, BF when turned round from the approach.
        assert_eq!(
            name_of(false, false, true, false, false, true, FiftyFifty),
            "FS_50_50"
        );
        assert_eq!(
            name_of(true, false, true, false, true, true, FiftyFifty),
            "BF_BS_50_50"
        );
        // The back truck: straight is a 5-0, low a smith or feeble, angled a salad (or 5-0).
        assert_eq!(
            name_of(false, true, true, false, false, true, FiveO),
            "FS_5_O"
        );
        assert_eq!(
            name_of(false, true, false, true, false, true, FiveO),
            "FS_SMITH"
        );
        assert_eq!(
            name_of(false, true, false, true, false, false, FiveO),
            "FS_FEEBLE"
        );
        assert_eq!(
            name_of(false, true, false, false, false, false, FiveO),
            "FS_SALAD"
        );
        // The front truck: straight a nosegrind, angled a crooked grind or overcrook, low a willy.
        assert_eq!(
            name_of(true, false, true, false, false, true, FiveO),
            "BS_NOSEGRIND"
        );
        assert_eq!(
            name_of(true, false, false, false, false, true, FiveO),
            "BS_CROOK"
        );
        assert_eq!(
            name_of(true, false, false, false, false, false, FiveO),
            "BS_OVERCROOK"
        );
        assert_eq!(
            name_of(false, false, false, true, true, false, FiveO),
            "BF_FS_WILLY"
        );
        assert_eq!(
            name_of(false, false, false, true, false, true, FiveO),
            "FS_OVERWILLY"
        );
        // Slides: board or lip, nose or tail slide or blunt by facing; blunts and darkslides.
        assert_eq!(
            name_of(false, false, true, false, false, true, Boardslide),
            "FS_LIP"
        );
        assert_eq!(
            name_of(false, false, true, false, false, false, Boardslide),
            "FS_BOARD"
        );
        assert_eq!(
            name_of(true, true, true, false, false, false, Tipslide),
            "BS_TAILSLIDE"
        );
        assert_eq!(
            name_of(true, true, true, false, false, true, Tipslide),
            "BS_BLUNT"
        );
        assert_eq!(
            name_of(true, false, true, false, false, false, Tipslide),
            "BS_NOSEBLUNT"
        );
        assert_eq!(
            name_of(false, true, true, false, false, false, Backslash),
            "FS_BLUNT"
        );
        assert_eq!(
            name_of(false, false, true, false, false, false, Darkslide),
            "BS_DARKSLIDE"
        );
        for n in GRAPH_NAMES {
            assert!(
                n.ends_with("50_50") || GrindKind::of(n) != GrindKind::FiftyFifty,
                "{n}"
            );
        }
        assert_eq!(display_name("BF_FS_CROOK"), "FS CROOKED GRIND");
        assert_eq!(display_name("BS_BLUNT"), "BS BLUNTSLIDE");
    }

    #[test]
    fn a_new_name_scores_only_once_it_holds() {
        let mut n = Naming::default();
        n.update("FS_50_50", true);
        assert_eq!(n.scoring, Some("FS_50_50"));
        n.update("FS_5_O", false);
        assert_eq!(n.anim, Some("FS_50_50"), "shown from the next tick");
        n.update("FS_5_O", false);
        assert_eq!(n.anim, Some("FS_5_O"));
        for _ in 0..11 {
            n.update("FS_5_O", false);
        }
        assert_eq!(n.scoring, Some("FS_50_50"));
        n.update("FS_5_O", false);
        assert_eq!(n.scoring, Some("FS_5_O"), "held 12 ticks after the change");
    }
}
