//! The skater: what the rider does (stance, pushing, braking, powerslides, manuals, kick turns, reverts,
//! flick tricks, grabs, grinds, landings, bails) on top of the board's physics (`board`: one rigid board
//! on two trucks with a full velocity, riding whatever is under its wheels: flat ground, banks, curved
//! transitions and walls). Grinds (`grind`) follow Skate 3's contact boxes, families, controls and
//! names. In the air (`air`) the board flies the landing Skate 3's trajectory selector
//! picks at take-off, spins with Skate 3's body spin controller (which keeps a let-go spin going and,
//! with the stick let go, turns the body towards the landing) and turns its up to the landing surface
//! just before touching down. Landings are classified as Skate 3 does (`landing`); bails come only from
//! Skate 3's own landing rules; after one the rider is put back on a recent safe spot (`respawn`).
//! Deterministic, stepped at Skate 3's 60 Hz. The numbers come from [`Tuning`] (Skate
//! 3's own when the player's disc supplied them) and the push and brake timing from [`ClipTiming`] (Skate
//! 3's clips when the player imported the motion library).
//!
//! The rider's state logic below hands the board a [`Drive`] each tick: the steering, whether a pushing
//! or braking foot is on the ground and how hard, a powerslide's angle, a manual, a move that turns the
//! board itself. A state graph driven by Skate 3's own clips can produce the same inputs later.
//!
//! Conventions: `heading` is where the board's nose points (in the rider's frame: the end the rider's
//! front foot is on), `speed` is along it, negative when rolling tail first (fakie), `slip` sideways
//! (+ to the board's right). Riding switch means the rider stands with the other foot forward; the drawn
//! skater is then mirrored.

pub mod adjust;
mod air;
pub mod balance;
pub mod biped;
mod board;
pub mod climb;
pub mod dropin;
pub mod flip;
mod grind;
pub mod handplant;
pub mod hurdle;
mod landing;
pub mod meat;
pub mod plant;
pub mod possession;
pub mod ragdoll;
pub mod reckoning;
pub mod respawn;
pub mod rig;
pub mod stall;
pub mod states;
pub mod wipeout;
pub mod wobble;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub use air::{BodySpin, Contact as AirContact, Flight, GrindTarget, Plan};
pub use board::{
    AXLE, DECK_HEIGHT, DECK_UNDERSIDE, Drive, HANGER_DROP, Pump, TRUCK_DROP, WALLRIDE_MIN_HEIGHT,
    WHEEL_RADIUS, WallRide,
};
pub use grind::{
    Approach, ApproachPose, Assist, Category, Contact, Entry, Family, GRAPH_NAMES, GRIND_MASS,
    Grind, GrindKind, Jumper, Lean, display_name, name_of, segment_box,
};
pub use landing::Quality;
pub use respawn::{Checkpoints, Recovery};
pub use states::State;

use crate::flick::FlickOut;
use crate::gesture::{Flick, Group, Trick};
use crate::input::{Pad, btn};
use crate::math::{PI, Vec3, atan2f, clamp, cosf, sinf, smooth, sqrtf, v3, wrap_angle, yaw_dir};
#[cfg(test)]
use crate::park::Park;
use crate::park::Terrain;
use crate::tuning::Tuning;
use biped::{Biped, BipedInput};
use plant::{AirPlant, GroundPlant, PlantKind};
use rig::AirMotion;

/// Highest ledge a bailed rider walks up (m).
pub const STEP: f32 = 0.12;
/// Kick turns (ours, measured against SK8's simulation of Skate 3's `KickTurnSteering` at its stock
/// spin scale 0.5): the yaw rate the turn builds to against the board's speed (rad/s: 3.2 standing,
/// 2.7 at 1.8 m/s, 2.0 at 3.4 m/s), the share of the gap it closes a tick, and what is left of it a tick
/// after the stick is let go.
const KICKTURN_RATES: [(f32, f32); 4] = [(0.0, 3.2), (1.8, 2.72), (3.4, 2.0), (5.0, 1.6)];
const KICKTURN_BUILD: f32 = 0.09;
const KICKTURN_RELEASE: f32 = 0.7;
const KICKTURN_SPIN_SCALE: f32 = 0.5;
/// Rolling tail first faster than this is fakie at once; above `FAKIE_SLOW` after `FAKIE_TIME`; never
/// within `FAKIE_GRACE` of a spawn (Skate 3's riding-fakie rules).
const FAKIE_FAST: f32 = 1.0;
const FAKIE_SLOW: f32 = 0.5;
const FAKIE_TIME: f32 = 0.2;
const FAKIE_GRACE: f32 = 1.0;
/// How fast the board follows the animated centre of mass in the air: the share of the gap closed a
/// tick (SK8's rider body is physical and follows its animation; fit to its ollies' COM over the deck).
const AIR_LEGS_FOLLOW: f32 = 0.15;
/// How many air ticks after a jump a free board is given the jump's velocity (SK8 air_phase/board.rs:
/// `frames_since_jump_correction_2576 < 12`).
const JUMP_CORRECTION_TICKS: u32 = 12;
/// The wheels' friction on a landing's impact: sideways speed taken off per m/s into the ground.
const LAND_SIDE_FRICTION: f32 = 0.41;
/// Powerslides: the board angle eases at this rate (rad/s, ours); a held slide relaxes after this long
/// (s).
const SLIDE_TURN_RATE: f32 = 8.0;
const SLIDE_RELAX: f32 = 0.5;
/// Stepping off the board after a runout and back on.
const RUNOUT_TIME: f32 = 1.2;
/// SK8 DisableDismount (`motion_offboard/dismount.rs`): no getting off on ground whose up is below this.
const DISMOUNT_MIN_UP: f32 = 0.64;
/// Y's request lapses if the graphs haven't got off (or on) the board by then (s).
const TOGGLE_WAIT: f32 = 2.0;
/// How long a landing is remembered (drawn: the landing clip).
const LANDING_SHOW: f32 = 1.5;
/// A spin landing keeps pivoting the board this long (s, ours), fading as a wall hit's turn does.
const SPIN_PIVOT_TIME: f32 = 0.15;
/// The C-stick past this picks a grab's variant (Skate 3's dead zone for `ScoringGrabs`).
const GRAB_VARIANT_DEAD: f32 = 0.5;
const MANUAL_MIN_TIME: f32 = 0.5;
/// Analog trigger past this holds a grab: any pressure, as Skate 3 and the state graphs' grab intents
/// (`graph::input`) count it; the pad's trigger dead zone (`input::TRIGGER_DEAD_ZONE`) already took out a
/// trigger at rest, so the physics and the graphs agree on when the rider grabs.
const GRAB_TRIGGER: f32 = 0.0;
/// Both push buttons pressed within this of each other wind up a hippie jump.
const HIPPY_CHORD: f32 = 0.08;

/// Which foot the rider naturally has forward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stance {
    /// Right foot forward: Skate 3's default.
    #[default]
    Goofy,
    Regular,
}

/// The yaw rate a kick turn builds to at `speed` (rad/s, [`KICKTURN_RATES`]).
pub fn kickturn_rate(speed: f32) -> f32 {
    let r = KICKTURN_RATES;
    if speed <= r[0].0 {
        return r[0].1;
    }
    for w in r.windows(2) {
        let ((a, ra), (b, rb)) = (w[0], w[1]);
        if speed <= b {
            return ra + (rb - ra) * (speed - a) / (b - a);
        }
    }
    r[r.len() - 1].1
}

pub fn trick_duration(t: Trick) -> f32 {
    let (flip, shove) = t.motion();
    match (flip != 0.0, shove.abs()) {
        (false, 0.0) => 0.0,
        (true, 0.0) => 0.42,
        (false, s) if s <= 1.0 => 0.38,
        (false, _) => 0.5,
        (true, s) if s <= 1.0 => 0.48,
        (true, _) => 0.56,
    }
}

/// Skate 3's landing types (`landing::classify`): straight (also when slightly off, or slow), sketchy,
/// or a spin landing (still turning towards the travel: the board keeps pivoting a moment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandKind {
    Clean,
    /// Board crooked to the travel direction; `frontside` when it is turned towards the rider's chest.
    Sketchy {
        frontside: bool,
    },
    Spin,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub kind: LandKind,
    /// Seconds since touching down.
    pub time: f32,
    /// How far the board was off the travel direction (rad), and how fast it hit the ground (m/s, down).
    pub crooked: f32,
    pub impact: f32,
    /// Skate 3's landing quality (type 0..3, signed magnitude).
    pub quality: Quality,
}

/// Which hand grabs (flatground's own rules; with the state graphs their grab states decide).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrabKind {
    /// Rear hand, toe side (frontside).
    Indy,
    /// Front hand, heel side (backside).
    Melon,
    /// Both hands.
    Double,
}

impl GrabKind {
    /// The grab's name with the C-stick centred, and its variants with the C-stick up, left, down and
    /// right (flatground's own table; the state graphs carry Skate 3's).
    pub fn names(self) -> (&'static str, [Option<&'static str>; 4]) {
        match self {
            GrabKind::Indy => ("INDY", [Some("TAILGRAB"), None, Some("STALEFISH"), None]),
            GrabKind::Melon => ("MELON", [Some("NOSEGRAB"), None, Some("MUTE"), None]),
            GrabKind::Double => ("DOUBLE GRAB", [None; 4]),
        }
    }

    pub fn name(self) -> &'static str {
        self.names().0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grab {
    pub kind: GrabKind,
    /// Seconds held.
    pub time: f32,
    /// The scored name: the variant the C-stick picks (or the state graph's `ScoringGrabs`). A variant
    /// once picked stays when the stick comes back to the middle (`base`: the name it started with).
    pub name: &'static str,
    pub base: &'static str,
    /// How far the C-stick tweaks it (0..1).
    pub tweak: f32,
}

/// Skate 3's grab variant pick (SK8 `select_grab_score`): within `GRAB_VARIANT_DEAD` of the middle the
/// base name, else the variant in the stick's quarter (up, left, down, right; a quarter grows into a
/// missing neighbour's). `x` right, `y` up.
pub fn select_grab<'a>(base: &'a str, variants: [Option<&'a str>; 4], x: f32, y: f32) -> &'a str {
    grab_variant(variants.map(|v| v.is_some()), x, y)
        .and_then(|i| variants[i])
        .unwrap_or(base)
}

/// Which of the variants (up, left, down, right; `has` says which exist) the stick at (`x`, `y`) picks,
/// or `None` for the base grab.
pub fn grab_variant(has: [bool; 4], x: f32, y: f32) -> Option<usize> {
    let [up, left, down, right] = has;
    if sqrtf(x * x + y * y) <= GRAB_VARIANT_DEAD {
        return None;
    }
    let mut angle = atan2f(y, x);
    if angle < 0.0 {
        angle += 2.0 * PI;
    }
    let windows = [
        (
            up,
            if right { 44.0 } else { 359.0 },
            if left { 134.0 } else { 181.0 },
        ),
        (
            left,
            if up { 44.0 } else { 89.0 },
            if down { 226.0 } else { 271.0 },
        ),
        (
            down,
            if left { 224.0 } else { 179.0 },
            if right { 316.0 } else { 1.0 },
        ),
        (
            right,
            if down { 314.0 } else { 269.0 },
            if up { 46.0 } else { 91.0 },
        ),
    ];
    for (i, (exists, start, end)) in windows.into_iter().enumerate() {
        let (start, end) = (start * PI / 180.0, end * PI / 180.0);
        let inside = if end < start {
            angle >= start || angle <= end
        } else {
            angle >= start && angle <= end
        };
        if inside && exists {
            return Some(i);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Manual {
    /// On the front wheels.
    pub nose: bool,
    /// -1..1: 0 is balanced; reaching -1 or 1 drops the manual.
    pub balance: f32,
    pub time: f32,
    /// 0 or 1: dragging the tail (or nose) to brake.
    pub brake: f32,
    /// Deck pitch (rad): down to the brake angle while braking.
    pub pitch: f32,
    /// Balance servo state (state graphs): the correction turning the deck each tick (rad per tick)
    /// and the filtered pitch error.
    pub correction: f32,
    pub filtered_error: f32,
    /// This tick Skate 3's Normal corrective drag acts at the dragging truck instead of the brake
    /// (SK8 manual/controller.rs `apply_correction`, `correction_active`).
    pub correcting: bool,
    /// The deck turn the controller asked for last tick (rad per 1/60 s tick).
    pub last_step: f32,
    /// The deck's pitch when the controller last measured it: its change is the derivative's
    /// angle change (SK8 manual/controller.rs `measured_angle`).
    pub measured: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveTrick {
    pub flick: Flick,
    /// Seconds since the pop.
    pub t: f32,
    /// Total board animation time (grows with extra flips).
    pub duration: f32,
    /// Flips of a held kickflip or heelflip (1 otherwise).
    pub flips: u8,
    /// With the state graphs: how far through the trick its clips are (0..1, 1 done). One clock: the
    /// clip's, not `duration`.
    pub clock: Option<f32>,
    /// Done after the first trick (a late flip, a finger flip).
    pub late: bool,
    /// Caught dark (grip down).
    pub dark: bool,
    /// The graph's name for it (string id), to tell a new trick from the one playing.
    pub id: u16,
}

impl ActiveTrick {
    pub fn new(flick: Flick) -> ActiveTrick {
        ActiveTrick {
            flick,
            t: 0.0,
            duration: trick_duration(flick.trick),
            flips: 1,
            clock: None,
            late: false,
            dark: false,
            id: u16::MAX,
        }
    }

    /// 0..1 progress of the board animation.
    pub fn progress(&self) -> f32 {
        if let Some(c) = self.clock {
            return c.clamp(0.0, 1.0);
        }
        if self.duration <= 0.0 {
            1.0
        } else {
            (self.t / self.duration).min(1.0)
        }
    }

    /// Skate 3's trick id for it: the flip (doubles up to quadruples of kick and heel flips, the nollie
    /// and late versions, a dark catch) as its scoring catalogue names it.
    pub fn scorable(&self) -> Option<u16> {
        crate::scoring::catalog::flip_id(self.flick, self.flips, self.late, self.dark)
    }

    pub fn name(&self) -> String {
        let base = if self.flick.trick == Trick::Ollie && self.flick.nollie {
            String::from("NOLLIE")
        } else if self.flick.nollie {
            format!("NOLLIE {}", self.flick.trick.name())
        } else {
            String::from(self.flick.trick.name())
        };
        let base = match self.flips {
            2 => format!("DOUBLE {base}"),
            3 => format!("TRIPLE {base}"),
            4 => format!("QUADRUPLE {base}"),
            _ => base,
        };
        let base = if self.late {
            format!("LATE {base}")
        } else {
            base
        };
        if self.dark {
            format!("{base} DARK CATCH")
        } else {
            base
        }
    }
}

/// One set of push clips (Skate 3 blends four: slow or fast, strong or gentle): how long the foot takes
/// to come down, stays down and swings back (s), and the speeds the push starts and ends at (m/s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushSet {
    pub into: f32,
    pub contact: f32,
    pub recover: f32,
    pub vel_b: f32,
    pub vel_e: f32,
}

/// Timing the rider's clips give the physics: the push sets (slow strong, fast strong, slow gentle, fast
/// gentle) and the brake foot's time to touch down. Skate 3's own when the motion library was imported
/// (`ClipTiming::from_motion`), else flatground's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipTiming {
    pub push: [PushSet; 4],
    pub brake_down: f32,
}

/// Skate 3's push clips by set (as `ClipTiming::push`), each `_INTO`, `_CYC1` (the foot on the ground)
/// and `_CYC2` (the swing back).
const PUSH_CLIPS: [&str; 4] = [
    "R_PUSHLSP_HSTR_N_0",
    "R_PUSHHSP_HSTR_N_0",
    "R_PUSHLSP_LSTR_N_0",
    "R_PUSHHSP_LSTR_N_0",
];
const BRAKE_INTO_CLIP: &str = "R_BRAKE_N_N_0_INTO";

impl ClipTiming {
    /// flatground's own timing.
    pub const BUILT_IN: ClipTiming = ClipTiming {
        push: [
            PushSet {
                into: 0.45,
                contact: 0.15,
                recover: 0.8,
                vel_b: 0.0,
                vel_e: 4.0,
            },
            PushSet {
                into: 0.45,
                contact: 0.1,
                recover: 0.8,
                vel_b: 8.0,
                vel_e: 10.0,
            },
            PushSet {
                into: 0.45,
                contact: 0.15,
                recover: 0.8,
                vel_b: 0.0,
                vel_e: 1.5,
            },
            PushSet {
                into: 0.45,
                contact: 0.1,
                recover: 0.8,
                vel_b: 8.0,
                vel_e: 8.5,
            },
        ],
        brake_down: 0.15,
    };

    /// Reads the push and brake clips' lengths and speeds from the player's motion library; anything
    /// missing keeps the built-in value.
    pub fn from_motion(db: &crate::motion::MotionDb) -> ClipTiming {
        let mut c = ClipTiming::BUILT_IN;
        let len = |name: &str| db.clip_index(name).map(|i| db.clip(i).length());
        let attr = |name: &str, a: &str| {
            let i = db.clip_index(name)?;
            db.attribute_at(i, a, 0.0).map(|v| v.value)
        };
        for (set, base) in c.push.iter_mut().zip(PUSH_CLIPS) {
            let into = format!("{base}_INTO");
            let cyc1 = format!("{base}_CYC1");
            let cyc2 = format!("{base}_CYC2");
            if let (Some(a), Some(b), Some(r)) = (len(&into), len(&cyc1), len(&cyc2))
                && a > 0.0
                && b > 0.0
                && r > 0.0
            {
                set.into = a;
                set.contact = b;
                set.recover = r;
            }
            if let Some(v) = attr(&cyc1, "HSTR_VEL_B").or_else(|| attr(&cyc1, "LSTR_VEL_B")) {
                set.vel_b = v;
            }
            if let Some(v) = attr(&cyc1, "VEL_E") {
                set.vel_e = v;
            }
        }
        if let Some(b) = len(BRAKE_INTO_CLIP)
            && b > 0.0
        {
            c.brake_down = b;
        }
        c
    }

    /// The strong and gentle sets blended for `speed`: (into, contact, recover, gentle end speed, strong
    /// end speed, the speeds the slow and fast sets start at).
    fn blend(&self, speed: f32) -> (f32, f32, f32, f32, f32, f32, f32) {
        let k = |a: &PushSet, b: &PushSet| {
            let w = b.vel_b - a.vel_b;
            if w.abs() < 1e-3 {
                0.0
            } else {
                ((speed - a.vel_b) / w).clamp(0.0, 1.0)
            }
        };
        let [ss, fs, sg, fg] = &self.push;
        let ks = k(ss, fs);
        let kg = k(sg, fg);
        let l = |a: f32, b: f32, k: f32| a + (b - a) * k;
        (
            l(ss.into, fs.into, ks),
            l(ss.contact, fs.contact, ks),
            l(ss.recover, fs.recover, ks),
            l(sg.vel_e, fg.vel_e, kg),
            l(ss.vel_e, fs.vel_e, ks),
            ss.vel_b,
            fs.vel_b,
        )
    }
}

impl Default for ClipTiming {
    fn default() -> Self {
        Self::BUILT_IN
    }
}

/// Braking with the foot (X).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FootBrake {
    pub time: f32,
    /// Let go: the foot comes back up (seconds since).
    pub released: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Powerslide {
    /// +1 sliding with the stick to the right (the board turned one way), -1 left.
    pub side: f32,
    /// 0..1 how far the board has turned sideways (1: square across).
    pub amount: f32,
    pub time: f32,
    /// Let go: the board straightens out.
    pub releasing: bool,
    /// The board's angle from travel (rad, signed like `side`).
    pub angle: f32,
    /// How long the board has looked "not sliding" (nearly in line).
    pub not_sliding: f32,
}

/// A revert: the board turned half round on its wheels by Skate 3's yaw servo (the travel kept).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Revert {
    /// +1 turning the nose towards +X seen from above (left), -1 the other way.
    pub side: f32,
    pub time: f32,
    /// How far round it has turned (rad) and how fast it turns (rad/s).
    pub turned: f32,
    pub rate: f32,
}

/// Something worth telling the player (and the tests) about.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Popped(Flick),
    Launched,
    /// A trick landed: its name as shown, what it was (for Skate 3's scoring) and its raw reward (filled
    /// in by the game's scoring; 0 from the skater).
    Landed {
        name: String,
        points: u32,
        score: crate::scoring::Scored,
    },
    Bailed,
    /// Stepped off the board after a hit (no respawn).
    RanOut,
    Respawned,
    /// Tipped out of a manual (no points, a little speed lost).
    ManualLost,
    /// Turned round from fakie to push switch.
    Switched,
    /// Got off the board to walk (Y), and back on it.
    SteppedOff,
    SteppedOn,
    /// Jumped off a foot planted on the ground (a boneless, fastplant or no-comply).
    FootJump(plant::PlantKind),
    /// Planted a foot in the air (on a ledge, the coping, a wall), and flew off it.
    FootPlanted,
    FootReleased,
    /// Dropping in on foot (`skater::dropin`): the board's tail set on the coping, then let go into the
    /// transition; or stepped away from the edge again.
    DropInSet,
    DroppedIn,
    DropInCancelled,
    /// A hand plant (`skater::handplant`): the hand on the coping, and let go.
    HandPlanted,
    HandReleased,
    /// A session marker went down where the rider is (`crate::marker`), or could not go down here.
    MarkerSet,
    MarkerRefused,
    /// Back at the session marker.
    MarkerReturned,
}

/// Stance and how the rider took off (for names), the surface left and whether the pop was re-aimed up
/// a vert face (for the landing prediction).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Takeoff {
    fakie: bool,
    switch: bool,
    hippy: bool,
    /// Got on the board in the air off it (Y jumping off the board): coming down onto it.
    mount: bool,
    /// Coming down onto the free board (`skater::possession`) from a jump off the board: Skate 3's
    /// landing on deck, the caveman.
    onto_free: bool,
    ground: Option<Vec3>,
    on_vert: bool,
    lean: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skater {
    pub tuning: Tuning,
    pub clips: ClipTiming,
    pub stance: Stance,
    pub pos: Vec3,
    /// Where the board's nose points: a unit vector along the surface (in the air, the board's own nose).
    pub fwd: Vec3,
    /// The nose's direction seen from above (yaw), kept in step with `fwd`.
    pub heading: f32,
    /// Speed along the nose on the ground (negative: tail first), and sideways (+ right).
    pub speed: f32,
    pub slip: f32,
    /// The velocity: in the air the flight; on the ground the same as `speed` and `slip` along the
    /// surface (kept in step every tick).
    pub vel: Vec3,
    /// The board's up: on the ground the surface's normal smoothed, in the air its own attitude.
    pub normal: Vec3,
    /// The ground's normal under the trucks (unsmoothed: what the physics rides on).
    pub contact_n: Vec3,
    /// Skate 3's physical state, the selector's counters and this tick's facts for it
    /// (`skater::states`).
    pub states: states::States,
    /// Seconds since the board left the ground (in the air).
    pub air_time: f32,
    /// Extra yaw spun in the air (about the board's up), relative to `heading` at take-off.
    pub spin: f32,
    pub spin_rate: f32,
    /// Skate 3's body spin controller (the stick's spin, its wind-up, the landing assist's spin).
    pub body: BodySpin,
    /// The landing prediction: the query in progress, and the landing the board flies to once known.
    pub selector: air::Selector,
    pub plan: Option<Plan>,
    pub trick: Option<ActiveTrick>,
    /// Riding with the other foot forward (the drawn rider is mirrored).
    pub switch: bool,
    /// The board's frame (`fwd`, and the rider's frame drawn on it) has been turned end for end an odd
    /// number of times (`Skater::flip_board`: fakie to switch). The physical board never turns (Skate 3
    /// flips only its "effective" frame, SK8 board_motion_output.rs); animation trees started before a
    /// turn are drawn turned back by half a turn so nothing turns in the world.
    pub frame_turned: bool,
    /// Rolling tail first (Skate 3's fakie detection: a moment of backwards speed).
    pub fakie: bool,
    /// Riding dark (the board grip down after a dark catch; the state graphs decide).
    pub dark: bool,
    fakie_timer: f32,
    /// Seconds since the rider was put on the board (spawn or respawn).
    pub since_spawn: f32,
    takeoff: Takeoff,
    /// Steering is weaker while a pushing foot is down (1 = full).
    push_steer: f32,
    /// Smoothed turn input, -1..1 (drawn: the rider leans into turns).
    pub steer: f32,
    /// The stick's turn this tick as Skate 3 reads it (-1..1, positive to the left like `steer`;
    /// unsmoothed), and the kick-turn or spin input: the camera's steering takes the larger when they
    /// agree with `steer` (SK8 manager_subject.rs `steering_for_turn`).
    pub turn_input: f32,
    pub kick_input: f32,
    /// The velocity's change over the last tick (m/s^2; bumps), and the velocity it was taken from.
    pub accel: Vec3,
    prev_vel: Vec3,
    /// Rolling on both trucks: the line from the back truck's contact to the front's (towards the
    /// nose), which the board is drawn along.
    pub chord: Option<Vec3>,
    /// The deck's tilt that turns the trucks (rad), and seconds since the wheels came down (the trucks'
    /// turn ramps back in).
    pub deck_tilt: f32,
    truck_ramp: f32,
    /// The board's yaw rate last tick (rad/s).
    pub yaw_rate: f32,
    /// The trucks' part of it, building up towards what the deck tilt asks (rad/s).
    truck_yaw: f32,
    /// A wall hit turning the board along the wall: yaw rate (rad/s) and seconds left.
    collide: Option<(f32, f32)>,
    /// Seconds since the last wall hit.
    wall_clock: f32,
    /// Speed away from the surface while the wheels are lifted (m/s), and how long the gap has been
    /// over Skate 3's short natural-air distance.
    vn: f32,
    gap_time: f32,
    /// How much the leading truck climbed last tick (m).
    lead_rise: f32,
    pub pump: Pump,
    /// Seconds since the player last touched anything (rolling resistance rises after a while).
    idle: f32,
    /// The speed the speed model holds the board to (m/s, Skate 3's `UpdateSpeedModel` target): only
    /// the slope and rolling resistance move it, so the speed the wheels lose turning or scrubbing
    /// sideways is put back (within `friction_max_change`).
    target_speed: f32,
    /// The speed the last ride ended with (a different one at the next is a change from outside).
    model_speed: f32,
    /// 0..1, how far the stick asks the skater to crouch (Skate 3's `Crouch` intent).
    pub crouch: f32,
    /// Without the imported rider: Skate 3's Crouching (SK8 crouching.rs, the graph's
    /// `disttocog`) run on the stick's crouch, and the centre of mass over the board it gives (m).
    stick_crouch: Option<crate::graph::riding::Crouch>,
    crouch_com: Option<f32>,
    crouch_pump: bool,
    /// The crouch is for a nollie (stick held forward).
    pub crouch_nose: bool,
    pub landing: Option<Landing>,
    pub manual: Option<Manual>,
    /// How long the C-stick has been in the manual zone.
    manual_engage: f32,
    /// After tipping out of a manual, the stick has to come back before another starts.
    manual_lock: bool,
    /// The physics asks the state graph to end the manual (`PhysicsWantsManualExit`): dragging the
    /// tail or nose while the board rolls against the manual's balance direction, with the physics
    /// mode's corrective force off (SK8 manual/controller.rs `apply_correction`).
    pub manual_exit: bool,
    /// Skate 3's speed wobble on the ground (`skater::wobble`).
    pub wobble: wobble::SpeedWobble,
    /// Skate 3's ground reckoning (`skater::reckoning`): the dynamic normal from the board's
    /// acceleration, the rider's up and his lean into a turn (what the turn conditioner reads).
    pub dynamic_normal: reckoning::DynamicNormal,
    pub reckoning: reckoning::Reckoning,
    /// Skate 3's rigid board while riding on the ground (`skater::rig`, `physics::board`).
    pub rig: Option<rig::Rig>,
    pub grab: Option<Grab>,
    /// The longest grab of this jump that was let go of (scored on landing).
    grabbed: Option<Grab>,
    pub grind: Option<Grind>,
    /// A rail just leaned off and how long until it can be locked onto again.
    regrind_block: Option<(usize, f32)>,
    /// Grind-out energy and the time since the last pop out of a grind (SK8 `Jumper`).
    pub jumper: Jumper,
    /// Where the rider came from, for a grind's FS/BS (SK8 chromosome history).
    pub approach: Approach,
    /// The grind air assist towards the rail the flight was steered onto.
    pub air_assist: Option<Assist>,
    /// Why the last grind ended (for the log).
    pub grind_exit: Option<&'static str>,
    pub brake: Option<FootBrake>,
    pub slide: Option<Powerslide>,
    /// Last tick's powerslide reading of the stick (entry window side, continuous amount).
    last_slide: (f32, f32),
    pub revert: Option<Revert>,
    /// Kick-turning on the spot: direction (+1 left), seconds.
    pub kickturn: Option<(f32, f32)>,
    /// The kick turn's yaw rate (rad/s, + left), running down after the stick is let go.
    kick_rate: f32,
    /// Winding up a hippie jump: seconds held.
    pub hippy: Option<f32>,
    /// When each push button last went down (seconds ago), for the hippie jump chord.
    since_a: f32,
    since_b: f32,
    /// Why the last bail happened (for the log).
    pub bail_reason: Option<&'static str>,
    /// Riding a wall.
    pub wallride: Option<WallRide>,
    /// Recent safe spots to be put back on after a bail, ticks ridden in a row, and the bail's clock.
    pub checkpoints: Checkpoints,
    riding_ticks: u32,
    pub recovery: Recovery,
    /// The C-stick's y this tick (+ away from the player, Skate 3's `PhysGrindUpDown`) and both sticks'
    /// x (`PhysGrindTranslation`): a grind's pitch and yaw.
    grind_pitch_input: f32,
    grind_sticks_x: f32,
    /// The numbers behind the last landing check (for the log).
    pub last_landing: Option<LandingCheck>,
    /// What Skate 3's state graphs decided this tick (`crate::graph`; the game sets it every tick).
    pub graph: GraphControl,
    /// A body flip in the air (`skater::flip`), and the last one landed (front, whole turns) for the
    /// landing's name.
    pub body_flip: Option<flip::BodyFlip>,
    flipped: Option<(bool, u32)>,
    /// A graph-driven push in contact: the speed it started at, the speed it chases, seconds left.
    graph_push: Option<(f32, f32, f32)>,
    /// With the imported rider: the rider's centre-of-mass height over the board (m). It is the
    /// animation's while riding; from the pop's tail strike and in the air the body follows its
    /// animation only so fast (SK8's rider is physical). And in the air, its height at take-off.
    pub body_com: Option<f32>,
    /// A microsecond clock from the platform (none: no timing), and how long the rigid board's
    /// solve took this tick.
    pub clock: Clock,
    pub board_us: u32,
    /// The rider's physical body (SK8's skeleton in the board's solve): on with an imported rider's
    /// animated pose, `rider_pose` (each physical body's frame on the deck, set each tick by the
    /// game), off without one. `body_us`: its step's cost this tick (inside the board's).
    pub rider_body: Option<alloc::boxed::Box<crate::physics::rider_body::RiderBody>>,
    pub rider_pose: Option<[crate::math::Xform; crate::physics::skeleton::ANIMATED_PARTS]>,
    /// The axes the rider's pose stands on (the reckoning's up, the rider's heading).
    pub rider_axes: crate::math::Mat3,
    /// The clips' board in the world as Skate 3's root frames place it (set each tick by the game with
    /// an imported rider): what the air's hook drive pulls the physical deck's turn to.
    pub anim_board: Option<crate::math::Mat3>,
    pub body_us: u32,
    /// The board was stepped this tick.
    board_stepped: bool,
    /// The last jump's velocity and the air ticks since it (SK8 `jump_velocity_848`,
    /// `frames_since_jump_correction_2576`): a free board is given it for its first ticks.
    jump: Option<(Vec3, u32)>,
    air_com: Option<f32>,
    /// How far the legs have lifted the board over the flight's path (m, world up), and how fast it
    /// moved last tick (m/s).
    pub air_lift: f32,
    air_lift_rate: f32,
    /// Off the board (the off-board states): the walker.
    pub biped: Biped,
    /// Climbing ledges on foot (SK8's own extension: `climb`).
    pub climb: climb::Climber,
    /// The camera's yaw: the walker reads the stick against it (the game sets it each tick).
    pub cam_yaw: f32,
    /// Y asked to get off the board (riding) or on it (walking), seconds ago: the graphs' clips decide
    /// the moment (`toggle_board`).
    pub board_toggle: Option<f32>,
    /// The mount clip's `OB_TRAJ` has started (the rider is on the board when it ends).
    mount_traj: bool,
    /// Board adjust (`skater::adjust`): the deck's yaw and pitch against the body, in the air only.
    pub deck_adjust: adjust::DeckAdjust,
    /// Where the board is when it isn't under the feet: in the hands, free in the park, hidden, flying
    /// back (`skater::possession`).
    pub possession: possession::Possession,
    /// Off the board in the air, Skate 3's landing assist has taken a flight down onto the free board
    /// (`CanLandOnBoard`): the landing on it starts once the motion graph's `DoLandOnBoard` says so (at
    /// once the graph says so).
    pub deck_assist: bool,
    /// A hippie jump's landing on deck: something in the way of the landing on the board, and the
    /// hippie hurdle (`IsHippyHurdling`; `skater::hurdle`).
    pub hurdle: hurdle::Hurdle,
    /// A foot plant on the ground (`skater::plant`): a boneless, fastplant or no-comply, from the planted
    /// foot to the jump off it.
    pub ground_plant: Option<plant::GroundPlant>,
    /// The ground plant this air jumped off (scored on landing).
    pub plant_trick: Option<plant::PlantKind>,
    /// In the air: Skate 3's foot plant manager (running in one-foot air), the swing over a planted foot
    /// (SK8 physical state 601), and the foot plant's scored name this air (`fsgrab_fs_footplant` ...).
    pub footplant: plant::Manager,
    pub air_plant: Option<plant::AirPlant>,
    pub footplanted: Option<&'static str>,
    /// Seconds since take-off, or since the last foot plant flew off (Skate 3's air state's clock).
    plant_clock: f32,
    /// Off the board: the edge ahead of the feet (`skater::dropin::find_edge`, Skate 3's `DistToEdge`),
    /// and a drop-in on foot under way.
    pub edge: Option<dropin::Edge>,
    pub drop_in: Option<dropin::DropIn>,
    /// Hand plants (`skater::handplant`): the manager looking for a coping riding up a ramp with the
    /// world grab held, and the plant itself (SK8 physical state 600).
    pub handplant: handplant::Manager,
    pub hand_plant: Option<handplant::Plant>,
    /// The name the graphs score the hand plant under, latched while they play it.
    hand_plant_name: Option<&'static str>,
    /// The stall the last grind was (`skater::stall`), if it was one.
    pub stalled: Option<stall::Stall>,
    /// The body in a bail (`skater::wipeout`: Skate 3's ragdoll wipeout), and Hall of Meat's injuries.
    pub wipeout: wipeout::Wipeout,
    /// Where the drawn rider's ragdoll points are (world), set by the game from the imported rider's
    /// pose each frame: a bail starts the body there (else from the placeholder's standing pose).
    pub pose_seed: Option<[Vec3; ragdoll::POINTS]>,
    /// The triggers fully in last tick (Skate 3's bail request and push off read their edges), and
    /// whether the bail request (`WipeOutRequest`) and the push off (`WipeOutPushOff`) came this tick.
    wipe_full: [bool; 2],
    pub wipe_request: bool,
    pub wipe_push_off: bool,
}

/// The state graphs' decisions for the physics (see `graph::drive::Driver::control`).
/// The platform's microsecond clock, for timing (none: no timing). Equal whatever it holds: it
/// isn't part of the simulation's state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock(pub Option<fn() -> u32>);

impl PartialEq for Clock {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Clock {
    /// Microseconds now (0 without a clock).
    pub fn now(self) -> u32 {
        self.0.map_or(0, |f| f())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GraphControl {
    /// A push clip's foot is on the ground (its `PUSHCONTACT` window): only then does a push drive.
    pub push_contact: bool,
    /// The speed the push chases while the foot is down: where the push trees' coefficients end
    /// (`graph::push`), or with `push_posed` the posed clips' trajectory speed this tick.
    pub push_target: Option<f32>,
    /// The push's clips are playing from the foot's first contact to the push's end (Skate 3's push
    /// contact events, `ProcessedPhysIn` bit 27, which weakens the steering).
    pub pushing: bool,
    /// `push_target` is the posed trajectory's speed (Skate 3's): it holds this tick only, as SK8's
    /// push contact does; else the push sets' end speed, held for the contact's length.
    pub push_posed: bool,
    /// The animated centre of mass's height over the posed board (m, the rider's last pose; SK8
    /// `com_to_deck`): what a jump pops from and, in the air, how far the legs have tucked the board
    /// up. `None` without the imported rider.
    pub pose_com: Option<f32>,
    /// How far the posed board's centre is lifted off its riding height (m): the pop's tail strike
    /// pitches it up about its back wheels before the jump.
    pub pose_lift: f32,
    /// A brake clip's foot is down (`BRAKE_CONTACT`): only then does the foot brake slow the board.
    pub brake_contact: bool,
    /// The takeoff clip reached its jump: pop now, as this trick, at this strength (0..1).
    pub pop: Option<(Flick, f32)>,
    /// The takeoff clip marks the pop as a trick out of a manual (`TrickFromManual`).
    pub pop_from_manual: bool,
    /// Manualing (nose manual), the stick's lean (0..1) and whether it is dragging to brake.
    pub manual: Option<(bool, f32, bool)>,
    /// Kick-turn steering is on (spin scale).
    pub kickturn: Option<f32>,
    /// A revert gesture (true: frontside).
    pub revert: Option<bool>,
    /// The graph turns the rider round (riding fakie into switch, or pushing out of fakie).
    pub turn_round: bool,
    /// The trick the graph is animating in the air (`None`: none, or it is done).
    pub air_trick: Option<AirTrick>,
    /// The playing clips are in their `DANGERZONE`: the board isn't caught yet, landing now bails.
    pub danger: bool,
    /// Dark: the board was caught grip down (a dark catch), and is ridden that way until the graph
    /// says otherwise.
    pub dark: bool,
    /// The motion graph went to its `WipeOut` state on its own (a grab tweaked into the landing): the
    /// physics bails with it.
    pub wipeout: bool,
    /// The wipeout gesture the motion graph's `Wipeout` behaviour blended (`WipeoutGestureX/Y`): the
    /// physics picks its control profile from it.
    pub wipe_gesture: [f32; 2],
    /// The grab's scored name (`ScoringGrabs`: the variant the C-stick picks), and the tweak's size
    /// (`TweakX`, `TweakY`).
    pub grab_name: Option<&'static str>,
    pub tweak: f32,
    /// The stick's spin as the graph hands it to the physics (`PhysBodySpin`; Skate 3's body spin
    /// controller reads it on the ground and in the air).
    pub body_spin: f32,
    /// The graph's `Turn` packet (SetTurning's), which Skate 3's physics takes as its processed turn
    /// (negated: SK8 scalar_attributes.rs).
    pub turn_packet: Option<f32>,
    /// The playing clips carry `Anticipating` (the ollie wind-up): Skate 3's camera reads it as
    /// `IsPreparingToJump`.
    pub anticipating: bool,
    /// A body flip the graphs ask for (`FrontFlip`: true, `BackFlip`: false) and a grab held
    /// (`Grabbing`, `WantsLeftAirGrab`, `WantsRightAirGrab`).
    pub body_flip: Option<bool>,
    pub grabbing: bool,
    /// Off the board: the playing clips carry `OB_TRAJ` (Skate 3's physics walks the rider while they
    /// do), and the motion graph is in its `Dismount` or `Mount` states.
    pub ob_traj: bool,
    pub dismounting: bool,
    pub mounting: bool,
    /// The action graph is adjusting the board (its BoardAdjust behaviour publishes the C-stick to the
    /// motion graph's board-adjust clips).
    pub board_adjust: bool,
    /// The motion graph is in its board-adjust states (`/BoardAdjusts/`): let go, their clips still
    /// pose the deck turned as they blend back, and Skate 3's physical board follows the posed one.
    pub adjust_posed: bool,
    /// Off the board, ToggleBoard's throw clip let go of the board this tick (`OB_RELEASEBOARD`: a drop,
    /// `Some(false)`; `OB_THROWBOARD`: a throw, `Some(true)`).
    pub release_board: Option<bool>,
    /// ToggleBoard published `OB_RetrieveBoard`: the board flies back to the hands.
    pub retrieve_board: bool,
    /// LandOnBoard asked for the board to be taken the other way round (its tail ahead).
    pub land_backwards: bool,
    /// The motion graph's `OB_Mounting` (its mount states, from the `OB_Mount` intent): Skate 3's physics
    /// fetches a free board into the hands for the mount.
    pub ob_mounting: bool,
    /// The motion graph's `DoLandOnBoard` (its `IntoLandingOnBoard` and `LandingOnBoard` states): the
    /// physics comes down onto the board (SK8: biped air into LandingOnDeck).
    pub land_on_board: bool,
    /// Crouching's `PlayerControlledPump` (the rider crouched or pumped within half a second): Skate
    /// 3's intentional pumping (SK8 Processed2476 bit 1), which pays in full.
    pub player_pump: bool,
    /// The posed board's nose in the rider's frame (x `up x nose`, y up, z ahead) from the imported
    /// rider's last pose: where the clips have put the deck. `None` without the imported rider.
    pub pose_deck: Option<Vec3>,
    /// Grinding, the grind grab the motion graph's grind-grab states score (`FS_5_O_FSGRAB`: `FSGRAB`)
    /// and the 5-0's free end they hold up (`PhysGrindGrabMinHeight`, 0 when none).
    pub grind_grab: Option<&'static str>,
    pub grind_grab_min_height: f32,
    /// Foot plants on the ground: the playing clips have the board in hand (`ANIMSKATEBOARD`: SK8's
    /// physical state 602) and jump off the planted foot (`FOOTJUMP`, on its first tick), as the plant the
    /// graph scores (its `PushTrick` states' `ScoringTrick`).
    pub anim_skateboard: bool,
    /// The clips move the board (SK8's `GroundAnimation` physical state: a trick's take-off from the
    /// ground, the board in hand): the drawn rider keeps the board's heading against his body as it was,
    /// rather than putting the posed board on the physical one (`Game`'s ground alignment).
    pub board_animated: bool,
    pub foot_jump: bool,
    pub plant: Option<plant::PlantKind>,
    /// In the air: a one-foot grab (the `OneFootAir` attribute: the foot plant manager runs) and the
    /// clips reaching for a foot plant (`FootPlanting`).
    pub one_foot_air: bool,
    pub foot_planting: bool,
    /// Hand plants: the motion graph attaches the world grab (Skate 3's RB: the physics looks for a
    /// coping while it is held), its hand plant is committed (`AnimCommittedHandPlant`), and the plant's
    /// scored name (`fs_handplant_up`, `BS_handplant_nofoot` ...).
    pub grab_world: bool,
    pub handplant_committed: bool,
    /// The motion graph's `UpdateIsWeightOnNose` state is running: the rider's weight is on the nose.
    pub weight_on_nose: bool,
    pub handplant_name: Option<&'static str>,
    /// Dropping in on foot: the motion graph stands at the edge (`IntoDropIn/Cycle`), and plays the
    /// board held on the coping (`Grinding/DroppingIn`).
    pub drop_in_stand: bool,
    pub drop_in_grinding: bool,
}

/// A trick the state graph is animating in the air.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirTrick {
    /// What it looks like (for the drawn board and the score).
    pub flick: Flick,
    pub flips: u8,
    /// How far through its clips (0..1).
    pub progress: f32,
    /// A late flip or finger flip (started in the air).
    pub late: bool,
    /// The graph's name for it (string id).
    pub id: u16,
}

/// What a landing was judged on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandingCheck {
    /// Board off the travel direction (rad), and the most it could have been at that speed.
    pub crooked: f32,
    pub limit: f32,
    /// Speed into the ground (m/s) and along it.
    pub impact: f32,
    pub speed: f32,
}

/// Skate 3's reading of the main stick on the ground: turn, hard turn and powerslide amounts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Steering {
    pub turn: f32,
    pub hard: f32,
    /// In a powerslide's entry window: -1 left, +1 right, 0 none.
    pub slide: f32,
    /// The stick's slide amount for a slide either way while it is fully over in the (wide) arc that holds
    /// one (0: not held there): about 0.56 straight back, less further round.
    pub hold_right: f32,
    pub hold_left: f32,
    /// The entry window's amount (signed like `slide`; 0 outside it).
    pub slide_amount: f32,
}

/// Angle 0 is the stick pulled straight back, +-pi straight forward, positive to the right.
pub fn steering(stick: [f32; 2]) -> Steering {
    let [x, y] = stick;
    let len = sqrtf(x * x + y * y).min(1.0);
    if len == 0.0 {
        return Steering::default();
    }
    let a = atan2f(x, -y);
    let signed = if x >= 0.0 { len } else { -len };
    let turn = if a.abs() > PI / 2.0 {
        // Forward half: a small dead cone straight ahead, eased into.
        if a.abs() > 3.05 {
            0.0
        } else if a.abs() > 2.35 {
            (3.05 - a.abs()) * 1.4286 * signed
        } else {
            signed
        }
    } else {
        x
    };
    let mut s = Steering {
        turn,
        ..Steering::default()
    };
    if len > 0.9 {
        if (0.91..=1.39).contains(&a.abs()) {
            s.hard = ((a.abs() - 1.39) * -2.083).max(0.001) * a.signum();
        } else if a.abs() < 0.91 && a != 0.0 {
            s.slide = a.signum();
        }
        // Skate 3's slide intents: right from 90 degrees left of straight back round to about 115
        // degrees right, and the mirror for left.
        if a > -PI / 2.0 && a < 2.0 {
            s.hold_right = (2.0 - a) * 0.28;
        }
        if a > -2.0 && a < PI / 2.0 {
            s.hold_left = (2.0 + a) * 0.28;
        }
        s.slide_amount = match s.slide {
            x if x > 0.0 => s.hold_right,
            x if x < 0.0 => -s.hold_left,
            _ => 0.0,
        };
    }
    s
}

impl Skater {
    pub fn spawn(park: &dyn Terrain) -> Skater {
        Skater::spawn_with(park, Tuning::BUILT_IN, Stance::Goofy)
    }

    pub fn spawn_with(park: &dyn Terrain, tuning: Tuning, stance: Stance) -> Skater {
        let (pos, heading) = park.spawn();
        Skater {
            tuning,
            clips: ClipTiming::BUILT_IN,
            stance,
            pos,
            heading,
            speed: 0.0,
            slip: 0.0,
            vel: Vec3::ZERO,
            normal: Vec3::UP,
            contact_n: Vec3::UP,
            states: states::States::default(),
            air_time: 0.0,
            spin: 0.0,
            spin_rate: 0.0,
            body: BodySpin::default(),
            selector: air::Selector::default(),
            plan: None,
            trick: None,
            switch: false,
            frame_turned: false,
            chord: None,
            fakie: false,
            dark: false,
            fakie_timer: 0.0,
            since_spawn: 0.0,
            takeoff: Takeoff::default(),
            push_steer: 1.0,
            steer: 0.0,
            turn_input: 0.0,
            kick_input: 0.0,
            accel: Vec3::ZERO,
            prev_vel: Vec3::ZERO,
            deck_tilt: 0.0,
            truck_ramp: 1.0,
            yaw_rate: 0.0,
            truck_yaw: 0.0,
            collide: None,
            wall_clock: f32::INFINITY,
            vn: 0.0,
            gap_time: 0.0,
            lead_rise: 0.0,
            pump: Pump::default(),
            idle: 0.0,
            target_speed: 0.0,
            model_speed: 0.0,
            crouch: 0.0,
            stick_crouch: None,
            crouch_com: None,
            crouch_pump: false,
            crouch_nose: false,
            landing: None,
            manual: None,
            manual_engage: 0.0,
            manual_lock: false,
            manual_exit: false,
            wobble: wobble::SpeedWobble::default(),
            dynamic_normal: reckoning::DynamicNormal::default(),
            reckoning: reckoning::Reckoning::default(),
            rig: None,
            grab: None,
            grabbed: None,
            grind: None,
            regrind_block: None,
            jumper: Jumper::default(),
            approach: Approach::default(),
            air_assist: None,
            grind_exit: None,
            brake: None,
            slide: None,
            last_slide: (0.0, 0.0),
            revert: None,
            kickturn: None,
            kick_rate: 0.0,
            hippy: None,
            since_a: f32::INFINITY,
            since_b: f32::INFINITY,
            bail_reason: None,
            last_landing: None,
            graph: GraphControl::default(),
            body_flip: None,
            flipped: None,
            graph_push: None,
            body_com: None,
            clock: Clock(None),
            board_us: 0,
            rider_body: None,
            rider_pose: None,
            rider_axes: crate::math::Mat3::IDENTITY,
            anim_board: None,
            body_us: 0,
            board_stepped: false,
            jump: None,
            air_com: None,
            air_lift: 0.0,
            air_lift_rate: 0.0,
            wallride: None,
            checkpoints: Checkpoints::default(),
            riding_ticks: 0,
            recovery: Recovery::default(),
            grind_pitch_input: 0.0,
            grind_sticks_x: 0.0,
            fwd: yaw_dir(heading),
            biped: Biped::default(),
            climb: climb::Climber::default(),
            cam_yaw: heading,
            board_toggle: None,
            mount_traj: false,
            deck_adjust: adjust::DeckAdjust::NONE,
            possession: possession::Possession::new(),
            deck_assist: false,
            hurdle: hurdle::Hurdle::default(),
            ground_plant: None,
            plant_trick: None,
            footplant: plant::Manager::default(),
            air_plant: None,
            footplanted: None,
            plant_clock: 0.0,
            edge: None,
            drop_in: None,
            handplant: handplant::Manager::default(),
            hand_plant: None,
            hand_plant_name: None,
            stalled: None,
            wipeout: wipeout::Wipeout::default(),
            pose_seed: None,
            wipe_full: [false; 2],
            wipe_request: false,
            wipe_push_off: false,
        }
    }

    /// Puts the rider back on the board after a bail (Skate 3's automatic checkpoint): where the body
    /// came to rest if that is a safe spot, else the best recent safe spot, else the park's spawn;
    /// keeping the tuning, clip timing, stance and the checkpoints.
    pub fn respawn(&mut self, park: &dyn Terrain) {
        let clips = self.clips;
        let mut checkpoints = core::mem::take(&mut self.checkpoints);
        let pick = checkpoints.pick(&self.tuning, park, self.pos, self.switch);
        let turned = self.frame_turned;
        *self = Skater::spawn_with(park, self.tuning, self.stance);
        self.clips = clips;
        // Only a drawing parity (animation trees still playing remember it), kept across a teleport.
        self.frame_turned = turned;
        self.checkpoints = checkpoints;
        if let Some(c) = pick {
            self.pos = c.pos;
            self.switch = c.switch;
            self.set_heading(c.heading);
        }
    }

    /// Back to the park's spawn point (a fresh start), keeping the tuning, clip timing and stance.
    pub fn reset_to_spawn(&mut self, park: &dyn Terrain) {
        let clips = self.clips;
        let turned = self.frame_turned;
        *self = Skater::spawn_with(park, self.tuning, self.stance);
        self.clips = clips;
        // Only a drawing parity (animation trees still playing remember it), kept across a teleport.
        self.frame_turned = turned;
    }

    /// Puts the rider back on the board at `pos` on the ground there, heading `yaw`, rolling at `speed`
    /// (scripted runs), keeping the tuning, clip timing, stance and checkpoints.
    pub fn put_at(&mut self, park: &dyn Terrain, pos: Vec3, yaw: f32, speed: f32) {
        let clips = self.clips;
        let checkpoints = core::mem::take(&mut self.checkpoints);
        let turned = self.frame_turned;
        *self = Skater::spawn_with(park, self.tuning, self.stance);
        self.clips = clips;
        // Only a drawing parity (animation trees still playing remember it), kept across a teleport.
        self.frame_turned = turned;
        self.checkpoints = checkpoints;
        let g = park.ground(pos.x, pos.z, pos.y + 0.1);
        let n = if g.height > f32::MIN / 2.0 {
            g.normal
        } else {
            Vec3::UP
        };
        self.pos = v3(pos.x, g.height.max(pos.y - 0.1), pos.z);
        self.normal = n;
        self.contact_n = n;
        self.set_heading(yaw);
        self.speed = speed;
        self.vel = self.ground_velocity();
        self.since_spawn = FAKIE_GRACE;
    }

    /// Back at a session marker (`crate::marker`), as a respawn puts him back: at `pos` (on the ground
    /// there), facing `heading`, in the stance he had (`switch`), at rest, on the board or on foot with the
    /// board in hand, keeping the tuning, clip timing, stance and checkpoints.
    pub fn return_to(&mut self, park: &dyn Terrain, m: &crate::marker::Marker) {
        self.put_at(park, m.pos, m.heading, 0.0);
        self.switch = m.switch;
        if !m.on_board {
            self.biped = Biped::new(m.heading, Vec3::ZERO, self.normal);
            self.force_state(State::BipedGround);
            self.possession.hold();
            self.sync_walk();
        }
    }

    /// Seconds until the board lands: the known landing's, else straight down onto the ground below.
    pub fn time_to_land(&self, park: &dyn Terrain) -> f32 {
        if let (true, Some(p)) = (self.is_airborne(), &self.plan) {
            return p.remaining(self.air_time).max(0.0);
        }
        let h = park
            .ground(self.pos.x, self.pos.z, self.pos.y + STEP)
            .height;
        let g = self.tuning.gravity.max(0.1);
        let vy = self.vel.y;
        (vy + sqrtf((vy * vy + 2.0 * g * (self.pos.y - h).max(0.0)).max(0.0))) / g
    }

    /// The deck's centre: over the board's contact point, lifted in the air as far as the legs have
    /// tucked the board up.
    pub fn deck(&self) -> Vec3 {
        self.pos + self.normal * DECK_HEIGHT + Vec3::UP * self.air_lift
    }

    /// The board's frame in the world: right, up (the surface normal or the board's own up in the air),
    /// nose.
    pub fn board_frame(&self) -> crate::math::Mat3 {
        // The nose along `fwd` (rolling on both trucks, along the chord between their contacts, so both
        // wheels sit on the surface over a kink), the up from the smoothed normal square to it.
        let f = match (self.rolling(), self.chord) {
            (true, Some(ch)) if ch.dot(self.fwd) > 0.9 => ch,
            _ => self.fwd,
        };
        let f = f.normalize_or(yaw_dir(self.heading));
        let n = self.normal.normalize_or(Vec3::UP);
        let up = (n - f * n.dot(f)).normalize_or(n);
        let f = tangent(f, up);
        crate::math::Mat3::from_cols(up.cross(f), up, f)
    }

    /// Points the nose at `yaw` (seen from above), along the surface under the board.
    pub fn set_heading(&mut self, yaw: f32) {
        self.fwd = tangent(yaw_dir(yaw), self.normal);
        self.heading = yaw;
    }

    /// Without the imported rider's pose, Skate 3's Crouching (the graph's `disttocog`, SK8
    /// crouching.rs) on the stick's crouch and the pumping's feedback gives the centre of mass over
    /// the board, which the pump reads (riding only).
    fn update_stick_crouch(&mut self, dt: f32) {
        use crate::graph::riding::{Crouch, CrouchIntents, CrouchPhysical};
        if self.graph.pose_com.is_some() || !self.rolling() {
            self.stick_crouch = None;
            self.crouch_com = None;
            self.crouch_pump = false;
            return;
        }
        let a = &self.tuning.anim;
        let speed = self.ground_velocity().length();
        let p = CrouchPhysical {
            vel_y: self.vel.y,
            ground_speed: speed,
            pump_potential: self.pump.pumping,
            force: self.pump.absorption,
            ground_force: self.pump.ground_absorption,
            min_crouch: self.pump.min_crouch,
            deck_angle: self.pump.deck_absorption,
            height: self
                .crouch_com
                .unwrap_or_else(|| a.crouch_max_height.eval(speed)),
        };
        let c = self.stick_crouch.get_or_insert_with(|| Crouch::begin(p, a));
        let i = CrouchIntents {
            crouch: Some(self.crouch),
            ..CrouchIntents::default()
        };
        let out = c.update(p, i, dt, a);
        self.crouch_com = Some(out.height);
        self.crouch_pump = out.player_pump;
    }

    /// Sets the air spin to `spin` (rad), turning the board and rider about the board's up to match.
    pub fn set_spin(&mut self, spin: f32) {
        let d = spin - self.spin;
        self.fwd = rotate(self.fwd, self.normal, d);
        self.spin = spin;
        self.sync_heading();
    }

    /// The stick's spin input: the state graph's `PhysBodySpin` when it runs, else the main stick's x.
    fn spin_input(&self) -> f32 {
        self.graph.body_spin
    }

    /// Keeps `heading` in step with `fwd` (left alone while the nose points straight up or down).
    fn sync_heading(&mut self) {
        let h = self.fwd.horizontal();
        if h.length() > 0.05 {
            self.heading = atan2f(h.x, h.z);
        }
    }

    /// The crouch is the nollie one.
    pub fn crouch_nollie(&self) -> bool {
        self.crouch_nose
    }

    /// In the air from a hippie jump.
    pub fn hippie_air(&self) -> bool {
        self.is_airborne() && self.takeoff.hippy
    }

    /// In the air off the board above it, coming down onto it (Skate 3's LandingOnDeck): a hippie jump,
    /// or getting on the board in the air. The board rolls along the ground underneath.
    pub fn landing_on_deck(&self) -> bool {
        self.is_airborne() && (self.takeoff.hippy || self.takeoff.mount)
    }

    /// Coming down onto the free board (the caveman: Skate 3's landing on deck from a jump off it).
    pub fn landing_on_free_board(&self) -> bool {
        self.is_airborne() && self.takeoff.onto_free
    }

    /// The rider as the possession reads him: on foot the walker's facing and velocity.
    pub fn rider_frame(&self) -> possession::RiderFrame {
        let forward = if self.walking() {
            self.biped.forward()
        } else {
            self.fwd
        };
        possession::RiderFrame {
            pos: self.pos,
            forward: forward.horizontal().normalize_or(yaw_dir(self.heading)),
            vel: self.vel,
        }
    }

    /// Coming down onto the free board: whether the feet will land on its deck (Skate 3's
    /// `CanLandOnBoard`: the landing within reach of where the board will be), and whether it is the
    /// wrong way round for the rider (its nose against his travel).
    pub fn free_board_landing(&self, time_to_land: f32) -> (bool, bool) {
        let b = self.possession.board;
        let t = time_to_land.max(0.0);
        let feet = self.pos + self.vel.horizontal() * t;
        let deck = b.pos + b.vel.horizontal() * t;
        let reach = (feet - deck).horizontal().length() < possession::LAND_REACH
            && self.possession.hold == possession::Hold::Free;
        let travel = self.vel.horizontal();
        let facing = if travel.length() > 0.3 {
            travel
        } else {
            self.fwd
        };
        (reach, b.nose.dot(facing) < 0.0)
    }

    /// In a bail with the ragdoll (`skater::wipeout`).
    pub fn ragdoll_active(&self) -> bool {
        self.wipeout.active && self.bailed()
    }

    /// The drawn rider is the mirror image of the clips (they are authored goofy).
    pub fn mirrored(&self) -> bool {
        (self.stance == Stance::Regular) != self.switch
    }

    /// Direction of travel (yaw): the board's heading, turned round when rolling tail first (a
    /// standing rigid board's micrometre-a-second jitter is not rolling).
    pub fn travel_yaw(&self) -> f32 {
        match self.state() {
            _ if (self.is_airborne() || self.bailed()) && self.vel.horizontal().length() > 0.5 => {
                let h = self.vel.horizontal();
                atan2f(h.x, h.z)
            }
            _ => {
                let f = if self.speed < -1e-3 {
                    -self.fwd
                } else {
                    self.fwd
                };
                let h = f.horizontal();
                if h.length() > 0.05 {
                    atan2f(h.x, h.z)
                } else if self.speed < 0.0 {
                    wrap_angle(self.heading + PI)
                } else {
                    self.heading
                }
            }
        }
    }

    /// Extra yaw of the drawn board relative to `heading` (powerslides, kick turns and reverts turn it).
    pub fn board_yaw_offset(&self) -> f32 {
        let mut y = 0.0;
        if let Some(s) = &self.slide {
            y += s.angle;
        }
        y
    }

    /// The gesture group for the board's state (manuals use only flicks off their end).
    pub fn flick_group(&self) -> Group {
        match &self.manual {
            Some(m) if m.nose => Group::Nose,
            Some(_) => Group::Tail,
            None => Group::Square,
        }
    }

    /// +1 when a positive (left) body spin is frontside for this rider: the chest leads.
    fn frontside_sign(&self, fakie: bool) -> f32 {
        let mut s = 1.0;
        if self.mirrored() {
            s = -s;
        }
        if fakie {
            s = -s;
        }
        // Goofy (the clips' own stance) faces +X when the nose points +Z: spinning right turns the
        // chest towards travel.
        -s
    }

    /// Ground direction of the board's nose, following the slope under it.
    fn surface_forward(&self) -> Vec3 {
        tangent(self.fwd, self.contact_n)
    }

    /// Turns the board end for end without changing how it moves (the rider turns round with it).
    fn flip_board(&mut self) {
        self.fwd = -self.fwd;
        self.speed = -self.speed;
        self.slip = -self.slip;
        self.chord = self.chord.map(|c| -c);
        self.frame_turned = !self.frame_turned;
    }

    /// Advances one fixed step (1/60 s: Skate 3's tick).
    pub fn step(
        &mut self,
        park: &dyn Terrain,
        pad: &Pad,
        flick: &FlickOut,
        dt: f32,
        events: &mut Vec<Event>,
    ) {
        self.body_us = 0;
        self.board_stepped = false;
        // SK8 state_timer_2664: the time in the state, this tick included.
        self.states.time += dt;
        if let Some(b) = &mut self.rider_body {
            b.tick();
        }
        let touched = pad.held != 0
            || pad.stick != [0.0, 0.0]
            || flick.stick != [0.0, 0.0]
            || pad.trig_l > 0.0
            || pad.trig_r > 0.0;
        self.idle = if touched { 0.0 } else { self.idle + dt };
        self.since_spawn += dt;
        self.dark = self.graph.dark;
        let rolling = self.rolling();
        let crouch = if rolling && self.manual.is_none() {
            flick
                .crouch
                .max(if self.hippy.is_some() { 1.0 } else { 0.0 })
        } else {
            0.0
        };
        self.crouch = crouch;
        self.update_stick_crouch(dt);
        self.grind_pitch_input = pad.flick[1];
        // The deck is posed apart from the body only in the air (touching down bakes it in).
        if !self.is_airborne() {
            self.deck_adjust = adjust::DeckAdjust::NONE;
        }
        if dt > 0.0 {
            self.accel = (self.vel - self.prev_vel) * (1.0 / dt);
        }
        self.prev_vel = self.vel;
        self.turn_input = steering(pad.stick).turn;
        self.kick_input = if self.kickturn.is_some() {
            self.turn_input
        } else {
            0.0
        };
        self.grind_sticks_x = clamp(pad.stick[0] + pad.flick[0], -1.0, 1.0);
        self.jumper.tick();
        if !self.grinding() {
            self.approach.away = self.approach.away.saturating_add(1);
        }
        if flick.crouch > 0.0 {
            self.crouch_nose = flick.stick[1] < 0.0;
        }
        self.since_a = if pad.was_pressed(btn::A) {
            0.0
        } else {
            self.since_a + dt
        };
        self.since_b = if pad.was_pressed(btn::B) {
            0.0
        } else {
            self.since_b + dt
        };
        if !self.is_airborne() {
            self.body.ground(self.spin_input());
            self.air_com = None;
            self.air_lift = 0.0;
        }
        // The rider's body over the board: the animation's while riding (a crouch over a lip comes
        // down with the clips, as SK8's does before it leaves the ground); following it only so fast
        // through the pop (SK8's GroundAnimation) until the jump's top (measured: SK8's COM over the
        // deck lags its animation there, and follows it again coming down).
        self.body_com = match (self.graph.pose_com, self.body_com) {
            (Some(c), Some(d))
                if (self.is_airborne() && self.vel.y > 0.0)
                    || self.state() == State::GroundAnimation =>
            {
                Some(d + (c - d) * (AIR_LEGS_FOLLOW * dt * 60.0).min(1.0))
            }
            (c, _) => c,
        };
        if self.graph.wipeout && (self.rolling() || self.is_airborne()) {
            if self.is_airborne() {
                self.vel = self.vel * 0.5;
            }
            self.bail("wiped out with the grab tweaked into the landing", events);
        }
        if let Some(l) = &mut self.landing {
            l.time += dt;
            if l.time > LANDING_SHOW {
                self.landing = None;
            }
        }
        if let Some((_, t)) = &mut self.regrind_block {
            *t -= dt;
            if *t <= 0.0 {
                self.regrind_block = None;
            }
        }
        self.read_wipe_buttons(pad, events);
        self.toggle_board(park, pad, dt, events);
        // A wipeout or a dismount asked for by the input (the bail buttons, the graphs' wipeout, Y's
        // clip) is selected before this tick's movement, as Skate 3's input phase precedes its
        // selection; the movement's facts then wait for the next tick's selection.
        let early = self.states.facts.wipeout || self.states.facts.dismount;
        if early {
            self.select_state();
        }
        let flying = self.is_airborne();
        self.step_mode(park, pad, flick, dt, events);
        // A jump off the ground launches the landing prediction on its tick (SK8's ground jump launches
        // the trajectory selector and its batch completes the same tick: an ollie goes from
        // GroundAnimation straight into KnownAir).
        if !flying && self.states.facts.jumped && self.plan.is_none() {
            self.update_prediction(park, self.air_time);
        }
        self.update_possession(park, dt, events);
        // Skate 3's state selector picks the state from what the tick's movement published.
        if !early {
            self.select_state();
        }
        if self.rolling() {
            // The board's up is the ground's under it (what is drawn is the physical deck; Skate 3
            // smooths the ground normal only in its reckoning); the velocity is the ground's.
            self.normal = self.contact_n;
            self.vel = self.ground_velocity() + self.contact_n * self.vn;
            self.update_reckoning(dt);
            // A safe spot to be put back on after a bail.
            self.riding_ticks += 1;
            let at = respawn::Checkpoint {
                pos: self.pos,
                heading: respawn::travel_heading(self.vel, self.heading),
                switch: self.switch,
            };
            let since = (self.since_spawn * 60.0) as u32;
            self.checkpoints
                .observe(&self.tuning, park, at, self.riding_ticks, since);
            let pose = self.approach_pose();
            self.approach.ride(pose);
        } else {
            self.riding_ticks = 0;
            // Off the ground: Skate 3 resets the wobble on entering it again. Its air, grind and
            // off-board reckonings clear the lean.
            self.wobble.reset();
            self.reckoning.lean = 0.0;
            if self.hippie_air() {
                self.roll_rig_alone(park);
            } else if self.is_airborne() {
                // The take-off's tick (a jump leaves the ground before the board's ground step) and air
                // ticks the flight didn't move (plants): the board goes where the flight is.
                if !self.board_stepped {
                    self.fly_board(park, AirMotion::Follow);
                }
            } else {
                self.rig = None;
            }
        }
        self.sync_heading();
    }

    /// One tick of Skate 3's ground reckoning (SK8 board_dynamic_normal.rs, ground_orientation.rs,
    /// reckoning_frames.rs): the dynamic normal from the board's acceleration this tick, then the
    /// rider's up and lean. The lean is published the way SK8's animation feedback does: negated
    /// unless the board's frame is turned (Processed2468 bit 20, the effective frame flipped).
    fn update_reckoning(&mut self, dt: f32) {
        let t = self.tuning.reckoning;
        let wheels = if self.vn != 0.0 {
            0
        } else if self.manual.is_some() {
            2
        } else {
            4
        };
        let accel = if dt > 0.0 {
            (self.vel - self.prev_vel) * (1.0 / dt)
        } else {
            Vec3::ZERO
        };
        self.dynamic_normal.update(
            &t,
            wheels,
            accel,
            self.tuning.gravity,
            self.prev_vel.length(),
        );
        let frame = self.board_frame();
        // The physical board's nose (its frame turns end for end only in effect).
        let nose = if self.frame_turned {
            -frame.cols[2]
        } else {
            frame.cols[2]
        };
        let speed = self.vel.length();
        self.reckoning.update(
            &t,
            reckoning::ReckoningInput {
                com_to_deck: -frame.cols[1],
                ground_normal: self.contact_n,
                dynamic_up: self.dynamic_normal.normal,
                speed,
                wheels,
                balance: if self.manual.is_some() { 1.0 } else { 0.0 },
                board_up: frame.cols[1],
                board_forward: nose,
                heading: nose,
            },
        );
    }

    /// The lean the turn conditioner reads (SK8 physical_feedback.rs `publish`).
    pub fn published_lean(&self) -> f32 {
        if self.frame_turned {
            self.reckoning.lean
        } else {
            -self.reckoning.lean
        }
    }

    fn step_mode(
        &mut self,
        park: &dyn Terrain,
        pad: &Pad,
        flick: &FlickOut,
        dt: f32,
        events: &mut Vec<Event>,
    ) {
        if self.drop_in.is_some() {
            self.step_drop_in(park, pad, dt, events);
            return;
        }
        // Each state's movement (Skate 3's physical states; `skater::states` moves between them).
        match self.state() {
            State::WipeoutGround if self.wipeout.active => {
                let time = self.state_time();
                self.step_wipeout(park, pad, dt);
                // Skate 3's push buttons (A, B: `WipeOutRecover`) put the rider back early once settled.
                let press = pad.was_pressed(btn::A) || pad.was_pressed(btn::B);
                let t = self.tuning;
                if self
                    .recovery
                    .update_settled(&t, time, self.wipeout.over, press, dt)
                {
                    self.respawn(park);
                    events.push(Event::Respawned);
                }
            }
            State::WipeoutGround => {
                let time = self.state_time();
                let h = self.vel.horizontal() * (1.0 - smooth(3.0, dt));
                self.vel = Vec3 { y: 0.0, ..h };
                self.try_move(park, self.vel * dt);
                self.pos.y = park
                    .ground(self.pos.x, self.pos.z, self.pos.y + STEP)
                    .height;
                let reset = self.bailed() && {
                    // Skate 3's push buttons (A, B) put the rider back early once settled.
                    let press = pad.was_pressed(btn::A) || pad.was_pressed(btn::B);
                    let speed = self.vel.length();
                    let t = self.tuning;
                    self.recovery.update(&t, time, speed, press, dt)
                };
                if reset {
                    self.respawn(park);
                    events.push(Event::Respawned);
                } else if self.running_out() && time >= RUNOUT_TIME {
                    // Back on the board where it stopped (the runout's end puts the state there).
                    let n = park
                        .ground(self.pos.x, self.pos.z, self.pos.y + STEP)
                        .normal;
                    self.contact_n = n;
                    self.normal = n;
                    self.fwd = tangent(self.fwd, n);
                    self.speed = 0.0;
                    self.slip = 0.0;
                    self.vn = 0.0;
                    self.vel = Vec3::ZERO;
                    self.fakie = false;
                    self.fakie_timer = 0.0;
                    self.force_state(State::PhysicsGround);
                }
            }
            s if s.category() == 100 => {
                if !self.update_handplant(park, events) {
                    self.roll(park, pad, flick, dt, events)
                }
            }
            s if s.is_grind() => self.slide_rail(park, pad, dt, events),
            s if s.category() == 500 => self.walk(park, pad, dt),
            State::Sleeping | State::Teleporting => {}
            _ => self.fly(park, pad, self.air_time + dt, dt, events),
        }
    }

    /// Y gets off the board on the ground and back on it on foot (Skate 3's `ToggleOffBoardState`). With
    /// the state graphs the clips time it: off once the dismount clip starts walking the rider
    /// (`OB_TRAJ`), on once the mount clip's `OB_TRAJ` is over; without them at once.
    fn toggle_board(&mut self, park: &dyn Terrain, pad: &Pad, dt: f32, events: &mut Vec<Event>) {
        if self.drop_in.is_some() || self.climbing() {
            return;
        }
        if self.toggle_starts(pad)
            && let Some(edge) = self.drop_in_edge()
        {
            // Standing at an edge with the board in hand, Y drops in (Skate 3's `IntoDropIn`, the first
            // of its ground mounts).
            self.drop_in = Some(dropin::DropIn::start(self.pos, edge));
            self.board_toggle = None;
            self.mount_traj = false;
            return;
        }
        if self.toggle_starts(pad) {
            self.board_toggle = Some(0.0);
            self.mount_traj = false;
        }
        let Some(t) = self.board_toggle else {
            return;
        };
        // In the air off the board (a jump, or stepping off a ledge mid-mount), Y gets on it there: the
        // rider comes down onto the board under him (Skate 3's air mount, its `Mount/Air` clips). A board
        // out of the hands is picked up first (Skate 3's `OB_Mounting` retrieves it, quickly).
        if self.walking() && self.biped.air.is_some() {
            match self.possession.hold {
                possession::Hold::Held | possession::Hold::Underfoot => self.mount_in_air(events),
                possession::Hold::Returning => {}
                _ => {
                    let rider = self.rider_frame();
                    self.possession.retrieve(&self.tuning, rider, true);
                }
            }
            return;
        }
        let t = t + dt;
        self.board_toggle = Some(t);
        let on_board = self.rolling();
        let g = self.graph;
        let go = match (on_board, self.walking()) {
            (true, _) => g.ob_traj && g.dismounting,
            (_, true) => {
                if g.mounting && g.ob_traj {
                    self.mount_traj = true;
                }
                self.mount_traj && !g.ob_traj
            }
            _ => {
                self.board_toggle = None;
                return;
            }
        };
        if go {
            if self.walking() {
                self.step_on(park, events);
            } else if self.selects(|f| f.dismount = true).category() == 500 {
                // Off the board once the selector takes the rider off it (Skate 3's BipedGround).
                self.step_off(events);
            }
        } else if t > TOGGLE_WAIT {
            self.board_toggle = None;
        }
    }

    /// Whether Y starts getting off or on the board this tick: pressed now (or in the air just before
    /// landing, off the board), and allowed where the rider is. The graphs hear Y only then
    /// ([`Skater::board_toggle_intent`]), so they and the physics always agree on it.
    fn toggle_starts(&self, pad: &Pad) -> bool {
        if self.board_toggle.is_some() || self.drop_in.is_some() || !pad.was_pressed(btn::Y) {
            return false;
        }
        if self.rolling() {
            // SK8 DisableDismount: not on ground steeper than this.
            self.contact_n.y >= DISMOUNT_MIN_UP
        } else {
            // On the ground or in the air: in the air the rider gets on the board as he comes down.
            self.walking()
        }
    }

    /// Skate 3's `ToggleOffBoardState` and `NewToggleOffBoardState` for the graphs this tick (held, new):
    /// only while the physics takes Y as a toggle (it starts this tick or is under way), so a Y it turns
    /// down (in the air, on steep ground) doesn't start the clips without it.
    /// Holding a ledge (climbing on foot).
    pub fn climbing(&self) -> bool {
        self.walking() && self.climb.holding()
    }

    pub fn board_toggle_intent(&self, pad: &Pad) -> (bool, bool) {
        let new = self.toggle_starts(pad);
        let toggling = self.board_toggle.is_some()
            || self.drop_in.is_some_and(|d| d.phase != dropin::Phase::Out);
        let held = new || (toggling && pad.held & btn::Y != 0);
        (held, new)
    }

    /// The board off the feet each tick: off the board, L fully in drops it and R throws it ahead
    /// (Skate 3's LT / RT); with it out of the hands either picks it back up. With the state graphs their
    /// ToggleBoard times it (the throw clips' release, the retrieve cycle), and getting on (`Mount`)
    /// retrieves a free board; then the board's own body moves on. In the air off the board over the
    /// free board, Skate 3's landing assist may bring the rider down onto it (the caveman).
    fn update_possession(&mut self, park: &dyn Terrain, dt: f32, events: &mut Vec<Event>) {
        // Under the feet riding or grinding, as the selector keeps it (not getting off, not wiping out).
        if (self.rolling() || self.grinding())
            && matches!(self.selects(|_| {}).category(), 100 | 400)
        {
            self.possession.under_feet();
        }
        let rider = self.rider_frame();
        let t = self.tuning;
        if self.walking() && !self.climb.holding() {
            let p = &mut self.possession;
            let g = self.graph;
            if let Some(throw) = g.release_board {
                p.let_go(&t, rider, throw);
            }
            if g.retrieve_board {
                p.retrieve(&t, rider, false);
            }
            if g.ob_mounting {
                p.retrieve(&t, rider, true);
            }
        }
        self.possession.step(&t, park, rider, dt);
        // Off the board in the air over the free board: Skate 3's landing assist (SK8 biped_air
        // update.rs, landing_deck `assist`) takes a flight down onto its deck when it is near enough to
        // the jump's own (by `landing_assist_limit` of the time in the air) and long enough (0.4 s); the
        // motion graph then starts the landing on it (`DoLandOnBoard`).
        if !self.walking() || self.biped.air.is_none() {
            self.deck_assist = false;
            return;
        }
        if self.possession.hold != possession::Hold::Free {
            self.deck_assist = false;
        }
        if !self.deck_assist
            && let Some(air) = self.biped.air
            && self.board_toggle.is_none()
            && let Some((flight, time)) =
                self.possession.landing_flight(&t, self.pos, self.biped.vel)
            && time > 0.4
            && (flight - self.biped.vel).length() < possession::landing_assist_limit(air)
        {
            self.biped.set_flight(flight, t.gravity, park, self.pos);
            self.sync_walk();
            self.deck_assist = true;
        }
        if self.deck_assist && self.graph.land_on_board {
            self.deck_assist = false;
            let flight = self.biped.vel;
            self.land_on_free_board_from(flight, events);
        }
    }

    /// Into the flight down onto the free board (Skate 3's LandingOnDeck): the rider off the board in
    /// the air, its nose the way he faces.
    fn land_on_free_board_from(&mut self, flight: Vec3, events: &mut Vec<Event>) {
        let _ = events;
        self.contact_n = Vec3::UP;
        self.normal = Vec3::UP;
        self.fwd = tangent(self.biped.forward(), Vec3::UP);
        self.fakie = false;
        self.fakie_timer = 0.0;
        self.take_off(None);
        self.takeoff.mount = true;
        self.takeoff.onto_free = true;
        self.vel = flight;
        self.speed = flight.dot(self.fwd);
        self.slip = 0.0;
        self.sync_heading();
    }

    /// On the board in the air off it: its nose the way the rider faces, flying as he was, to come down
    /// onto it (no points, as no trick).
    fn mount_in_air(&mut self, events: &mut Vec<Event>) {
        let vel = self.biped.vel;
        self.contact_n = Vec3::UP;
        self.normal = Vec3::UP;
        self.fwd = tangent(self.biped.forward(), Vec3::UP);
        self.fakie = false;
        self.fakie_timer = 0.0;
        self.take_off(None);
        self.takeoff.mount = true;
        self.vel = vel;
        self.speed = vel.dot(self.fwd).max(0.0);
        self.slip = 0.0;
        self.board_toggle = None;
        self.mount_traj = false;
        self.sync_heading();
        events.push(Event::SteppedOn);
    }

    /// Off the board, facing the way it was going (or its nose, standing).
    fn step_off(&mut self, events: &mut Vec<Event>) {
        let travel = self.vel.horizontal();
        let heading = if travel.length() > 0.3 {
            atan2f(travel.x, travel.z)
        } else {
            self.travel_yaw()
        };
        self.biped = Biped::new(heading, self.vel, self.contact_n);
        self.states.facts.dismount = true;
        self.board_toggle = None;
        // The board into the hands.
        self.possession.hold();
        self.manual = None;
        self.slide = None;
        self.kickturn = None;
        self.revert = None;
        self.hippy = None;
        self.sync_walk();
        events.push(Event::SteppedOff);
    }

    /// Back on the board, its nose the way the rider faces, rolling at his speed that way, in the stance
    /// he got off in.
    fn step_on(&mut self, park: &dyn Terrain, events: &mut Vec<Event>) {
        let n = park
            .ground(self.pos.x, self.pos.z, self.pos.y + STEP)
            .normal;
        self.contact_n = n;
        self.normal = n;
        self.fwd = tangent(self.biped.forward(), n);
        let along = self.biped.vel.dot(self.fwd).max(0.0);
        self.speed = along;
        self.slip = 0.0;
        self.vn = 0.0;
        self.vel = self.fwd * along;
        self.fakie = false;
        self.fakie_timer = 0.0;
        // The stance stays: off the board after riding switch, the graphs get him on switch (the mount
        // clips' switch versions), so he rides on switch. Setting it back to natural here turned the rider
        // round a moment after getting on.
        self.spin = 0.0;
        self.spin_rate = 0.0;
        self.states.facts.mount = Some(true);
        self.board_toggle = None;
        // From the hands (or wherever the mount's retrieval had it) under the feet.
        self.possession.under_feet();
        self.sync_heading();
        events.push(Event::SteppedOn);
    }

    /// On foot: Skate 3's biped controller (`skater::biped`). Sprint is A (Skate 3's A, without its
    /// B: here X), jump is B (Skate 3's X, by position).
    fn walk(&mut self, park: &dyn Terrain, pad: &Pad, dt: f32) {
        // Holding a ledge, the climb moves the rider; B pulls him up (SK8 climbing: its jump button).
        if let Some(h) = self.climb.hold(park, pad.was_pressed(btn::B), dt) {
            self.pos = h.pos;
            self.biped.heading = h.heading;
            self.biped.turn = 0.0;
            self.biped.vel = Vec3::ZERO;
            self.biped.speed = 0.0;
            self.biped.air = None;
            self.biped.loco = biped::Loco::Stand;
            self.sync_walk();
            self.edge = None;
            return;
        }
        let input = BipedInput {
            stick: pad.stick,
            cam_yaw: self.cam_yaw,
            sprint: pad.held & btn::A != 0 && pad.held & btn::X == 0,
            jump: pad.was_pressed(btn::B) && self.board_toggle.is_none(),
        };
        let mut pos = self.pos;
        let tuning = self.tuning;
        self.biped
            .step(&tuning.biped, tuning.gravity, park, &mut pos, input, dt);
        self.pos = pos;
        // Reaching for a ledge ahead, and in the air catching it.
        let b = &self.biped;
        self.climb.approach(
            park,
            self.pos,
            b.heading,
            b.vel,
            b.air.is_some(),
            b.world_stick,
            dt,
        );
        if self.climb.holding() {
            self.biped.air = None;
            self.biped.vel = Vec3::ZERO;
        }
        self.sync_walk();
        // The edge ahead (Skate 3's off-board `DistToEdge`).
        self.edge = if self.biped.air.is_none() {
            dropin::find_edge(park, self.pos, self.biped.forward())
        } else {
            None
        };
        if self.pos.y < park.kill_height() {
            self.respawn(park);
        }
    }

    /// The board-riding view of the walker (what the camera and the rest read).
    fn sync_walk(&mut self) {
        let b = &self.biped;
        self.heading = b.heading;
        self.fwd = b.forward();
        self.vel = b.vel;
        self.speed = b.vel.dot(self.fwd);
        self.slip = 0.0;
        self.normal = Vec3::UP;
        self.contact_n = b.normal;
    }

    /// Skate 3's fakie flag: a moment of rolling tail first; never just after a spawn, and off again
    /// once rolling backwards slowly.
    fn update_fakie(&mut self, dt: f32) {
        if self.revert.is_some() {
            return;
        }
        let back = -self.speed;
        if self.since_spawn < FAKIE_GRACE || back <= FAKIE_SLOW {
            self.fakie = false;
            self.fakie_timer = 0.0;
        } else if back > FAKIE_FAST {
            self.fakie = true;
        } else {
            self.fakie_timer += dt;
            if self.fakie_timer > FAKIE_TIME {
                self.fakie = true;
            }
        }
    }

    /// The rider's state on the ground (fakie, revert, powerslide, kick turn, steering, pushing, braking,
    /// hippie jump, flicks), then the board's physics with what the rider does to it.
    fn roll(
        &mut self,
        park: &dyn Terrain,
        pad: &Pad,
        flick: &FlickOut,
        dt: f32,
        events: &mut Vec<Event>,
    ) {
        let t = self.tuning;
        let ticks = dt * 60.0;
        let tail_brake = self.update_manual(dt, events);
        let st = steering(pad.stick);
        self.update_fakie(dt);
        if self.graph.turn_round && (self.fakie || self.speed < 0.0) {
            // Skate 3 turns the rider round: same travel, the other foot forward.
            self.flip_board();
            self.switch = !self.switch;
            self.fakie = false;
            self.fakie_timer = 0.0;
            events.push(Event::Switched);
        }

        // Revert: the board turned half round on its wheels, the travel kept (Skate 3's yaw servo).
        if self.revert.is_some() {
            self.update_revert(dt, events);
        } else if let Some(fs) = self.graph.revert {
            self.revert = Some(Revert {
                side: if fs { 1.0 } else { -1.0 },
                time: 0.0,
                turned: 0.0,
                rate: 0.0,
            });
            self.slide = None;
            self.landing = None;
        }

        self.update_slide(&st, dt);

        // Kick turn: the state graph's KickTurn state (speed and stick gates) turns the board.
        let graph_kick = self.graph.kickturn;
        let kick = graph_kick.is_some() && pad.stick[0] != 0.0 && self.manual.is_none();
        // The kick turn's yaw rate builds up towards its speed's rate and, let go, runs down (SK8's rider
        // turning the board through its feet).
        if kick {
            let dir = -pad.stick[0].signum();
            let time = self.kickturn.map_or(0.0, |k| k.1) + dt;
            self.kickturn = Some((dir, time));
            let scale = graph_kick.map_or(1.0, |k| k.clamp(0.25, 2.0) / KICKTURN_SPIN_SCALE);
            let target = dir * kickturn_rate(self.speed.abs()) * scale;
            self.kick_rate +=
                (target - self.kick_rate) * (1.0 - libm::powf(1.0 - KICKTURN_BUILD, ticks));
        } else {
            self.kickturn = None;
            self.kick_rate *= libm::powf(KICKTURN_RELEASE, ticks);
            if self.kick_rate.abs() < 0.05 || self.slide.is_some() || self.revert.is_some() {
                self.kick_rate = 0.0;
            }
        }
        if self.kick_rate != 0.0 {
            self.fwd = rotate(self.fwd, self.contact_n, self.kick_rate * dt);
        }

        // Steering: Skate 3's reading of the stick, eased in. A powerslide's trucks hold a steady turn.
        // Riding fakie the turn is negated (Skate 3's action graph hands the physics `FakieTurn`: `Turn`
        // with `fakieFilter="negate"`, read as the `Turn` packet by SetTurning, SK8 set_turning.rs): with
        // the trucks' yaw already reversing rolling tail first, the stick turns the travel its own way.
        let fakie_turn = if self.fakie { -1.0 } else { 1.0 };
        let follow = 1.0 - libm::powf(1.0 - t.steer_follow.clamp(0.0, 1.0), ticks);
        let turn_in = match &self.slide {
            Some(s) if !s.releasing => s.side * t.slide_turn_value,
            Some(_) => 0.0,
            None if kick => 0.0,
            None if st.hard != 0.0 => st.turn.signum() * fakie_turn,
            None => st.turn * fakie_turn,
        };
        self.steer += (turn_in - self.steer) * follow;

        // Pushing (A: the usual push, B: mongo), hippie jump (both), foot brake (X).
        self.update_hippy(pad, dt, events);
        if !self.stays_rolling() {
            return;
        }
        // Foot plants (a boneless, fastplant or no-comply): no push, brake or pop until the jump off the
        // planted foot.
        if self.update_ground_plant(dt, events) {
            return;
        }
        let planting = self.ground_plant.is_some();
        // With the state graphs, a push only drives while the push clip's foot is down, and the foot
        // brake only once the brake clip's foot touches.
        let g = self.graph;
        let push_target = match g {
            _ if planting => None,
            g if g.push_posed => {
                // Skate 3 (SK8 contact_events.rs, push.rs): while the push clip's foot is down the
                // board chases the posed trajectory's speed, within the push's change a tick.
                self.graph_push = None;
                g.push_target
                    .filter(|_| g.push_contact)
                    .map(|v| v.min(self.tuning.push_max_speed))
            }
            g => self.graph_push(g.push_contact, g.push_target, dt),
        };
        self.update_brake(pad, dt);
        let foot_brake = self.brake.is_some() && g.brake_contact;
        let brake = t.braking_scalar
            * if tail_brake {
                t.tail_brake
            } else if foot_brake {
                t.foot_brake
            } else {
                0.0
            };

        // The trick to pop: the state graph's (its takeoff clip reached the jump).
        let pop = if planting { None } else { g.pop };
        if let Some((f, strength)) = pop {
            if let Some(w) = self.wallride.take() {
                // Popping off a wall: away from it as well as up.
                if w.time > 0.0 {
                    events.push(Event::Landed {
                        name: "WALLRIDE".into(),
                        points: 0,
                        score: crate::scoring::Scored::Flat(250 + (w.time * 200.0) as u32),
                    });
                }
                let n = self.contact_n;
                self.vel = self.ground_velocity() + n * 2.5;
                self.pos += n * 0.05;
                self.take_off(Some(f));
                self.vel.y += self.pop_speed(self.tuning.jump_height(strength, 0.0)) * 0.6;
                self.jump = Some((self.vel, 0));
                events.push(Event::Popped(f));
                return;
            }
            // The tick's roll still happens (the board doesn't stand still for the pop's tick, which
            // the camera's anchor would read as a stop).
            self.pos += self.ground_velocity() * dt;
            self.pop(f, strength, pad.stick, events);
            return;
        }

        let drive = Drive {
            turn: self.steer,
            hard: if self.slide.is_some() { 0.0 } else { st.hard },
            raw_turn: if self.slide.is_some() || kick {
                0.0
            } else {
                st.turn
            },
            push_target,
            pushing: g.pushing,
            brake,
            manual: self.manual.is_some(),
            manual_correction: self.manual.is_some_and(|m| m.correcting),
            slide: self
                .slide
                .as_ref()
                .filter(|s| !s.releasing)
                .map(|s| s.angle),
            slide_time: self.slide.as_ref().map_or(0.0, |s| s.time),
            pivot: kick || self.revert.is_some(),
            keep_velocity: self.revert.is_some(),
            intentional_pump: self.graph.player_pump || self.crouch_pump,
        };
        self.ride(park, &drive, dt, events);
        // Riding onto a rail: into coping, off a ledge's top, up a bank into a ledge; not while the
        // board tips over an edge on its trailing wheels (a slow roll off a deck drops in).
        if self.stays_rolling() && !self.tipping_over_edge() {
            self.try_grind(park, Category::Ground, events);
        }
    }

    /// One tick of a revert (SK8 revert_state.rs): the yaw rate eases (by `revert_gain` a tick) to the
    /// target rate for the angle left (never braking a faster turn the same way); done within the
    /// tolerance for the slope, or after `revert_duration`. The velocity stays where it is in the world.
    fn update_revert(&mut self, dt: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        let n = self.contact_n;
        let Some(r) = &mut self.revert else { return };
        r.time += dt;
        let left = PI - r.turned;
        let tolerance = t.revert_tolerance.eval(n.y) * PI / 180.0;
        if left < tolerance || r.time > t.revert_duration {
            // Skate 3's fsrevert / bsrevert: the graphs say which (+1 frontside).
            let fs = r.side > 0.0;
            let id = crate::scoring::catalog::id(if fs { "fsrevert" } else { "bsrevert" });
            let held = r.time;
            self.revert = None;
            self.fakie = self.speed < 0.0;
            self.fakie_timer = 0.0;
            events.push(Event::Landed {
                name: "REVERT".into(),
                points: 0,
                score: crate::scoring::Scored::Ground { id, held },
            });
            return;
        }
        let target = t.revert_speed.eval(left);
        if r.rate <= target {
            r.rate += (target - r.rate) * t.revert_gain * dt * 60.0;
        }
        let turn = (r.rate * dt).min(left);
        r.turned += turn;
        let side = r.side;
        let vel = self.ground_velocity();
        self.fwd = rotate(self.fwd, n, side * turn);
        self.set_ground_velocity(vel);
    }

    /// Powerslides (Skate 3's rules): the main stick swung fully back to one side, at speed, starts one;
    /// it holds while the stick stays fully over anywhere in a wide arc and lasts at least
    /// `well_into_slide_time`; it ends when let go, too slow, or the board has come back in line. The
    /// board's angle from its travel follows how far back the stick is, and relaxes after a while.
    fn update_slide(&mut self, st: &Steering, dt: f32) {
        let t = self.tuning;
        let (last_side, last_amount) = self.last_slide;
        self.last_slide = (st.slide, st.slide_amount);
        let speed = self.ground_velocity().length();
        let blocked = self.manual.is_some() || self.revert.is_some();
        let Some(s) = &mut self.slide else {
            // Newly in the entry window, moving towards straight back.
            let entered = st.slide != 0.0 && last_side == 0.0;
            let inward = last_amount.abs() <= st.slide_amount.abs();
            if entered && inward && !blocked && speed > t.slide_speed_threshold {
                self.slide = Some(Powerslide {
                    side: st.slide,
                    ..Powerslide::default()
                });
            }
            return;
        };
        s.time += dt;
        if s.releasing {
            s.angle = approach(s.angle, 0.0, SLIDE_TURN_RATE * dt);
            s.amount = (s.angle.abs() / (PI / 2.0)).min(1.0);
            if s.angle == 0.0 {
                self.slide = None;
            }
            return;
        }
        // The stick's slide amount for this side (0: no longer held over).
        let held = if s.side > 0.0 {
            st.hold_right
        } else {
            st.hold_left
        };
        let mut amount = held;
        if s.time < t.well_into_slide_time {
            amount = amount.max(t.slide_min_entry);
        }
        let mut target = amount * PI;
        if s.time > SLIDE_RELAX {
            let relaxed = t.slide_input_remap.eval(amount) * PI;
            let w = t.slide_remap_vs_speed.eval(speed) * (s.time - SLIDE_RELAX).clamp(0.0, 1.0);
            target += (relaxed - target) * w;
        }
        s.angle = approach(s.angle, s.side * target, SLIDE_TURN_RATE * dt);
        s.amount = (s.angle.abs() / (PI / 2.0)).min(1.0);
        let c = cosf(s.angle);
        let sliding = t.slide_speed_factor.eval(speed) * (1.0 - c.abs());
        s.not_sliding = if sliding < t.not_sliding_threshold.eval(c) {
            s.not_sliding + dt
        } else {
            0.0
        };
        let well_into = s.time >= t.well_into_slide_time;
        let done = held == 0.0
            || speed <= t.slide_speed_threshold
            || s.not_sliding > t.not_sliding_time.eval(c)
            || blocked;
        if well_into && done {
            s.releasing = true;
        }
    }

    /// Pushing under the state graph: its push states decide when the foot is down (`contact`, then the
    /// push sets' contact time) and the speed the board chases (`target`: the speed the push trees'
    /// coefficients end at, as Skate 3 takes the push from the blended clips).
    fn graph_push(&mut self, contact: bool, target: Option<f32>, dt: f32) -> Option<f32> {
        let (base, end, left) = match self.graph_push {
            Some((b, e, l)) if l > 0.0 => (b, e, l - dt),
            _ if contact => {
                let (_, contact_time, ..) = self.clips.blend(self.speed.max(0.0));
                (self.speed, 0.0, contact_time)
            }
            _ => {
                self.graph_push = None;
                return None;
            }
        };
        let end = target
            .unwrap_or(end)
            .min(self.tuning.push_max_speed)
            .max(end);
        self.graph_push = Some((base, end, left));
        Some(end)
    }

    /// The foot brake (X): returns whether the braking foot is on the ground.
    fn update_brake(&mut self, pad: &Pad, dt: f32) -> bool {
        let t = self.tuning;
        let steep =
            libm::acosf(self.contact_n.y.clamp(-1.0, 1.0)) > t.push_slope_limit * PI / 180.0;
        let can = self.manual.is_none()
            && self.slide.is_none()
            && self.revert.is_none()
            && self.wallride.is_none()
            && !steep;
        let held = pad.is_held(btn::X) && can;
        match &mut self.brake {
            Some(b) if held && b.released.is_none() => {
                b.time += dt;
                // The foot has to reach the ground first.
                return b.time >= self.clips.brake_down;
            }
            Some(b) => {
                let r = b.released.unwrap_or(0.0) + dt;
                b.released = Some(r);
                if r > 0.35 || held {
                    self.brake = None;
                }
            }
            None if held && self.speed.abs() > 0.05 => {
                self.brake = Some(FootBrake {
                    time: 0.0,
                    released: None,
                });
            }
            None => {}
        }
        false
    }

    /// In a hippie jump, the landing on deck's query for something in the way of the landing on the
    /// board (SK8 landing_deck `update`, `prepare_query`, `sync`; `skater::hurdle`).
    fn step_hurdle(&mut self, park: &dyn Terrain) {
        let land_in = self.time_to_land(park);
        let g = self.tuning.gravity.max(0.1);
        // Seconds past the apex: the vertical speed against gravity.
        self.hurdle.update(-self.vel.y / g, land_in);
        if !self.hurdle.wants_query() {
            return;
        }
        let mut landing = self.pos + self.vel * land_in;
        landing.y -= 0.5 * g * land_in * land_in;
        // The board rolls on along the ground under him: where it is when he lands.
        let floor = park.ground(landing.x, landing.z, self.pos.y + STEP).height;
        self.hurdle
            .prepare(crate::math::v3(landing.x, floor.min(landing.y), landing.z));
        let flight = hurdle::Flight {
            time_to_land: land_in,
            landing,
            descending: self.vel.y < 0.0,
            height: self.pos.y,
        };
        // Flatground's parks and districts are static: nothing a sweep meets moves, so the query meets
        // no moving body and nothing is hurdled (a still one would only block, which the landing here
        // doesn't read).
        self.hurdle.sync(None, &flight);
    }

    /// Both push buttons together wind up a hippie jump; letting go jumps off the board and back on,
    /// along the surface's normal (weaker on steep faces, as a pop is).
    fn update_hippy(&mut self, pad: &Pad, dt: f32, events: &mut Vec<Event>) {
        let both = pad.is_held(btn::A) && pad.is_held(btn::B);
        match self.hippy {
            Some(t) if both => self.hippy = Some(t + dt),
            Some(t) => {
                self.hippy = None;
                let tu = self.tuning;
                let k = tu.hippy_windup.eval(t).clamp(0.0, 1.0);
                let k_speed = (self.speed.abs() / tu.jump_speed_range.max(0.01)).clamp(0.0, 1.0);
                let lo = tu.jump_floor
                    + tu.jump_low_vs_speed.eval(k_speed) * (tu.hippy_low - tu.jump_floor);
                let hi = tu.jump_floor
                    + tu.jump_high_vs_speed.eval(k_speed) * (tu.hippy_high - tu.jump_floor);
                let up = self.pop_speed(lo + (hi - lo) * k);
                self.vel = self.pop_velocity(up, [0.0, 0.0]);
                self.pos += self.contact_n * 0.02;
                self.take_off(None);
                self.takeoff.hippy = true;
                self.hurdle.take_off();
                events.push(Event::Launched);
            }
            None if both
                && self.since_a.max(self.since_b) <= HIPPY_CHORD
                && self.manual.is_none()
                && self.slide.is_none() =>
            {
                self.hippy = Some(0.0);
            }
            None => {}
        }
    }

    /// Moves horizontally by `delta`, refusing to go into anything more than a step up.
    /// Slides along a wall when only one axis is blocked. Returns false when fully blocked.
    fn try_move(&mut self, park: &dyn Terrain, delta: Vec3) -> bool {
        let here = self.pos;
        let ok = |x: f32, z: f32| {
            park.ground(x, z, here.y + STEP).height <= here.y + STEP
                && !park.blocked(here, Vec3 { x, y: here.y, z })
        };
        let (nx, nz) = (self.pos.x + delta.x, self.pos.z + delta.z);
        if ok(nx, nz) {
            self.pos.x = nx;
            self.pos.z = nz;
            true
        } else if ok(nx, self.pos.z) {
            self.pos.x = nx;
            false
        } else if ok(self.pos.x, nz) {
            self.pos.z = nz;
            false
        } else {
            false
        }
    }

    /// Manuals: the state graph engages, holds and leaves them. The stick past the manual zone drags the
    /// tail (or nose) to brake: returns whether it is.
    fn update_manual(&mut self, dt: f32, events: &mut Vec<Event>) -> bool {
        self.manual_exit = false;
        self.graph_manual(self.graph.manual, dt, events)
    }

    /// A manual the state graph holds (engaged and left as the graph says): the deck pitch servoes to a
    /// target that grows with the stick's lean (SK8's manual controller aims at (0.25 + 0.75 lean) of
    /// the most tilt), or to the brake angle while dragging (SK8 manual/controller.rs `calculate`).
    ///
    /// Skate 3's balance is positive on the tail (the stick pulled back) and negative on the nose. While
    /// dragging the tail or nose with the board rolling against that direction (a nose drag rolling
    /// forwards, a tail drag rolling backwards; `apply_correction`), either the physics mode's corrective
    /// force acts (Normal, Hardcore) or, with it off (Easy), the physics asks the graph to end the manual
    /// (`PhysicsWantsManualExit`). The corrective force (`correcting`) needs the speed along the board
    /// over `manual_correction_speed` and the deck within a degree of its target; it then replaces the
    /// tick's brake and the deck's turn (SK8 grounded/state/ordinary.rs). With the manual's wheels off
    /// the ground (lifted off a crest while still riding) the correction bleeds off
    /// (`TorqueBleedOffWithNoContact`) and turns the deck by `TorqueScalarWithNoContact` of it.
    fn graph_manual(
        &mut self,
        want: Option<(bool, f32, bool)>,
        dt: f32,
        events: &mut Vec<Event>,
    ) -> bool {
        let Some((nose, lean, brake)) = want else {
            if self.manual.is_some() {
                self.end_manual(events);
            }
            return false;
        };
        if self.manual.is_none_or(|m| m.nose != nose) {
            self.end_manual(events);
            self.manual = Some(Manual {
                nose,
                ..Manual::default()
            });
            self.brake = None;
        }
        // Skate 3's balance sign: the tail's positive, the nose's negative.
        let balance = if nose { -1.0 } else { 1.0 };
        let opposing = brake && self.speed * balance < 0.0;
        self.manual_exit = opposing && !self.tuning.manual_corrective;
        // The manual's wheels on the ground: the board is not lifted off it (`vn`).
        let contact = self.vn == 0.0;
        let t = &self.tuning;
        let speed = libm::fabsf(self.speed);
        let m = self.manual.as_mut().unwrap();
        let started = m.time > 0.0;
        m.time += dt;
        m.brake = if brake { 1.0 } else { 0.0 };
        // SK8's manual controller: the target grows with the lean; while dragging it is the brake angle
        // with no wobble.
        let reach = 0.25 + 0.75 * lean.clamp(0.0, 1.0);
        let target = if brake {
            t.manual_brake_tilt * reach * PI / 180.0
        } else {
            let wobble = libm::sinf(2.0 * PI * t.manual_noise_freq * m.time) * t.manual_noise_scale;
            t.manual_max_tilt * reach * PI / 180.0 + t.manual_noise_vs_speed.eval(speed) * wobble
        };
        if !started {
            m.correction = t.manual_start_torque * target;
        }
        // PID on the deck pitch, per 1/60 s tick (the deck turns by the accumulated correction).
        let ticks = (dt * 60.0).max(0.0);
        let error = (target - m.pitch).clamp(-t.manual_max_error, t.manual_max_error);
        m.filtered_error = 0.95 * m.filtered_error + 0.05 * error;
        // The derivative reads the deck's measured turn since last tick (the rigid deck's, which
        // lags what the controller asks).
        let last_turn = if started { m.pitch - m.measured } else { 0.0 };
        m.measured = m.pitch;
        let derivative = (last_turn * t.manual_d).clamp(-t.manual_d_limit, t.manual_d_limit);
        let output = if contact {
            m.correction += derivative + m.filtered_error * t.manual_i + error * t.manual_p;
            1.0
        } else {
            m.correction *= t.manual_torque_bleed;
            t.manual_torque_no_contact
        };
        m.correcting = opposing
            && t.manual_corrective
            && speed > t.manual_correction_speed
            && libm::fabsf(target - m.pitch) * (180.0 / PI) < 1.0;
        m.last_step = if m.correcting {
            0.0
        } else {
            m.correction * output
        };
        m.pitch += m.last_step * ticks;
        m.balance = 0.0;
        brake && !m.correcting
    }

    /// Ends a manual on purpose (stick released, or popped out of): scores it if it was long enough.
    fn end_manual(&mut self, events: &mut Vec<Event>) {
        self.manual_engage = 0.0;
        if let Some(m) = self.manual.take()
            && m.time >= MANUAL_MIN_TIME
        {
            let id = crate::scoring::catalog::id(if m.nose { "nosemanual" } else { "tailmanual" });
            events.push(Event::Landed {
                name: (if m.nose { "NOSE MANUAL" } else { "MANUAL" }).into(),
                points: 0,
                score: crate::scoring::Scored::Ground { id, held: m.time },
            });
        }
    }

    /// Vertical speed for a jump to a centre-of-mass height, from riding height.
    fn pop_speed(&self, height: f32) -> f32 {
        let t = &self.tuning;
        sqrtf((2.0 * t.gravity * (height - t.jump_floor)).max(0.0))
    }

    fn pop(&mut self, f: Flick, strength: f32, stick: [f32; 2], events: &mut Vec<Event>) {
        self.end_manual(events);
        // A trick the clip marks as out of a manual jumps from the mode's manual low end.
        let low = if self.graph.pop_from_manual {
            self.tuning.jump_low_manual
        } else {
            self.tuning.jump_low
        };
        let h = self.tuning.jump_height_from(low, strength, self.speed);
        // Skate 3 pops the centre of mass from where the animation has it at the jump (SK8
        // ground_jump `animation_com_position`): over the deck, which the tail strike has already
        // pitched up off its riding height. Without the imported rider, from `AbsoluteMinHeight`.
        let g = self.graph;
        let posed = self.body_com.or(g.pose_com).map(|c| (c, g.pose_lift));
        let speed = match posed {
            Some((com, lift)) => {
                let now = DECK_HEIGHT - WHEEL_RADIUS + lift + com;
                sqrtf((2.0 * self.tuning.gravity * (h - now)).max(0.0))
            }
            None => self.pop_speed(h),
        };
        self.vel = self.pop_velocity(speed, stick);
        let on_vert = self.contact_n.y < self.tuning.vert_align_max_ny;
        // The flight starts from the lifted deck.
        let lift = posed.map_or(0.0, |p| p.1);
        self.pos += self.contact_n * (0.02 + lift);
        self.take_off(Some(f));
        self.jump = Some((self.vel, 0));
        self.takeoff.on_vert = on_vert;
        self.takeoff.lean = stick[1];
        events.push(Event::Popped(f));
    }

    fn take_off(&mut self, f: Option<Flick>) {
        let pose = self.approach_pose();
        self.approach.take_off(pose);
        self.air_assist = None;
        // Off the ground: the rigid board flies on under the air states (`Skater::fly_board`).
        if let Some(r) = &mut self.rig {
            r.airborne = true;
        }
        self.publish_jump();
        if self.walking() {
            self.states.facts.mount = Some(false);
        }
        self.air_time = 0.0;
        // Rolling off a ledge mid-manual just ends it (no points: it wasn't finished).
        self.manual = None;
        self.manual_engage = 0.0;
        self.landing = None;
        self.grab = None;
        self.grabbed = None;
        self.brake = None;
        self.slide = None;
        self.revert = None;
        self.kickturn = None;
        self.kick_rate = 0.0;
        self.hippy = None;
        self.wallride = None;
        self.collide = None;
        self.vn = 0.0;
        self.gap_time = 0.0;
        self.lead_rise = 0.0;
        self.pump = Pump::default();
        self.spin = 0.0;
        self.spin_rate = 0.0;
        self.plan = None;
        self.body_flip = None;
        self.selector = air::Selector::default();
        self.takeoff = Takeoff {
            fakie: self.fakie,
            switch: self.switch,
            hippy: false,
            mount: false,
            onto_free: false,
            ground: Some(self.contact_n),
            on_vert: false,
            lean: 0.0,
        };
        self.trick = f.map(ActiveTrick::new);
        self.ground_plant = None;
        self.plant_trick = None;
        self.footplant = plant::Manager::default();
        self.air_plant = None;
        self.footplanted = None;
        self.plant_clock = 0.0;
        self.air_com = self.body_com;
        self.air_lift = 0.0;
        self.air_lift_rate = 0.0;
        self.jump = None;
        // The air's up starts from the reckoning's (SK8's air reckoning goes on from the ground's up,
        // smoothed, not the deck's: a board tipped over a lip leaves with the rider upright).
        self.normal = self.reckoning.up.normalize_or(self.normal);
        self.fwd = tangent(self.fwd, self.normal);
        // Rolled off: the flight starts where the physical deck is, under that up (a board tipped
        // over a lip would otherwise be put back over its wheels and jump up and back).
        if f.is_none()
            && let Some(r) = &self.rig
        {
            self.pos = r.board.deck().position - self.normal * DECK_HEIGHT;
        }
    }

    /// The rider's centre of mass over the board's point (m, along the board's up).
    fn com_height(&self) -> f32 {
        DECK_HEIGHT + self.body_com.unwrap_or(air::COM_TO_DECK)
    }

    /// Skate 3's foot plants on the ground (`skater::plant`), timed by the graphs' clips: the board in
    /// hand while they carry `ANIMSKATEBOARD` (SK8's physical state 602), the jump off the planted foot
    /// on their `FOOTJUMP`. True: jumped off.
    fn update_ground_plant(&mut self, dt: f32, events: &mut Vec<Event>) -> bool {
        let g = self.graph;
        let kind = g.plant.or(self.ground_plant.map(|p| p.kind));
        if g.foot_jump
            && let Some(kind) = kind
        {
            self.foot_jump_off(kind, events);
            return true;
        }
        if g.anim_skateboard {
            let p = self.ground_plant.get_or_insert(GroundPlant {
                kind: kind.unwrap_or(PlantKind::FsBoneless),
                time: 0.0,
                lifted: true,
            });
            p.time += dt;
        } else {
            self.ground_plant = None;
        }
        false
    }

    /// The jump off the planted foot (SK8 boneless.rs, the `FOOTJUMP` launch).
    fn foot_jump_off(&mut self, kind: PlantKind, events: &mut Vec<Event>) {
        self.end_manual(events);
        let up = self.contact_n;
        let travel = if self.speed < 0.0 {
            self.fwd * -1.0
        } else {
            self.fwd
        };
        let forward = tangent(travel, up);
        let current = self.vel;
        let v = plant::foot_jump(&self.tuning.plant, up, current, current, forward);
        self.pos += up * 0.02;
        self.take_off(None);
        self.vel = v;
        self.plant_trick = Some(kind);
        events.push(Event::FootJump(kind));
    }

    /// In the air: the one-foot grab (Skate 3's `OneFootAir`: a grab and a push button; with the graphs
    /// their one-foot states) runs the foot plant manager; when the free foot meets something it plants
    /// and the swing over it starts (SK8 known/footplant.rs, footplant/update.rs, pose.rs).
    fn update_footplant(&mut self, park: &dyn Terrain, now: f32, events: &mut Vec<Event>) {
        let runs = self.graph.one_foot_air
            && self.plant_clock > plant::MIN_AIR_TIME
            && !(self.takeoff.hippy || self.takeoff.mount || self.takeoff.onto_free);
        if !runs {
            self.footplant = plant::Manager::default();
            return;
        }
        let t = self.tuning;
        let com = self.pos + self.normal * self.com_height();
        let (landing_normal, landing_time) = match &self.plan {
            Some(p) => (p.normal, p.remaining(now)),
            None => (Vec3::UP, 1.0),
        };
        let frame = plant::Frame {
            com,
            vel: self.vel,
            gravity: t.gravity,
            up: self.normal,
            landing_normal,
            landing_time,
            candidate: true,
            planting: self.graph.foot_planting,
        };
        self.footplant.update(&t.plant, park, &frame);
        if !self.footplant.perform {
            return;
        }
        let m = self.footplant;
        self.air_plant = Some(AirPlant::start(
            &t.plant, com, self.vel, m.contact, m.normal,
        ));
        self.footplanted = Some(match self.graph.grab_name {
            Some(n) if n.ends_with("footplant") => n,
            _ => plant::footplant_name(true, true),
        });
        self.plan = None;
        self.selector = air::Selector::default();
        self.air_assist = None;
        events.push(Event::FootPlanted);
    }

    /// The swing over the planted foot (SK8 physical state 601): the centre of mass on its curve, the
    /// board under it; too fast down or across wipes out; at its end the flight off the foot.
    fn swing_over_plant(&mut self, time: f32, dt: f32, events: &mut Vec<Event>) {
        let Some(mut p) = self.air_plant else {
            return;
        };
        let (com, released) = p.step(dt);
        let to = com - self.normal * self.com_height();
        self.vel = (to - self.pos) * (1.0 / dt.max(1e-4));
        self.pos = to;
        if let Some(reason) = plant::plant_wipeout(&self.tuning.plant, self.vel) {
            self.air_plant = None;
            self.bail(reason, events);
            return;
        }
        if released {
            self.air_plant = None;
            self.vel = p.release;
            self.pos = p.anchor
                + p.normal * 0.02
                + Vec3::UP * (plant::RELEASE_LIFT - plant::RELEASE_BOARD_DROP);
            self.footplant = plant::Manager::default();
            self.plant_clock = 0.0;
            self.plan = None;
            self.selector = air::Selector::default();
            events.push(Event::FootReleased);
        } else {
            self.air_plant = Some(p);
        }
        self.air_time = time;
    }

    /// The landing prediction (SK8 PhysicsAir / trajectory selector): while no landing is known, a
    /// query every few ticks; each tick the query in progress follows its flights a little further. A
    /// known landing becomes the plan the board flies, its start error blended away.
    fn update_prediction(&mut self, park: &dyn Terrain, now: f32) {
        let t = self.tuning;
        if self.plan.is_none() && !self.selector.pending() {
            if self.selector.countdown == 0 {
                self.selector.launch(
                    &t,
                    air::Launch {
                        time: now,
                        pos: self.pos,
                        vel: self.vel,
                        ground: self.takeoff.ground.unwrap_or(Vec3::UP),
                        on_vert: self.takeoff.on_vert,
                        lean: self.takeoff.lean,
                        grind: self.jumper.cooldown == 0,
                        lift: self.air_com.map_or(0.0, |c0| (c0 - t.air_com_min).max(0.0)),
                        sphere: self.reckoning.up.normalize_or(Vec3::UP)
                            * (self.com_height() - t.traj_displacement),
                    },
                );
                self.selector.countdown = air::QUERY_EVERY;
            } else {
                self.selector.countdown -= 1;
            }
        }
        if let Some(mut p) = self.selector.update(&t, park) {
            p.adopted = now;
            p.offset = self.pos - p.flight.at(now - p.start);
            self.plan = Some(p);
        }
    }

    fn fly(&mut self, park: &dyn Terrain, pad: &Pad, time: f32, dt: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        let now = time - dt;
        self.plant_clock += dt;
        if self.air_plant.is_some() {
            self.swing_over_plant(time, dt, events);
            return;
        }
        if self.hand_plant.is_some() {
            self.in_hand_plant(time, dt, events);
            return;
        }
        self.update_prediction(park, now);
        if self.takeoff.hippy {
            self.step_hurdle(park);
        }

        // Body spin (Skate 3's controller): the stick, its wind-up, a let-go spin carrying on; with a
        // known landing and the stick let go, the assist turns the body towards the landing direction
        // (or the other way round when that is nearer); without one the stick's spin goes through the
        // assist's path.
        let stick = self.spin_input();
        let auto = match &self.plan {
            Some(p) => match p.heading {
                Some(h) if stick == 0.0 && p.grind.is_none() => {
                    let lead = if self.takeoff.fakie {
                        -self.fwd
                    } else {
                        self.fwd
                    };
                    let left = p.remaining(now);
                    air::auto_spin(
                        &t,
                        lead,
                        h,
                        p.normal,
                        self.normal.y,
                        self.body.speed,
                        left,
                        dt,
                    )
                }
                _ => 0.0,
            },
            None => air::physics_air_spin(&t, stick, now),
        };
        // Let go before the take-off's landing is known (it resolves over a few ticks here; Skate 3's
        // known air has it from the first) and not spinning yet: no let-go spin starts (SK8: a turn let
        // go just before an ollie doesn't spin it). A spin already going carries on, as in Skate 3,
        // whose assist takes over only from no spin or the same way.
        let waiting = stick == 0.0
            && self.plan.is_none()
            && self.selector.pending()
            && self.body.speed.abs() < 0.02;
        let spin = self.body.air(&t, stick, auto);
        let spin = if waiting {
            self.body.hold_still()
        } else {
            spin
        };
        // A body flip: started, turned, and off when falling upside down without the grab.
        if self.update_body_flip(now, dt) {
            self.bail("fell upside down", events);
            return;
        }
        let spin = if self.body_flip.is_some() {
            spin * t.moves.flip_spin_scale
        } else {
            spin
        };

        // The board's up towards the landing surface: arriving a few ticks before touchdown on a known
        // landing, else a share of the angle a tick (none past a limit).
        let (to, share) = match &self.plan {
            Some(p) => (p.normal, air::align_share(p.remaining(now), dt)),
            None => {
                let n = self.selector.suggested.unwrap_or(self.normal);
                let angle = libm::acosf(self.normal.dot(n).clamp(-1.0, 1.0));
                let share = if angle <= t.phys_air_dont_align {
                    t.phys_air_align
                } else {
                    0.0
                };
                (n, share)
            }
        };
        let max = t.air_up_max_delta.eval(air::COM_TO_DECK);
        let up = air::tilt_towards(self.normal, to, share, max);
        self.fwd = tangent(self.fwd, up);
        self.normal = up;
        // Skate 3's air reckoning turns the one reckoning's up (the rider's axes, which place the
        // animated board the hook turns the deck to).
        self.reckoning.up = up;
        // The spin turns the body about its own up.
        let turn = spin * dt;
        self.spin_rate = spin;
        self.spin += turn;
        self.fwd = rotate(self.fwd, self.normal, turn);
        // Board adjust: the C-stick lines the deck up against the body.
        self.update_deck_adjust(pad);
        // The legs: the board follows the animated centre of mass's height over it, as fast as the
        // rider's body follows its animation (SK8's physical rider; measured, 0.15 of the gap a tick).
        if self.takeoff.hippy || self.takeoff.mount {
            // A hippie jump or getting on in the air: the rider is off the board above it while it rolls
            // on along the ground (Skate 3's LandingOnDeck). He lands when he comes back down onto it,
            // not when the board his clips' legs leave behind touches the ground.
            self.air_lift = 0.0;
            self.air_lift_rate = 0.0;
        } else if let (Some(c0), Some(d)) = (self.air_com, self.body_com) {
            self.air_lift_rate = (c0 - d - self.air_lift) / dt.max(1e-4);
            self.air_lift = c0 - d;
        }
        let g = self.graph;
        self.follow_graph_trick(&g);
        self.update_grab(pad, dt);
        self.update_footplant(park, now, events);
        if self.air_plant.is_some() {
            self.air_time = time;
            return;
        }

        // A flight steered onto a rail: the grind air assist moves and turns the board towards it.
        if let Some(target) = self.plan.as_ref().and_then(|p| p.grind) {
            self.grind_air_assist(park, target, dt);
        }
        let category = Category::Air;
        if self.takeoff.hippy || self.takeoff.mount {
            self.fly_off_board(park, time, dt, category, events);
            return;
        }
        // The flight (SK8 PhysicsAir / KnownAir): the physical board carries it. With a known landing
        // the board follows the selected trajectory (its start error blended away; the legs' tuck and
        // a body flip place the deck off it, `air_board_target`); without one it flies on its own,
        // given the jump's velocity for its first ticks (SK8 `calculate_velocity_from_jump`).
        let from = (self.pos, self.vel);
        let motion = match &self.plan {
            Some(p) => {
                let assist = self.air_assist.map_or(Vec3::ZERO, |a| a.offset);
                self.pos = p.target(time, t.traj_error_blend) + assist;
                self.vel = (self.pos - from.0) * (1.0 / dt.max(1e-4));
                AirMotion::Follow
            }
            None => match self.jump_velocity(dt) {
                Some(v) => {
                    self.vel = v;
                    AirMotion::Velocity(v)
                }
                None => {
                    self.vel.y -= t.gravity * dt;
                    AirMotion::Free
                }
            },
        };
        // A rail is looked for where the flight takes the board this tick, before the board meets the
        // rail's own box (Skate 3's grind contact takes it onto the rail, not its collision).
        if motion != AirMotion::Follow {
            self.pos = from.0 + self.vel * dt;
        }
        if self.try_grind(park, category, events) {
            return;
        }
        if motion != AirMotion::Follow {
            (self.pos, self.vel) = from;
        }
        let incoming = self.fly_board(park, motion);
        self.read_air_board();
        // A wall: before Skate 3's selector would take the board out of the air on it, the board meets
        // it on its own contacts, off the predicted flight (asked again); after, a wallride if there is
        // a floor well below, else off it (or off the board).
        let wall = self.air_wall(park, incoming);
        if wall.is_some() && !self.touchdown_lands() {
            self.plan = None;
            self.selector = air::Selector::default();
        } else if let Some(wn) = wall {
            let hit = crate::park::Hit {
                t: 0.0,
                point: self.deck(),
                normal: wn,
                surface: 0,
            };
            if self.try_air_wallride(park, &hit, events) {
                return;
            }
            // Off the predicted flight: ask again.
            self.plan = None;
            self.selector = air::Selector::default();
            match self.hit_wall(wn, incoming, true, events) {
                Some(v) => {
                    self.vel = v;
                    if let Some(r) = &mut self.rig {
                        for p in &mut r.board.parts {
                            p.velocity = v;
                        }
                    }
                }
                None => return,
            }
        }
        // Touchdown: the board's wheels on a surface it lands on, or its deck or a truck against the
        // world (a tail on the coping). Skate 3's state selector takes the board onto the ground from
        // any of those contacts (its `colliding`) once the air state has settled; until then it stays
        // in the air, the board on its own contacts.
        let (wheels, n, point) = self.air_wheels();
        let (n, point) = match &self.rig {
            Some(r) if wheels == 0 => (r.board.body_normal, r.board.deck().position),
            _ => (n, point),
        };
        let body = self.states.facts.deck || self.states.facts.trucks;
        if wheels > 0 || body {
            if wheels > 0 {
                self.publish_wheels(wheels);
            }
            if self.selects(|_| {}).category() == 100 {
                let surface = park
                    .raycast(point + n * 0.05, -n, 0.1)
                    .map_or(0, |h| h.surface);
                if let Some(r) = &self.rig {
                    let d = r.board.deck();
                    self.pos = d.position - d.axes.cols[1] * DECK_HEIGHT;
                }
                self.vel = incoming;
                self.land(n, surface, time, events);
                return;
            }
        }
        if self.pos.y < park.kill_height() {
            // Fell out of the world: start over.
            self.respawn(park);
            events.push(Event::Respawned);
        } else {
            self.air_time = time;
        }
    }

    /// The skater's flight read off the physical board after its air step: the point the deck is held
    /// over (the target's offset from it taken off the deck), and the deck's velocity.
    fn read_air_board(&mut self) {
        let offset = self.air_board_target().0 - self.pos;
        if let Some(r) = &mut self.rig {
            let d = r.board.deck();
            self.pos = d.position - offset;
            self.vel = d.velocity;
            r.pos = self.pos;
            r.velocity = self.vel;
        }
    }

    /// A wall the board ran into this step (its trucks' or deck's contact across its up, or a wheel on
    /// a face it can't land on), its normal facing the board (the side the wall isn't on), while the
    /// board's incoming velocity `v` moves into it.
    fn air_wall(&self, park: &dyn Terrain, v: Vec3) -> Option<Vec3> {
        let r = self.rig.as_ref()?;
        let up = self.normal;
        let wheel = r
            .board
            .wheels
            .iter()
            .flatten()
            .filter(|w| w.load > 0.0 && !(w.normal.dot(up) > 0.5 || w.normal.y > 0.6))
            .max_by(|a, b| a.load.total_cmp(&b.load))
            .map(|w| w.normal);
        let n = r.board.wall.map(|w| w.0).or(wheel)?;
        let at = r.board.deck().position;
        let behind = |n: Vec3| {
            park.raycast(at, -n, 0.5)
                .is_some_and(|h| h.normal.dot(n) > 0.5)
        };
        let n = if behind(n) {
            n
        } else if behind(-n) {
            -n
        } else {
            return None;
        };
        (n.y.abs() < 0.5 && v.dot(n) < 0.0).then_some(n)
    }

    /// SK8 `calculate_velocity_from_jump` (TU3 0x82D34BC0): for 12 ticks after a jump the free board
    /// is given the jump's velocity under gravity, unless the board's own has fallen 3 m/s below it
    /// (then its own vertical), its horizontal clamped to the jump's speed when 3 m/s or more off it.
    fn jump_velocity(&mut self, dt: f32) -> Option<Vec3> {
        let (jump, frames) = self.jump.as_mut()?;
        if *frames >= JUMP_CORRECTION_TICKS {
            return None;
        }
        let predicted = *jump - Vec3::UP * (self.tuning.gravity * dt * *frames as f32);
        *frames += 1;
        let current = self
            .rig
            .as_ref()
            .map_or(self.vel, |r| r.board.deck().velocity);
        let error = current - predicted;
        let mut out = predicted;
        if error.y <= -3.0 {
            out.y = current.y;
        }
        let eh = error.horizontal();
        if eh.dot(eh) >= 9.0 {
            let max = out.horizontal().length();
            let h = current.horizontal();
            let len = h.length();
            let h = if len > max && len > 1e-5 {
                h * (max / len)
            } else {
                h
            };
            out.x = h.x;
            out.z = h.z;
        }
        Some(out)
    }

    /// The rider's flight off the board (a hippie jump, getting on in the air: Skate 3's
    /// LandingOnDeck, whose skeleton flies its centre of mass's trajectory): with a known landing along
    /// it, else under gravity; he lands when he comes back down onto something.
    fn fly_off_board(
        &mut self,
        park: &dyn Terrain,
        time: f32,
        dt: f32,
        category: Category,
        events: &mut Vec<Event>,
    ) {
        let t = self.tuning;
        match &self.plan {
            Some(p) => {
                let next = p.target(time, t.traj_error_blend);
                self.vel = (next - self.pos) * (1.0 / dt.max(1e-4));
            }
            None => self.vel.y -= t.gravity * dt,
        }
        let from = self.pos;
        let delta = self.vel * dt;
        let len = delta.length();
        if len > 1e-6
            && let Some(h) = park.raycast(from, delta * (1.0 / len), len + 0.01)
            && delta.dot(h.normal) < 0.0
        {
            self.pos = h.point + h.normal * 1e-3;
            if self.normal.dot(h.normal) > 0.5 || h.normal.y > 0.6 {
                if self.try_grind(park, category, events) {
                    return;
                }
                if self.selects(|f| f.wheels = 4).category() == 100 {
                    self.pos = h.point;
                    self.land(h.normal, h.surface, time, events);
                    return;
                }
                self.publish_wheels(4);
            }
            let into = self.vel.dot(h.normal);
            if into < 0.0 {
                self.vel -= h.normal * into;
            }
        } else {
            self.pos = from + delta;
        }
        if self.pos.y < park.kill_height() {
            self.respawn(park);
            events.push(Event::Respawned);
        } else {
            self.air_time = time;
        }
    }

    /// The side the rider faces (the toes): for a goofy rider riding normally the board's left
    /// (up x nose), the other way when mirrored.
    pub fn toe_side(&self) -> Vec3 {
        let side = self.normal.cross(self.fwd).normalize_or(v3(1.0, 0.0, 0.0));
        if self.mirrored() { -side } else { side }
    }

    /// The board's long axis pointed the way it travels, for a body flip's axis (SK8 BeginBodyFlip:
    /// the reckoning frame turned round riding tail first): the nose, or the tail flying backwards.
    fn flip_ahead(&self) -> Vec3 {
        let along = self.vel.dot(self.fwd);
        let backwards = if along.abs() > 0.1 {
            along < 0.0
        } else {
            self.fakie
        };
        if backwards { -self.fwd } else { self.fwd }
    }

    /// The body flip's tick (SK8 KnownAir `CalculateBodyFlipSpeed`, Reckoning `UpdateBodyFlip`,
    /// `CheckUpsideDownFallingWipeout`): starts with a flip asked for and a grab held once the known
    /// flight is long enough, turns while the grab is held (or early on), slows let go. Returns whether
    /// the rider is falling upside down without the grab (a bail).
    fn update_body_flip(&mut self, now: f32, dt: f32) -> bool {
        let m = self.tuning.moves;
        let g = self.graph;
        let grabbing = g.grabbing || self.grab.is_some();
        if self.body_flip.is_none()
            && let (Some(front), true, Some(p)) = (g.body_flip, grabbing, &self.plan)
            && p.contact.time > m.flip_start_time.eval(p.normal.y)
        {
            self.body_flip = Some(flip::BodyFlip::begin(self.normal, self.flip_ahead(), front));
        }
        let Some(mut f) = self.body_flip else {
            return false;
        };
        let target = self.plan.as_ref().map_or(f.speed, |p| {
            flip::BodyFlip::target_speed(&m, p.contact.time)
        });
        // A perfect body flip (the mode's) turns whatever the grab.
        let requested = if m.perfect_body_flips || grabbing || now < m.flip_min_grab_time {
            target
        } else {
            0.0
        };
        f.update(&m, requested, dt);
        self.body_flip = Some(f);
        let landing_up = self.plan.as_ref().map_or(Vec3::UP, |p| p.normal);
        m.upside_down_bail
            && !grabbing
            && flip::falling_upside_down(&m, f.apply(self.normal), landing_up)
    }

    /// The name and points of the jump so far (without spin and grab), for grinds and landings.
    /// The air's trick as shown, and its Skate 3 trick id.
    fn trick_label(&self) -> Option<(String, Option<u16>)> {
        if let Some(tr) = &self.trick {
            return Some((tr.name(), tr.scorable()));
        }
        if let Some(k) = self.plant_trick {
            return Some((
                String::from(k.name()),
                crate::scoring::catalog::id(k.trick()),
            ));
        }
        self.takeoff.hippy.then(|| {
            (
                String::from("HIPPIE JUMP"),
                crate::scoring::catalog::id("hippyjump"),
            )
        })
    }

    /// Stance words for a name: FAKIE or SWITCH (from take-off).
    fn stance_prefix(&self) -> &'static str {
        if self.takeoff.fakie {
            "FAKIE "
        } else if self.takeoff.switch {
            "SWITCH "
        } else {
            ""
        }
    }

    /// In the air under the state graphs, the board does what the graph's clips do: the trick (a late
    /// flip or finger flip starts one), its flips, how far through it is, a dark catch.
    fn follow_graph_trick(&mut self, g: &GraphControl) {
        match g.air_trick {
            Some(a) => {
                let fresh = a.late && self.trick.is_none_or(|t| t.id != a.id);
                if fresh {
                    let mut t = ActiveTrick::new(a.flick);
                    t.late = true;
                    self.trick = Some(t);
                }
                if let Some(t) = &mut self.trick {
                    if t.id == u16::MAX || fresh {
                        t.id = a.id;
                    }
                    t.clock = Some(if g.danger { a.progress.min(0.99) } else { 1.0 });
                    t.flips = t.flips.max(a.flips);
                }
            }
            None => {
                if let Some(t) = &mut self.trick {
                    t.clock = Some(1.0);
                }
            }
        }
        if let Some(t) = &mut self.trick {
            t.dark |= g.dark;
        }
    }

    fn bail(&mut self, reason: &'static str, events: &mut Vec<Event>) {
        self.body_flip = None;
        self.bail_reason = Some(reason);
        if self.grinding() || self.grind.is_some() {
            self.grind_exit = Some("wiped out");
            self.grind = None;
        }
        if !self.is_airborne() && !self.grinding() && self.vel == Vec3::ZERO {
            self.vel = self.surface_forward() * self.speed;
        }
        self.wallride = None;
        // The selector takes the rider into Skate 3's WipeoutGround.
        self.states.facts.wipeout = true;
        self.states.runout = false;
        self.trick = None;
        self.grab = None;
        self.grabbed = None;
        self.manual = None;
        self.landing = None;
        self.brake = None;
        self.slide = None;
        self.revert = None;
        self.kickturn = None;
        self.kick_rate = 0.0;
        self.hippy = None;
        self.collide = None;
        self.plan = None;
        self.ground_plant = None;
        self.air_plant = None;
        self.footplant = plant::Manager::default();
        self.hand_plant = None;
        self.handplant = handplant::Manager::default();
        self.drop_in = None;
        self.recovery = Recovery::default();
        self.start_wipeout();
        events.push(Event::Bailed);
    }

    /// The bail's body: the ragdoll from the drawn rider's pose (the game's seed from the imported
    /// rider, else the placeholder standing on the board), moving as the rider did; the board lets go
    /// and goes its own way (Skate 3's wipeout frees the board's drives).
    fn start_wipeout(&mut self) {
        let points = match self.pose_seed {
            Some(p) => p,
            None => {
                let place = crate::math::Xform::new(self.board_frame(), self.pos);
                wipeout::built_in_pose(&place, self.mirrored())
            }
        };
        self.wipeout.start(&points, self.vel, 1.0 / 60.0);
        let rider = self.rider_frame();
        match self.possession.hold {
            possession::Hold::Underfoot => {
                self.possession
                    .release_with(rider, self.board_frame(), self.vel)
            }
            possession::Hold::Held => self.possession.release_with(
                rider,
                crate::math::Mat3::from_cols(v3(1.0, 0.0, 0.0), Vec3::UP, v3(0.0, 0.0, 1.0)),
                self.vel,
            ),
            _ => {}
        }
    }

    /// Skate 3's wipeout buttons (SK8 `input/wipeout_intentions.rs`, by position on the pad): both
    /// triggers fully in with Z (its LT and RT fully in with both sticks clicked in, which the
    /// GameCube pad hasn't got), new on any of them, bail on purpose (`WipeOutRequest`); in a bail R
    /// going fully in pushes off (its RT, `WipeOutPushOff`).
    fn read_wipe_buttons(&mut self, pad: &Pad, events: &mut Vec<Event>) {
        let full = crate::graph::input::wipe_triggers(pad);
        let new = [full[0] && !self.wipe_full[0], full[1] && !self.wipe_full[1]];
        self.wipe_full = full;
        self.wipe_request = full[0]
            && full[1]
            && pad.is_held(btn::Z)
            && (new[0] || new[1] || pad.was_pressed(btn::Z));
        self.wipe_push_off = new[1];
        let can = (self.rolling() || self.is_airborne() || self.grinding() || self.walking())
            && self.drop_in.is_none();
        if self.wipe_request && can {
            if self.walking() {
                self.vel = self.biped.vel;
            }
            self.bail("bailed on purpose", events);
        }
    }

    /// A tick of the bail's body: the sticks and the push off into it, then the rider's place
    /// (where the camera and the reset look) follows the pelvis.
    fn step_wipeout(&mut self, park: &dyn Terrain, pad: &Pad, dt: f32) {
        let control = biped::world_stick(pad.stick, self.cam_yaw);
        let gesture = self.graph.wipe_gesture;
        let input = wipeout::Input {
            control,
            gesture,
            push_off: self.wipe_push_off,
        };
        let t = self.tuning;
        self.wipeout.step(park, &t, input, dt);
        let pelvis = self.wipeout.body.pos[ragdoll::PELVIS];
        let g = park.ground(pelvis.x, pelvis.z, pelvis.y + 0.1);
        let low = pelvis.y - ragdoll::REST[ragdoll::PELVIS].y;
        self.pos = v3(
            pelvis.x,
            if g.height > f32::MIN / 2.0 {
                g.height.max(low)
            } else {
                low
            },
            pelvis.z,
        );
        self.vel = self.wipeout.body.com_vel(dt);
    }

    /// Stepped off the board after a hit the rider could run out of: no bail, back on the board where it
    /// stops (Skate 3's runout).
    fn run_out(&mut self, reason: &'static str, events: &mut Vec<Event>) {
        self.bail_reason = Some(reason);
        self.wallride = None;
        // Skate 3's runout is a wipeout state too (WipeoutGround), back on the board at its end.
        self.states.facts.wipeout = true;
        self.states.runout = true;
        self.trick = None;
        self.grab = None;
        self.grabbed = None;
        self.manual = None;
        self.landing = None;
        self.brake = None;
        self.slide = None;
        self.revert = None;
        self.kickturn = None;
        self.kick_rate = 0.0;
        self.hippy = None;
        self.collide = None;
        events.push(Event::RanOut);
    }

    /// Coming down onto a steep face with a floor well below it (not a quarter pipe or bowl): the board
    /// turns onto the wall and rides it. Returns whether it did.
    fn try_air_wallride(
        &mut self,
        park: &dyn Terrain,
        h: &crate::park::Hit,
        events: &mut Vec<Event>,
    ) -> bool {
        let t = self.tuning;
        let n = h.normal;
        let along = self.vel - n * self.vel.dot(n);
        let free = self.trick.is_none_or(|t| t.progress() >= 1.0) && self.grab.is_none();
        if along.length() < 1.0 || !free {
            return false;
        }
        let Some((floor, height)) = Self::floor_under(park, h.point, n) else {
            return false;
        };
        if floor.dot(n) >= t.wallride_max_dot_floor || height < WALLRIDE_MIN_HEIGHT {
            return false;
        }
        // Onto the wall's face only as the selector takes the board onto the ground.
        if !self.touchdown_lands() {
            return false;
        }
        if let Some((name, trick)) = self.trick_label() {
            events.push(Event::Landed {
                name: format!("{}{name}", self.stance_prefix()),
                points: 0,
                score: crate::scoring::Scored::Air(crate::scoring::Air {
                    trick,
                    ..Default::default()
                }),
            });
        }
        self.publish_wheels(4);
        self.pos = h.point;
        self.contact_n = n;
        self.normal = n;
        let travel = along.normalize_or(tangent(self.fwd, n));
        let backwards = tangent(self.fwd, n).dot(travel) < 0.0;
        self.fwd = if backwards { -travel } else { travel };
        self.vn = 0.0;
        self.gap_time = 0.0;
        self.truck_ramp = 0.0;
        self.set_ground_velocity(along);
        self.fakie = backwards;
        self.trick = None;
        self.spin = 0.0;
        self.spin_rate = 0.0;
        self.takeoff.hippy = false;
        self.start_wallride(true);
        true
    }

    /// Holds a grab while a trigger is down (once any flip has finished). L is the left hand, R the
    /// right: which grab that is depends on the stance. The C-stick picks the grab's variant and tweaks
    /// it (with the state graphs, their `ScoringGrabs` name and `TweakX`/`TweakY`). Letting go keeps the
    /// longest grab of the jump.
    fn update_grab(&mut self, pad: &Pad, dt: f32) {
        let (l, r) = (pad.trig_l > GRAB_TRIGGER, pad.trig_r > GRAB_TRIGGER);
        // The clips' goofy rider has the right hand at the back.
        let (rear, front) = if self.mirrored() { (l, r) } else { (r, l) };
        let held = match (rear, front) {
            (true, true) => Some(GrabKind::Double),
            (true, false) => Some(GrabKind::Indy),
            (false, true) => Some(GrabKind::Melon),
            (false, false) => None,
        };
        let free = self.trick.is_none_or(|t| t.progress() >= 1.0);
        // The grab's name is the graph's (its `ScoringGrabs` variant), else the hand's base grab.
        let graph = self.graph;
        let name_of = |kind: GrabKind| graph.grab_name.unwrap_or(kind.names().0);
        let tweak = graph.tweak;
        match (&mut self.grab, held) {
            (Some(g), Some(kind)) => {
                g.time += dt;
                // Adding the second hand turns it into a double grab.
                if kind == GrabKind::Double {
                    g.kind = kind;
                }
                let name = name_of(g.kind);
                if kind == GrabKind::Double && g.name == g.base {
                    g.base = name;
                }
                if name != g.base || g.name == g.base {
                    g.name = name;
                }
                g.tweak = tweak;
            }
            (Some(g), None) => {
                let g = *g;
                if self.grabbed.is_none_or(|b| g.time > b.time) {
                    self.grabbed = Some(g);
                }
                self.grab = None;
            }
            (None, Some(kind)) if free => {
                self.grab = Some(Grab {
                    kind,
                    time: 0.0,
                    name: name_of(kind),
                    base: name_of(kind),
                    tweak,
                })
            }
            (None, _) => {}
        }
    }

    /// Touching down on a surface (`surface`: its collision surface id). Bails only on Skate 3's own
    /// landing rules (SK8 player/wipeout/air.rs `bad_landing`, the danger zone's closing-speed check):
    /// the board too far off its travel for the speed along the surface, too fast into the ground (or
    /// stairs), or the board not caught yet. Otherwise the board keeps its heading (no snap to the
    /// travel: the wheels' grip scrubs the sideways speed and straightens it), the speed along the
    /// surface is kept (gaining on transitions), and the landing is classified as Skate 3 does.
    fn land(&mut self, normal: Vec3, surface: u16, air_time: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        if self.takeoff.onto_free && !self.land_on_free_board(normal, events) {
            return;
        }
        // The board lands as the rider lined it up (board adjust): its nose is the landing's. The
        // physical board flew turned to the animated one (the clips' adjust, `fly_rig`): Skate 3
        // lands that board, so its nose is the landing's.
        self.bake_deck_adjust();
        if self.body_flip.is_none()
            && let Some(r) = self.rig.as_ref().filter(|r| r.airborne)
        {
            let up = self.normal.normalize_or(Vec3::UP);
            let nose = tangent(r.board.deck().axes.cols[2] * self.rig_sign(), up);
            if nose.length() > 0.5 {
                self.fwd = nose;
                self.sync_heading();
            }
        }
        self.air_com = None;
        self.air_lift = 0.0;
        let n = normal.normalize_or(Vec3::UP);
        let vt = self.vel - n * self.vel.dot(n);
        // A body flip ends with the landing: its turn baked into the board, which has to come down on
        // its wheels (SK8 ResetBodyFlip).
        self.flipped = None;
        if let Some(f) = self.body_flip.take() {
            if f.apply(self.normal).dot(n) < flip::LAND_MIN_DOT {
                self.normal = n;
                self.vel = vt;
                self.bail("landed off the body flip", events);
                return;
            }
            let (whole, rest) = f.turns();
            let turns = whole + u32::from(rest > PI);
            if turns > 0 {
                self.flipped = Some((f.front, turns));
            }
        }
        let speed = vt.length();
        let impact = -self.vel.dot(n);
        // Skate 3's closing velocity (SK8 physics/board_ground.rs: the speed into the surface along its
        // normal) seen from the rider (wipeout/common.rs `closing`, the animation frame): along his up,
        // and across it when he comes down tilted against the surface.
        let rider_up = self.normal.normalize_or(n);
        let into = impact.max(0.0);
        let cos = n.dot(rider_up).clamp(-1.0, 1.0);
        let closing_up = into * cos.abs();
        let closing_side = into * sqrtf((1.0 - cos * cos).max(0.0));
        let axis = tangent(self.fwd, n);
        let travel = if speed > 0.5 {
            vt * (1.0 / speed)
        } else if self.speed < 0.0 {
            -axis
        } else {
            axis
        };
        // The landing angle: the board against the travel along the surface, from the nearer end.
        let off = atan2f(axis.cross(travel).dot(n), axis.dot(travel).clamp(-1.0, 1.0));
        let backwards = off.abs() > PI / 2.0;
        let crooked_signed = if backwards { wrap_angle(off + PI) } else { off };
        let crooked = crooked_signed.abs();
        // The board isn't caught yet: the clips' `DANGERZONE`.
        let danger = self.graph.danger;
        let stairs = crate::park::surface_category(surface) == crate::park::STAIRS;
        // Skate 3's bad landing (SK8 wipeout/air.rs `bad_landing`): made only where the physics mode
        // checks it, its limits times the mode's scale.
        let scale = t.bad_landing_scale;
        let max_impact = if !t.bad_landing {
            f32::INFINITY
        } else if stairs {
            t.land_max_stairs * scale
        } else {
            t.land_max_impact * scale
        };
        let limit = if t.bad_landing {
            t.land_angle.eval(speed) * scale
        } else {
            f32::INFINITY
        };
        let reason = if danger
            && air_time * 60.0 > t.danger_ignore_ticks
            && (closing_up > t.danger_max_impact || closing_side > t.danger_max_side)
        {
            Some("landed before the board was caught")
        } else if crooked > limit {
            Some("landed too crooked for the speed")
        } else if impact > max_impact {
            Some(if stairs {
                "hit the stairs too hard"
            } else {
                "hit the ground too hard"
            })
        } else {
            None
        };
        self.last_landing = Some(LandingCheck {
            crooked,
            impact,
            speed,
            limit,
        });
        self.normal = n;
        if let Some(r) = reason {
            self.vel = vt;
            self.bail(r, events);
            return;
        }
        // A grab still held ends with the landing.
        if let Some(g) = self.grab.take()
            && self.grabbed.is_none_or(|b| g.time > b.time)
        {
            self.grabbed = Some(g);
        }
        let spin = self.body.speed;
        // KnownAir's exit onto the ground (SK8 known/trajectory.rs `restore_velocity`): the speed along
        // the surface scaled by `LandingSpeedScalarVsGroundNormalY`, as far as the travel goes downhill;
        // PhysicsAir's exit keeps the board's.
        let scale = if self.plan.is_some() {
            let downhill = n.cross(n.cross(Vec3::UP)).normalize_or(Vec3::ZERO);
            let w = downhill.dot(vt.normalize_or(Vec3::ZERO)).clamp(0.0, 1.0);
            (t.landing_speed_vs_ny.eval(n.y) - 1.0) * w + 1.0
        } else {
            1.0
        };
        let along = vt * scale;
        self.contact_n = n;
        self.fwd = axis;
        self.set_ground_velocity(along);
        // Skate 3's rigid board comes down on its wheels: their friction against the touchdown's
        // impact takes the sideways speed off a crooked landing (measured on SK8: about 0.4 of the
        // speed into the ground, a 70-degree landing at 6 m/s keeping 0.75 m/s).
        let into = (-(self.vel + Vec3::UP * self.air_lift_rate).dot(n)).max(0.0);
        let scrub = (LAND_SIDE_FRICTION * into).min(self.slip.abs());
        self.slip -= scrub * self.slip.signum();
        // The physical board leaves the air with that velocity (its speed into the ground kept), which
        // the ground ride takes over from it.
        let ground = self.ground_velocity();
        if let Some(r) = self.rig.as_mut().filter(|r| r.airborne) {
            let keep = n * r.board.deck().velocity.dot(n);
            for p in &mut r.board.parts {
                p.velocity = ground + keep;
            }
        }
        let quality = landing::classify(&t, along, self.fwd, n, spin);
        let kind = match quality.kind {
            1 => LandKind::Sketchy {
                frontside: crooked_signed * self.frontside_sign(backwards) > 0.0,
            },
            2 => LandKind::Spin,
            _ => LandKind::Clean,
        };
        if kind == LandKind::Spin {
            // Still turning: the board pivots on a moment.
            self.collide = Some((spin, SPIN_PIVOT_TIME));
        }
        self.landing = Some(Landing {
            kind,
            time: 0.0,
            crooked,
            impact,
            quality,
        });
        self.vn = 0.0;
        self.gap_time = 0.0;
        self.lead_rise = 0.0;
        self.truck_ramp = 0.0;
        self.pump = Pump::default();
        self.vel = self.ground_velocity();
        self.fakie = self.speed < 0.0;
        self.fakie_timer = 0.0;
        // Half-turns spun with the body, named frontside or backside for the rider's stance.
        let halves = libm::roundf(self.spin / PI) as i32;
        let grab = self.grabbed.take();
        // A foot plant this air is scored as Skate 3 names it (`fsgrab_fs_footplant` ...), for its grab.
        let footplant = self.footplanted.take();
        let grab_name = footplant.or(grab.map(|g| g.name));
        let mut label = self.trick_label();
        let mut body_flips = 0;
        if let Some((front, turns)) = self.flipped.take() {
            let flip = match (front, turns) {
                (true, 1) => String::from("FRONTFLIP"),
                (false, 1) => String::from("BACKFLIP"),
                (true, n) => format!("{n}X FRONTFLIP"),
                (false, n) => format!("{n}X BACKFLIP"),
            };
            body_flips = turns;
            label = Some(label.map_or((flip.clone(), None), |(l, id)| (format!("{flip} {l}"), id)));
        }
        let fs = (halves as f32) * self.frontside_sign(self.takeoff.fakie) > 0.0;
        if let Some(name) = landed_name(
            self.stance_prefix(),
            label.as_ref().map(|l| l.0.as_str()),
            halves,
            fs,
            grab_name,
        ) {
            // Skate 3's scoring: the trick, the grab held longest, a foot plant, the spin and body flips
            // (the air's distance and height the game's scoring measures itself).
            let score = crate::scoring::Air {
                trick: label.and_then(|l| l.1),
                grab: grab.and_then(|g| Some((crate::scoring::catalog::grab_id(g.name)?, g.time))),
                footplant: footplant.and_then(crate::scoring::catalog::id),
                halves: halves.unsigned_abs(),
                body_flips,
                landing: match kind {
                    LandKind::Clean => crate::scoring::Landing::Clean,
                    LandKind::Sketchy { .. } => crate::scoring::Landing::Sketchy,
                    LandKind::Spin => crate::scoring::Landing::Plain,
                },
            };
            let name = if matches!(kind, LandKind::Sketchy { .. }) {
                format!("SKETCHY {name}")
            } else {
                name
            };
            events.push(Event::Landed {
                name,
                points: 0,
                score: crate::scoring::Scored::Air(score),
            });
        }
        self.publish_wheels(4);
        self.trick = None;
        self.plant_trick = None;
        self.spin = 0.0;
        self.spin_rate = 0.0;
        self.plan = None;
        self.takeoff.hippy = false;
    }

    /// Touching down from the flight onto the free board (the caveman). On its deck: onto the board
    /// where it is, its nose the rider's way (or its tail, when LandOnBoard turns him onto it the wrong
    /// way round), only the speed along it kept (its wheels take the rest); then an ordinary landing
    /// (true). Missed (or the board went): on his feet beside it (Skate 3's `HippyRunout`); faster than
    /// `MaxSpeedLandingOnBoard`: a bail (false either way).
    fn land_on_free_board(&mut self, normal: Vec3, events: &mut Vec<Event>) -> bool {
        self.takeoff.onto_free = false;
        self.takeoff.mount = false;
        let n = normal.normalize_or(Vec3::UP);
        if !self.possession.under(self.pos) {
            let travel = self.vel.horizontal();
            let heading = if travel.length() > 0.3 {
                atan2f(travel.x, travel.z)
            } else {
                self.heading
            };
            self.biped = Biped::new(heading, self.vel - n * self.vel.dot(n), n);
            self.states.facts.dismount = true;
            self.trick = None;
            self.plan = None;
            self.sync_walk();
            return false;
        }
        if self.vel.length() > self.tuning.possession.max_landing_speed {
            self.vel = self.vel - n * self.vel.dot(n);
            self.possession.under_feet();
            self.bail("came down onto the board too fast", events);
            return false;
        }
        let b = self.possession.board;
        self.pos.x = b.pos.x;
        self.pos.z = b.pos.z;
        let nose = if self.graph.land_backwards {
            -b.nose
        } else {
            b.nose
        };
        let axis = tangent(nose, n);
        let vt = self.vel - n * self.vel.dot(n);
        self.vel = axis * vt.dot(axis) + n * self.vel.dot(n);
        self.fwd = axis;
        self.normal = n;
        self.possession.under_feet();
        true
    }
}

/// `v` along the plane with normal `n`, unit length (a sideways direction if `v` is along `n`).
pub fn tangent(v: Vec3, n: Vec3) -> Vec3 {
    let t = v - n * v.dot(n);
    if t.length() > 1e-5 {
        return t.normalize_or(v);
    }
    let other = if n.y.abs() < 0.9 {
        Vec3::UP
    } else {
        v3(1.0, 0.0, 0.0)
    };
    n.cross(other).normalize_or(v3(0.0, 0.0, 1.0))
}

/// `v` turned by `angle` (rad) about the unit `axis` (positive: anticlockwise looking down the axis).
pub fn rotate(v: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let (s, c) = (sinf(angle), cosf(angle));
    v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c))
}

fn approach(v: f32, target: f32, by: f32) -> f32 {
    if v > target {
        (v - by).max(target)
    } else {
        (v + by).min(target)
    }
}

/// A graph's grab name (`Indy_Grab`) as a landed name's word (`INDY GRAB`).
fn shout(name: &str) -> String {
    name.replace('_', " ").to_ascii_uppercase()
}

/// "FAKIE FS 360 KICKFLIP INDY" style names. `None` for nothing worth naming (rolled off a lip, no spin,
/// no grab).
fn landed_name(
    stance: &str,
    trick: Option<&str>,
    halves: i32,
    frontside: bool,
    grab: Option<&str>,
) -> Option<String> {
    use core::fmt::Write;
    let mut s = String::from(stance);
    if halves != 0 {
        let dir = if frontside { "FS" } else { "BS" };
        let _ = write!(s, "{dir} {} ", halves.unsigned_abs() * 180);
    }
    match (trick, grab) {
        (Some(d), Some(g)) => {
            let _ = write!(s, "{d} {}", shout(g));
        }
        (Some(d), None) => s.push_str(d),
        (None, Some(g)) => s.push_str(&shout(g)),
        (None, None) if halves != 0 => s.push_str("AIR"),
        (None, None) => return None,
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flick::FlickOut;
    use crate::tuning::Difficulty;
    use alloc::vec;

    const DT: f32 = 1.0 / 60.0;

    fn flat() -> Park {
        Park {
            shapes: Vec::new(),
            half_size: 50.0,
            wall_height: 2.0,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
            rails: Vec::new(),
        }
    }

    /// A skater on `park`, past the just-spawned fakie grace.
    fn spawn(park: &dyn Terrain) -> Skater {
        let mut s = Skater::spawn(park);
        s.since_spawn = FAKIE_GRACE;
        s
    }

    /// One tick with the state graph popping `f` at full strength (its takeoff clip reaching the jump,
    /// as the game's graph hands it to the physics).
    fn popped(s: &mut Skater, park: &dyn Terrain, pad: &Pad, f: Flick, ev: &mut Vec<Event>) {
        s.graph.pop = Some((f, 1.0));
        s.step(park, pad, &FlickOut::default(), DT, ev);
        s.graph.pop = None;
    }

    /// `n` ticks; with `flick` the state graph pops it on the first.
    fn steps(
        s: &mut Skater,
        park: &dyn Terrain,
        pad: &Pad,
        flick: Option<Flick>,
        n: usize,
    ) -> Vec<Event> {
        let mut ev = Vec::new();
        for i in 0..n {
            match flick {
                Some(f) if i == 0 => popped(s, park, pad, f, &mut ev),
                _ => s.step(park, pad, &FlickOut::default(), DT, &mut ev),
            }
        }
        ev
    }

    fn press(b: u32) -> Pad {
        Pad {
            held: b,
            pressed: b,
            ..Default::default()
        }
    }

    fn hold(b: u32) -> Pad {
        Pad {
            held: b,
            ..Default::default()
        }
    }

    fn landed(ev: &[Event]) -> Vec<&str> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Landed { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn main_stick_reads_like_skate_3() {
        // Straight forward: no turn; to the side: full turn; back and to the side: powerslide or hard turn.
        assert_eq!(steering([0.0, 1.0]).turn, 0.0);
        assert!((steering([1.0, 0.0]).turn - 1.0).abs() < 1e-5);
        assert!((steering([-0.6, 0.0]).turn + 0.6).abs() < 1e-5);
        let a = 30.0 * PI / 180.0;
        assert_eq!(steering([sinf(a), -cosf(a)]).slide, 1.0);
        let a = 65.0 * PI / 180.0;
        let s = steering([-sinf(a), -cosf(a)]);
        assert!(s.hard < 0.0 && s.slide == 0.0, "{s:?}");
    }

    #[test]
    fn push_and_brake_timing_comes_from_the_motion_library() {
        use crate::motion::write::{AttrIn, ClipIn, write};
        // Made-up clips for the slow strong set only, and the brake's wind-up.
        let clip = |name: String, frames: f32, attrs: Vec<(&str, f32)>| ClipIn {
            name,
            fps: 60.0,
            frames,
            base_speed: 1.0,
            andale_flags: 0,
            has_pose: false,
            attrs: attrs
                .into_iter()
                .map(|(n, v)| AttrIn {
                    name: n.into(),
                    kind: crate::motion::KIND_SCALAR,
                    begin: -1.0,
                    end: -1.0,
                    words: alloc::vec![v.to_bits()],
                })
                .collect(),
        };
        let base = PUSH_CLIPS[0];
        let mut clips = alloc::vec![
            clip(format!("{base}_INTO"), 31.0, alloc::vec![("VEL_E", 3.0)]),
            clip(
                format!("{base}_CYC1"),
                7.0,
                alloc::vec![("HSTR_VEL_B", 0.5), ("VEL_E", 3.0)]
            ),
            clip(format!("{base}_CYC2"), 61.0, alloc::vec![("VEL_E", 3.0)]),
            clip(BRAKE_INTO_CLIP.into(), 13.0, alloc::vec![]),
        ];
        clips.sort_by(|a, b| a.name.cmp(&b.name));
        let bytes = write(&clips, &[]);
        let db = crate::motion::MotionDb::parse(&bytes).unwrap();
        let c = ClipTiming::from_motion(&db);
        let set = c.push[0];
        assert!((set.into - 0.5).abs() < 1e-5 && (set.contact - 0.1).abs() < 1e-5);
        assert!((set.recover - 1.0).abs() < 1e-5);
        assert_eq!((set.vel_b, set.vel_e), (0.5, 3.0));
        assert!((c.brake_down - 0.2).abs() < 1e-5);
        assert_eq!(
            c.push[1],
            ClipTiming::BUILT_IN.push[1],
            "missing sets stay built in"
        );
    }

    /// The main stick at `deg` degrees from straight back (+ right), fully over.
    fn stick_back(deg: f32) -> Pad {
        let a = deg * PI / 180.0;
        Pad {
            stick: [sinf(a), -cosf(a)],
            ..Default::default()
        }
    }

    #[test]
    fn powerslide_holds_curves_and_ends() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 6.0;
        let yaw0 = s.travel_yaw();
        steps(&mut s, &park, &stick_back(30.0), None, 30);
        let sl = s.slide.expect("sliding");
        // The board is well round from its travel (how far follows the stick), and speed scrubs off.
        assert!(
            sl.amount > 0.6 && s.board_yaw_offset().abs() > 1.0,
            "{sl:?}"
        );
        assert!(s.speed < 5.0, "{}", s.speed);
        // The path curves (the trucks hold a turn).
        assert!(wrap_angle(s.travel_yaw() - yaw0).abs() > 0.05);
        // Swinging the stick round the side keeps it (the wide holding arc).
        steps(&mut s, &park, &stick_back(100.0), None, 10);
        assert!(s.slide.is_some_and(|s| !s.releasing));
        // Letting go ends it, and the board comes back in line.
        steps(&mut s, &park, &Pad::default(), None, 30);
        assert!(s.slide.is_none());
        // A slide lasts at least its minimum time even after a flick.
        let mut s = spawn(&park);
        s.speed = 6.0;
        steps(&mut s, &park, &stick_back(30.0), None, 2);
        steps(&mut s, &park, &Pad::default(), None, 5);
        assert!(s.slide.is_some_and(|s| !s.releasing));
        // Holding the stick in the entry window doesn't start another one, and slow riding ends it.
        let mut s = spawn(&park);
        s.speed = 3.0;
        steps(&mut s, &park, &stick_back(30.0), None, 240);
        assert!(s.speed.abs() < 2.0);
        assert!(s.slide.is_none() || s.slide.is_some_and(|s| s.releasing));
        let mut s = spawn(&park);
        s.speed = 0.8;
        steps(&mut s, &park, &stick_back(30.0), None, 5);
        assert!(s.slide.is_none(), "too slow to slide");
    }

    // ---- Pinned to SK8's simulation (SK8-ENGINE/skate-3-rust-engine run headless with the stock tuning,
    // the same scripted pad on both; see docs/graph-runner.md, "Measured against SK8") ----

    /// The travel's turn rate (rad/s, + left) over the last half second of 1.5 s with the main stick at
    /// `x`, from `speed`, and the speed at the end.
    fn carve(speed: f32, x: f32) -> (f32, f32) {
        let park = flat();
        let mut s = spawn(&park);
        s.put_at(
            &park,
            v3(-40.0, 0.0, -40.0),
            core::f32::consts::FRAC_PI_4,
            speed,
        );
        steps(&mut s, &park, &Pad::default(), None, 5);
        let pad = Pad {
            stick: [x, 0.0],
            ..Pad::default()
        };
        steps(&mut s, &park, &pad, None, 60);
        let y0 = s.travel_yaw();
        steps(&mut s, &park, &pad, None, 30);
        (
            wrap_angle(s.travel_yaw() - y0) / 0.5,
            s.ground_velocity().length(),
        )
    }

    #[test]
    fn carves_turn_as_fast_as_sk8_and_cost_no_speed() {
        // SK8: full stick 1.75 rad/s at 6.5 m/s and 1.83 at 8.3 m/s; half a stick 0.42 at 6.65 m/s and
        // 0.48 at 8.46 m/s (a 16 m and a 17.5 m radius). The speed model keeps most of the speed
        // through them: SK8's full carve from 6.65 m/s settles at 6.5 (its wheels slip 7 degrees).
        for (speed, x, want) in [
            (6.5, -1.0, 1.75),
            (8.3, -1.0, 1.83),
            (6.65, -0.5, 0.42),
            (8.46, -0.5, 0.48),
        ] {
            let (rate, end) = carve(speed, x);
            assert!(
                (rate - want).abs() < 0.08 * want,
                "{speed} m/s, stick {x}: {rate} rad/s, SK8 {want}"
            );
            assert!(end > speed * 0.96, "{speed} m/s carving ends at {end}");
        }
        // Mirrored.
        let (rate, _) = carve(6.5, 1.0);
        assert!((rate + 1.75).abs() < 0.14, "{rate}");
    }

    #[test]
    fn carves_weaken_through_a_push_and_settle_as_sk8s() {
        // SK8 at 6.5 m/s: let go of a full carve, the yaw rate is under a tenth of it after 0.25 s.
        // Through a push's whole animation (its contact events, the foot down and coming back) the
        // steering is at `PushScalarMin`, recovering by `PushScalarIncriment` a tick after.
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 6.5;
        let left = Pad {
            stick: [-1.0, 0.0],
            ..Pad::default()
        };
        steps(&mut s, &park, &left, None, 90);
        let full = s.yaw_rate.abs();
        let mut settle = None;
        for i in 0..60 {
            steps(&mut s, &park, &Pad::default(), None, 1);
            if settle.is_none() && s.yaw_rate.abs() < 0.1 * full {
                settle = Some((i + 1) as f32 * DT);
            }
        }
        let settle = settle.expect("settles");
        assert!((0.18..0.3).contains(&settle), "{settle} s (SK8 0.25)");
        // Pushing: half the tilt.
        let half = Pad {
            stick: [-0.5, 0.0],
            ..Pad::default()
        };
        let rate = |pushing: bool| {
            let mut s = spawn(&park);
            s.speed = 6.65;
            s.graph = GraphControl {
                pushing,
                ..GraphControl::default()
            };
            steps(&mut s, &park, &half, None, 60);
            s.yaw_rate.abs()
        };
        let (free, pushing) = (rate(false), rate(true));
        let min = s.tuning.push_steer_min;
        assert!(
            (pushing / free - min).abs() < 0.1,
            "{pushing} vs {free} (x{min})"
        );
    }

    #[test]
    fn kick_turns_build_up_and_run_down_as_sk8s() {
        // SK8 standing: the yaw rate builds to 3.2 rad/s and turns about 148 degrees in the second the
        // stick is held; let go, it runs down by 0.7 a tick.
        let park = flat();
        let mut s = spawn(&park);
        let yaw = |s: &Skater| atan2f(s.fwd.x, s.fwd.z);
        let y0 = yaw(&s);
        let pad = Pad {
            stick: [-1.0, 0.0],
            ..Pad::default()
        };
        // The graph's KickTurn state, at its stock spin scale.
        s.graph.kickturn = Some(KICKTURN_SPIN_SCALE);
        let mut peak: f32 = 0.0;
        for _ in 0..60 {
            let before = yaw(&s);
            steps(&mut s, &park, &pad, None, 1);
            peak = peak.max(wrap_angle(yaw(&s) - before).abs() * 60.0);
        }
        let turned = wrap_angle(yaw(&s) - y0).abs().to_degrees();
        assert!((135.0..160.0).contains(&turned), "{turned} degrees");
        assert!((peak - 3.2).abs() < 0.15, "peak {peak} rad/s");
        let before = yaw(&s);
        s.graph.kickturn = None;
        steps(&mut s, &park, &Pad::default(), None, 1);
        let after = wrap_angle(yaw(&s) - before).abs() * 60.0;
        assert!((after - 0.7 * peak).abs() < 0.3, "let go: {after} rad/s");
        // Rolling slows it: about 2.0 rad/s at 3.4 m/s.
        assert!((kickturn_rate(3.4) - 2.0).abs() < 1e-3 && kickturn_rate(1.8) < 2.8);
    }

    #[test]
    fn the_foot_brake_slows_as_sk8s() {
        // SK8: once the foot drags, 4.08 m/s^2 (its force 50 on smooth concrete).
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 6.65;
        // The brake clip's foot is down (its `BRAKE_CONTACT`).
        s.graph.brake_contact = true;
        steps(&mut s, &park, &hold(btn::X), None, 20);
        let v0 = s.speed;
        steps(&mut s, &park, &hold(btn::X), None, 30);
        let decel = (v0 - s.speed) * 2.0;
        assert!((decel - 4.08).abs() < 0.2, "{decel} m/s^2");
    }

    #[test]
    fn a_powerslide_brings_the_travel_round_as_sk8s() {
        // SK8 from 6.65 m/s, the stick back and to the side: the travel turns about 150 degrees and the
        // speed falls to about 2 m/s in the slide's second.
        let park = flat();
        let mut s = spawn(&park);
        s.put_at(&park, v3(-30.0, 0.0, -30.0), 0.0, 6.65);
        steps(&mut s, &park, &Pad::default(), None, 2);
        let y0 = atan2f(s.ground_velocity().x, s.ground_velocity().z);
        let mut turned: f32 = 0.0;
        let mut last = y0;
        let mut ticks = 0;
        for _ in 0..90 {
            steps(&mut s, &park, &stick_back(-35.0), None, 1);
            let v = s.ground_velocity();
            let y = atan2f(v.x, v.z);
            turned += wrap_angle(y - last);
            last = y;
            ticks += 1;
            if s.slide.is_none_or(|sl| sl.releasing) {
                break;
            }
        }
        let turned = turned.abs().to_degrees();
        assert!(
            (110.0..200.0).contains(&turned),
            "{turned} degrees in {ticks} ticks"
        );
        assert!(s.speed.abs() < 3.0, "{}", s.speed);
    }

    #[test]
    fn ollie_lands_on_flat_and_a_quicker_flick_pops_higher() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 5.0;
        let ev = steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 120);
        assert_eq!(ev[0], Event::Popped(Flick::OLLIE));
        assert_eq!(landed(&ev), ["OLLIE"], "{ev:?}");
        assert!(s.rolling());
        assert!(s.pos.z > 3.0);

        let apex = |strength: f32| {
            let mut s = spawn(&park);
            s.speed = 4.0;
            let mut ev = Vec::new();
            s.graph.pop = Some((Flick::OLLIE, strength));
            s.step(&park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
            s.graph.pop = None;
            let mut top: f32 = 0.0;
            for _ in 0..90 {
                s.step(&park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
                top = top.max(s.pos.y);
            }
            top
        };
        assert!(apex(1.0) > apex(0.0) + 0.2, "{} {}", apex(1.0), apex(0.0));
    }

    #[test]
    fn the_physical_board_carries_the_known_air_and_lands_on_its_wheels() {
        // SK8's ollie: the jump launches the prediction, known that tick (straight into KnownAir); the
        // physical board follows the trajectory with its hook turning it only, and touches down on its
        // own wheels when the trajectory meets the ground.
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 5.0;
        let mut ev = steps(&mut s, &park, &Pad::default(), None, 5);
        popped(&mut s, &park, &Pad::default(), Flick::OLLIE, &mut ev);
        assert_eq!(s.state(), State::KnownAir);
        let contact = s.plan.as_ref().map(|p| p.contact.time).unwrap();
        let mut ticks = 0;
        let mut follows = true;
        while s.is_airborne() && ticks < 120 {
            let before = s
                .plan
                .map(|p| p.target(s.air_time + DT, s.tuning.traj_error_blend));
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            ticks += 1;
            // The board is where the trajectory puts it (the deck over the flight's point).
            if let (Some(p), true) = (before, s.is_airborne()) {
                let d = s.rig.as_ref().unwrap().board.deck().position - s.deck();
                follows &= d.length() < 0.01 && (s.pos - p).length() < 0.01;
            }
        }
        assert!(follows, "the board follows the known air");
        assert!(s.rolling(), "{ev:?}");
        assert_eq!(landed(&ev), ["OLLIE"], "{ev:?}");
        // Down when the trajectory meets the ground, within a tick or two of the prediction.
        let air = ticks as f32 * DT;
        assert!((air - contact).abs() < 2.5 * DT, "{air} vs {contact}");
        // On its wheels: the deck at its riding height.
        let deck = s.rig.as_ref().unwrap().board.deck().position;
        assert!((deck.y - DECK_HEIGHT).abs() < 0.02, "{deck:?}");
    }

    #[test]
    fn a_board_rolled_off_a_ledge_flies_free_and_lands_on_its_own_physics() {
        // PhysicsAir without a pop: no jump velocity, the board falls on its own and its wheels meet the
        // ground below; the selector takes it down once the air state has settled.
        let park = flat();
        let mut s = spawn(&park);
        s.pos = v3(0.0, 0.3, 0.0);
        s.vel = v3(0.0, 0.0, 3.0);
        s.force_state(State::PhysicsAir);
        let mut ev = Vec::new();
        let mut ticks = 0;
        while s.is_airborne() && ticks < 60 {
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            ticks += 1;
        }
        assert!(s.rolling(), "{ev:?}");
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        // About the fall's time (0.25 s for 0.3 m), not held up.
        assert!((10..=20).contains(&ticks), "{ticks}");
        assert!((s.speed - 3.0).abs() < 0.3, "{}", s.speed);
    }

    #[test]
    fn every_flick_trick_lands_on_flat() {
        let park = flat();
        for t in crate::gesture::TRICKS {
            for nollie in [false, true] {
                let f = Flick { trick: t, nollie };
                let mut s = spawn(&park);
                s.speed = 4.0;
                let ev = steps(&mut s, &park, &Pad::default(), Some(f), 120);
                assert!(!ev.contains(&Event::Bailed), "{f:?}: {ev:?}");
                assert_eq!(landed(&ev).len(), 1, "{f:?}: {ev:?}");
            }
        }
        // Nollie names.
        let mut s = spawn(&park);
        s.speed = 4.0;
        let f = Flick {
            trick: Trick::TreFlip,
            nollie: true,
        };
        let ev = steps(&mut s, &park, &Pad::default(), Some(f), 120);
        assert_eq!(landed(&ev), ["NOLLIE 360 FLIP"]);
    }

    /// Touches down on flat ground moving at `speed` along +z (and `down` m/s into it) with the board
    /// `deg` degrees off the travel (+ nose towards +x), spinning at `spin` rad/s.
    fn touch_down(deg: f32, speed: f32, down: f32, spin: f32) -> (Skater, Vec<Event>) {
        let park = flat();
        let mut s = spawn(&park);
        s.take_off(Some(Flick::OLLIE));
        s.select_state();
        let a = deg * PI / 180.0;
        s.fwd = v3(sinf(a), 0.0, cosf(a));
        s.vel = v3(0.0, -down, speed);
        s.body.speed = spin;
        let mut ev = Vec::new();
        s.states.time = 0.6;
        s.land(Vec3::UP, 0, 0.6, &mut ev);
        s.select_state();
        (s, ev)
    }

    #[test]
    fn a_sideways_landing_at_riding_speed_rides_away() {
        // Skate 3's landing angle limit is past a right angle below about 9 m/s: no sideways bail.
        let (s, ev) = touch_down(80.0, 5.0, 3.0, 0.0);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        assert!(s.rolling());
        // The board keeps its heading (no snap to the travel) and the wheels straighten it.
        assert!(s.fwd.x > 0.9, "{:?}", s.fwd);
        let park = flat();
        let mut s = s;
        steps(&mut s, &park, &Pad::default(), None, 30);
        let off = libm::acosf(s.fwd.dot(s.vel.normalize_or(s.fwd)).abs().min(1.0));
        assert!(off < 0.3, "straightened to {off}");
    }

    #[test]
    fn the_landing_angle_curve_bails_only_at_speed() {
        let t = Tuning::BUILT_IN;
        // 50 degrees off: fine at 6 m/s, too crooked fast.
        let (_, ev) = touch_down(50.0, 6.0, 2.0, 0.0);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        let fast = 25.0;
        assert!(t.land_angle.eval(fast) < 50.0 * PI / 180.0);
        let (s, ev) = touch_down(50.0, fast, 2.0, 0.0);
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
        assert_eq!(s.bail_reason, Some("landed too crooked for the speed"));
        // Too hard into the ground at any angle.
        let (_, ev) = touch_down(0.0, 4.0, t.land_max_impact + 1.0, 0.0);
        assert!(ev.contains(&Event::Bailed));
    }

    #[test]
    fn landings_are_classified_as_skate_3_does() {
        let kind = |deg: f32, speed: f32, spin: f32| {
            let (s, ev) = touch_down(deg, speed, 2.0, spin);
            assert!(!ev.contains(&Event::Bailed));
            s.landing.expect("landed")
        };
        assert_eq!(kind(0.0, 5.0, 0.0).kind, LandKind::Clean);
        assert!(matches!(
            kind(25.0, 5.0, 0.0).kind,
            LandKind::Sketchy { .. }
        ));
        // Slow: straight whatever the angle.
        assert_eq!(kind(25.0, 1.5, 0.0).kind, LandKind::Clean);
        // Still spinning towards the travel: a spin landing that keeps pivoting a moment.
        let (s, _) = touch_down(-25.0, 5.0, 2.0, 3.0);
        assert_eq!(s.landing.map(|l| l.kind), Some(LandKind::Spin));
        assert!(s.collide.is_some_and(|(rate, _)| rate > 0.0));
        // A sketchy landing scores half; a clean one keeps its speed along the ground, a crooked one
        // loses some of the sideways part to the wheels on the impact (2 m/s into the ground).
        let (s, ev) = touch_down(25.0, 5.0, 2.0, 0.0);
        let (c, ev2) = touch_down(0.0, 5.0, 2.0, 0.0);
        let points = |ev: &[Event]| {
            ev.iter()
                .find_map(|e| match e {
                    Event::Landed { points, .. } => Some(*points),
                    _ => None,
                })
                .unwrap_or(0)
        };
        assert_eq!(landed(&ev), ["SKETCHY OLLIE"]);
        assert_eq!(points(&ev), points(&ev2) / 2);
        assert!((c.ground_velocity().length() - 5.0).abs() < 1e-3);
        let a = 25.0f32.to_radians();
        let side = 5.0 * sinf(a) - LAND_SIDE_FRICTION * 2.0;
        let kept = sqrtf((5.0 * cosf(a)).powi(2) + side * side);
        assert!(
            (s.ground_velocity().length() - kept).abs() < 0.05,
            "{} vs {kept}",
            s.ground_velocity().length()
        );
    }

    #[test]
    fn transitions_add_speed_on_landing() {
        // SK8 `restore_velocity`: out of KnownAir, the speed along the surface times
        // `LandingSpeedScalarVsGroundNormalY`, weighted by how far the travel goes downhill.
        let t = Tuning::BUILT_IN;
        let land = |down_hill: bool| {
            let park = flat();
            let mut s = spawn(&park);
            s.vel = v3(0.0, 3.0, 4.0);
            s.take_off(Some(Flick::OLLIE));
            s.update_prediction(&park, 0.0);
            assert!(s.plan.is_some(), "a known landing");
            s.select_state();
            s.trick = None;
            let a = 0.98; // about 56 degrees: the surface rises towards +z
            let n = v3(0.0, cosf(a), -sinf(a));
            let dir = if down_hill { -1.0 } else { 1.0 };
            s.fwd = tangent(v3(0.0, 0.0, dir), n);
            s.normal = n;
            s.vel = s.fwd * 5.0 - n * 2.0;
            let mut ev = Vec::new();
            s.states.time = 0.6;
            s.land(n, 0, 0.6, &mut ev);
            s.select_state();
            (s.speed, n)
        };
        let (down, n) = land(true);
        let gain = t.landing_speed_vs_ny.eval(n.y);
        assert!(gain > 1.05);
        assert!((down - 5.0 * gain).abs() < 1e-3, "{down}");
        let (up, _) = land(false);
        assert!((up - 5.0).abs() < 1e-3, "{up}");
    }

    #[test]
    fn a_bail_puts_the_rider_back_on_a_safe_spot_not_the_spawn() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        // Ride a while (checkpoints), then bail.
        for _ in 0..200 {
            s.speed = 4.0;
            steps(&mut s, &park, &Pad::default(), None, 1);
        }
        assert!(!s.checkpoints.is_empty());
        let rode_to = s.pos;
        s.bail("test", &mut Vec::new());
        s.select_state();
        let mut ev = Vec::new();
        let mut ticks = 0;
        while !ev.contains(&Event::Respawned) && ticks < 60 * 25 {
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            ticks += 1;
        }
        assert!(ev.contains(&Event::Respawned));
        let secs = ticks as f32 / 60.0;
        let t = Tuning::BUILT_IN;
        assert!(
            secs > t.reset_min_time && secs < t.reset_min_time + t.reset_settled + 1.0,
            "{secs}"
        );
        // Back near where it came to rest (a safe spot), not at the park's spawn.
        assert!(
            (s.pos - rode_to).length() < 3.0,
            "{:?} vs {rode_to:?}",
            s.pos
        );
        assert!(s.pos.length() > 5.0);
        assert_eq!(s.stance, Stance::Goofy);
        // The player can skip the wait once settled.
        s.speed = 4.0;
        steps(&mut s, &park, &Pad::default(), None, 5);
        s.bail("test", &mut Vec::new());
        s.select_state();
        let mut ev = steps(&mut s, &park, &Pad::default(), None, 100);
        ev.extend(steps(&mut s, &park, &press(btn::A), None, 1));
        ev.extend(steps(&mut s, &park, &Pad::default(), None, 4));
        assert!(ev.contains(&Event::Respawned), "{ev:?}");
    }

    #[test]
    fn an_unsafe_rest_spot_falls_back_to_a_recent_checkpoint() {
        let park = flat();
        let mut s = spawn(&park);
        for _ in 0..200 {
            s.speed = 4.0;
            steps(&mut s, &park, &Pad::default(), None, 1);
        }
        let n = s.checkpoints.len();
        assert!(n >= 2, "{n}");
        // Came to rest outside the park's ground (no floor under it within reach).
        s.pos = v3(0.0, -50.0, 0.0);
        s.respawn(&park);
        assert!(s.pos.y.abs() < 0.01 && s.pos.z > 3.0, "{:?}", s.pos);
        assert_eq!(
            s.checkpoints.len(),
            n - 1,
            "the checkpoint used is taken out"
        );
    }

    #[test]
    fn half_spin_lands_fakie_then_pushing_turns_round_to_switch() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        let mut ev = steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
        s.set_spin(PI);
        ev.extend(steps(&mut s, &park, &Pad::default(), None, 80));
        assert!(s.fakie && s.speed < 0.0, "{ev:?}");
        // Goofy, spinning left: the back leads (backside).
        assert_eq!(landed(&ev), ["BS 180 OLLIE"], "{ev:?}");
        // Travel stays along +z; the board's nose points back.
        assert!(s.travel_yaw().abs() < 0.1);
        // The graph turns the rider round (pushing out of fakie).
        s.graph.turn_round = true;
        let ev = steps(&mut s, &park, &press(btn::A), None, 1);
        s.graph.turn_round = false;
        assert!(ev.contains(&Event::Switched));
        assert!(s.switch && !s.fakie && s.speed > 0.0 && s.mirrored());
        assert!(s.heading.abs() < 0.1, "nose leads again");
        // A fakie flick is named as such.
        let mut s = spawn(&park);
        s.speed = -4.0;
        steps(&mut s, &park, &Pad::default(), None, 2);
        assert!(s.fakie);
        let ev = steps(
            &mut s,
            &park,
            &Pad::default(),
            Some(Flick::of(Trick::Kickflip)),
            120,
        );
        assert_eq!(landed(&ev), ["FAKIE KICKFLIP"]);
    }

    #[test]
    fn rolling_backwards_slowly_is_not_fakie_at_once() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = -0.7;
        steps(&mut s, &park, &Pad::default(), None, 5);
        assert!(!s.fakie);
        steps(&mut s, &park, &Pad::default(), None, 15);
        assert!(s.fakie);
        // Creeping back slower than Skate 3's limit is not fakie.
        s.speed = -0.4;
        steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(!s.fakie);
        // Never just after a spawn.
        let mut s = Skater::spawn(&park);
        s.speed = -4.0;
        steps(&mut s, &park, &Pad::default(), None, 30);
        assert!(!s.fakie);
        steps(&mut s, &park, &Pad::default(), None, 40);
        assert!(s.fakie);
    }

    #[test]
    fn revert_after_landing_fakie_rides_forward_again() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 5.0;
        steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
        s.set_spin(PI);
        for _ in 0..80 {
            steps(&mut s, &park, &Pad::default(), None, 1);
            if s.rolling() {
                break;
            }
        }
        assert!(s.fakie);
        // The graph's revert gesture (frontside).
        s.graph.revert = Some(true);
        let mut ev = steps(&mut s, &park, &Pad::default(), None, 1);
        s.graph.revert = None;
        ev.extend(steps(&mut s, &park, &Pad::default(), None, 90));
        assert_eq!(landed(&ev), ["REVERT"], "{ev:?}");
        assert!(!s.fakie && s.speed > 0.0);
    }

    #[test]
    fn the_landing_assist_straightens_a_slightly_off_landing_with_the_stick_let_go() {
        let park = flat();
        let land = |spin: f32| {
            let mut s = spawn(&park);
            s.speed = 5.0;
            let mut ev = steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
            s.set_spin(spin);
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 100));
            (s, ev)
        };
        let (s, ev) = land(25.0 * PI / 180.0);
        assert_eq!(landed(&ev), ["OLLIE"], "{ev:?}");
        // Skate 3's controller slows a spin no faster than its opposite-direction limit: it may run a
        // little past, but lands straight (not sketchy).
        let straight =
            |l: Option<Landing>| l.is_some_and(|l| l.crooked < 0.15 && l.kind == LandKind::Clean);
        assert!(straight(s.landing), "{:?}", s.landing);
        // A 180 short by 30 degrees is finished round to fakie.
        let (s, ev) = land(150.0 * PI / 180.0);
        assert!(landed(&ev).iter().any(|n| n.contains("180")), "{ev:?}");
        assert!(s.fakie && straight(s.landing), "{:?}", s.landing);
        // Without the assist the same takes the landing sketchy.
        let mut s = spawn(&park);
        s.speed = 5.0;
        steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
        s.set_spin(25.0 * PI / 180.0);
        // The graph's `PhysBodySpin` nudged off centre.
        s.graph.body_spin = 0.0001;
        let ev = steps(&mut s, &park, &Pad::default(), None, 100);
        assert!(
            landed(&ev).iter().any(|n| n.starts_with("SKETCHY")),
            "{ev:?}"
        );
    }

    #[test]
    fn a_let_go_spin_carries_on() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
        // The graph's `PhysBodySpin`: the stick hard left, then let go.
        s.graph.body_spin = -1.0;
        steps(&mut s, &park, &Pad::default(), None, 8);
        let at_release = s.spin;
        let rate = s.spin_rate;
        assert!(rate > 1.0, "{rate}");
        s.graph.body_spin = 0.0;
        steps(&mut s, &park, &Pad::default(), None, 8);
        assert!(s.is_airborne());
        assert!(s.spin > at_release + 0.2, "{} -> {}", at_release, s.spin);
    }

    #[test]
    fn big_drops_land_and_bigger_ones_bail() {
        let park = flat();
        let drop = |h: f32| {
            let mut s = spawn(&park);
            s.pos.y = h;
            s.take_off(None);
            s.select_state();
            let mut ev = Vec::new();
            for _ in 0..150 {
                ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
                if !s.is_airborne() && s.rolling() {
                    break;
                }
            }
            (s, ev)
        };
        let (s, ev) = drop(3.0);
        assert!(
            !ev.contains(&Event::Bailed) && s.landing.is_some(),
            "{ev:?}"
        );
        let (_, ev) = drop(8.0);
        assert!(ev.contains(&Event::Bailed));
    }

    #[test]
    fn a_quick_pull_for_an_ollie_is_not_a_manual() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        let mut ev = Vec::new();
        for y in [0.5, 1.0, 1.0, 1.0, 1.0, 1.0] {
            let f = FlickOut {
                stick: [0.0, y],
                crouch: 1.0,
                ..FlickOut::default()
            };
            s.step(&park, &Pad::default(), &f, DT, &mut ev);
        }
        assert!(s.manual.is_none());
        assert!(s.crouch > 0.5);
    }

    #[test]
    fn hippie_jump_from_both_push_buttons() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        let both = btn::A | btn::B;
        let mut ev = steps(&mut s, &park, &press(both), None, 1);
        ev.extend(steps(&mut s, &park, &hold(both), None, 20));
        assert!(s.hippy.is_some());
        ev.extend(steps(&mut s, &park, &Pad::default(), None, 120));
        assert_eq!(landed(&ev), ["HIPPIE JUMP"], "{ev:?}");
    }

    #[test]
    fn grabs_follow_the_hands() {
        let park = flat();
        let grab = |trig_r: f32| Pad {
            trig_r,
            ..Default::default()
        };
        let mut s = spawn(&park);
        s.speed = 5.0;
        let mut ev = steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 5);
        ev.extend(steps(&mut s, &park, &grab(1.0), None, 18));
        // Goofy: the right hand is the rear hand.
        assert_eq!(s.grab.map(|g| g.kind), Some(GrabKind::Indy));
        ev.extend(steps(&mut s, &park, &grab(0.0), None, 80));
        assert_eq!(landed(&ev), ["OLLIE INDY"], "{ev:?}");

        let mut s = Skater::spawn_with(&park, Tuning::BUILT_IN, Stance::Regular);
        s.speed = 5.0;
        steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 5);
        steps(&mut s, &park, &grab(1.0), None, 5);
        assert_eq!(s.grab.map(|g| g.kind), Some(GrabKind::Melon));
        // Held into the landing without a tweak: it lands.
        let ev = steps(&mut s, &park, &grab(1.0), None, 80);
        assert_eq!(landed(&ev), ["OLLIE MELON"], "{ev:?}");
        // The graph names the variant (`ScoringGrabs`, from the C-stick).
        let mut s = spawn(&park);
        s.speed = 5.0;
        steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 5);
        s.graph.grab_name = Some("NOSEGRAB");
        let pad = Pad {
            trig_l: 1.0,
            ..Default::default()
        };
        steps(&mut s, &park, &pad, None, 5);
        assert_eq!(s.grab.map(|g| g.name), Some("NOSEGRAB"));
    }

    #[test]
    fn the_c_stick_picks_a_grab_variant() {
        let (base, v) = GrabKind::Melon.names();
        assert_eq!(select_grab(base, v, 0.2, 0.3), "MELON");
        assert_eq!(select_grab(base, v, 0.0, 0.9), "NOSEGRAB");
        assert_eq!(select_grab(base, v, 0.0, -0.9), "MUTE");
        // Left and right are missing: up and down grow into them.
        assert_eq!(select_grab(base, v, -0.9, 0.2), "NOSEGRAB");
        assert_eq!(select_grab(base, v, 0.9, -0.2), "MUTE");
        // All four present: each keeps its own quarter.
        let all = [Some("U"), Some("L"), Some("D"), Some("R")];
        assert_eq!(select_grab("B", all, -0.9, 0.0), "L");
        assert_eq!(select_grab("B", all, 0.9, 0.0), "R");
        assert_eq!(grab_variant([true; 4], 0.1, 0.1), None);
    }

    #[test]
    fn a_wound_up_spin_turns_further() {
        let park = flat();
        let spun = |windup: bool| {
            let mut s = spawn(&park);
            s.speed = 4.0;
            let mut ev = Vec::new();
            // Either the stick is already over (no wind-up), or it is thrown across just before the pop.
            let before: Vec<f32> = if windup {
                let mut v = vec![0.0; 36];
                v.extend([0.4, 0.8, 1.0, 1.0]);
                v
            } else {
                vec![1.0; 40]
            };
            for x in before {
                let pad = Pad {
                    stick: [-x, 0.0],
                    ..Default::default()
                };
                s.step(&park, &pad, &FlickOut::default(), DT, &mut ev);
            }
            let pad = Pad {
                stick: [-1.0, 0.0],
                ..Default::default()
            };
            popped(&mut s, &park, &pad, Flick::OLLIE, &mut ev);
            for _ in 0..20 {
                s.step(&park, &pad, &FlickOut::default(), DT, &mut ev);
            }
            s.spin
        };
        assert!(
            spun(true) > spun(false) * 1.3,
            "{} {}",
            spun(true),
            spun(false)
        );
    }

    /// Flat ground with a 0.5 m high thin rail along +Z from z = 0 to z = `len`.
    fn rail_park(len: f32) -> Park {
        let mut p = flat();
        p.spawn = v3(0.0, 0.0, -3.0);
        p.rails = vec![crate::rails::Rail::new(vec![
            v3(0.0, 0.5, 0.0),
            v3(0.0, 0.5, len),
        ])];
        p
    }

    /// Flat ground with a 0.4 m high ledge (top over x 0..2, z -10..10), its edge at x = 0 a rail.
    fn ledge_park() -> crate::collision::TriMesh {
        let mut tris: Vec<([[f32; 3]; 3], u16, u16)> = Vec::new();
        let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            tris.push(([a, b, c], 0, 0));
            tris.push(([a, c, d], 0, 0));
        };
        quad(
            [-20.0, 0.0, -30.0],
            [-20.0, 0.0, 30.0],
            [20.0, 0.0, 30.0],
            [20.0, 0.0, -30.0],
        );
        quad(
            [0.0, 0.4, -10.0],
            [0.0, 0.4, 10.0],
            [2.0, 0.4, 10.0],
            [2.0, 0.4, -10.0],
        );
        quad(
            [0.0, 0.0, -10.0],
            [0.0, 0.4, -10.0],
            [0.0, 0.4, 10.0],
            [0.0, 0.0, 10.0],
        );
        let mut m = crate::collision::TriMesh::parse(
            &crate::collision::encode(&tris),
            (v3(-0.6, 0.0, -9.0), 0.0),
        )
        .unwrap();
        let rail = crate::rails::Rail::new(vec![v3(0.0, 0.4, -10.0), v3(0.0, 0.4, 10.0)]);
        let rail = rail.classified(&m);
        m.rails = vec![rail];
        m
    }

    /// Ollies towards the rail; `spin` (rad) is set just after the pop.
    fn ollie_at_rail(park: &Park, spin: f32) -> (Skater, Vec<Event>) {
        let mut s = spawn(park);
        s.speed = 5.0;
        let mut ev = steps(&mut s, park, &Pad::default(), Some(Flick::OLLIE), 1);
        s.set_spin(spin);
        for _ in 0..60 {
            ev.extend(steps(&mut s, park, &Pad::default(), None, 1));
            if s.grinding() {
                break;
            }
        }
        (s, ev)
    }

    #[test]
    fn ollie_onto_a_rail_grinds_and_scores_off_the_end() {
        let park = rail_park(4.0);
        let (mut s, ev) = ollie_at_rail(&park, 0.0);
        assert!(s.grinding(), "{ev:?}");
        // Onto the rail's start on the front truck, both trucks on it a moment later.
        steps(&mut s, &park, &Pad::default(), None, 6);
        let g = s.grind.clone().unwrap();
        assert_eq!(g.family, Family::FiftyFifty);
        assert_eq!(g.class, crate::rails::RailClass::Thin);
        // The trucks' hangers on the rail: the deck's centre that far over it.
        let deck = s.pos.y + DECK_HEIGHT;
        assert!(
            g.dir > 0.0 && (deck - (0.5 + HANGER_DROP)).abs() < 1e-3,
            "{:?}",
            s.pos
        );
        let ev = steps(&mut s, &park, &Pad::default(), None, 120);
        let names = landed(&ev);
        assert!(
            names.len() == 1 && names[0].starts_with("OLLIE ") && names[0].ends_with(" 50-50"),
            "{ev:?}"
        );
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        assert!(s.rolling());
        assert!(s.pos.z > 4.0);
    }

    #[test]
    fn board_across_the_rail_is_a_boardslide_and_a_flick_pops_off() {
        let park = rail_park(20.0);
        let (mut s, _) = ollie_at_rail(&park, PI / 2.0);
        assert_eq!(s.grind.as_ref().map(|g| g.family), Some(Family::Boardslide));
        steps(&mut s, &park, &Pad::default(), None, 20);
        assert!(s.grinding());
        let ev = steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
        assert!(
            matches!(&ev[..], [Event::Landed { name, .. }, Event::Popped(_)]
                if name.ends_with("BOARDSLIDE") || name.ends_with("LIPSLIDE")),
            "{ev:?}"
        );
        assert!(s.is_airborne() && s.vel.y > 0.0);
        let ev = steps(&mut s, &park, &Pad::default(), None, 90);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    }

    #[test]
    fn a_grinds_pop_is_the_physics_modes_for_its_family() {
        // SK8 grind/substate.rs: the pop's speed along the launch normal is the mode's blend for the
        // family (x 1.03), not a ground jump's height. A boardslide pops with its own pair.
        let park = rail_park(20.0);
        let pop = |pair: [f32; 2]| {
            let (mut s, _) = ollie_at_rail(&park, PI / 2.0);
            assert_eq!(s.grind.as_ref().map(|g| g.family), Some(Family::Boardslide));
            steps(&mut s, &park, &Pad::default(), None, 20);
            s.tuning.grind.pop = [[9.0, 9.0], pair, [9.0, 9.0]];
            steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
            assert!(s.is_airborne());
            s.vel.y
        };
        let (strong, weak) = (pop([3.0, 3.0]), pop([1.0, 1.0]));
        assert!(
            (strong - weak - 2.0 * 1.03).abs() < 0.15,
            "{strong} vs {weak}: 2 m/s apart along the launch normal"
        );
    }

    #[test]
    fn an_angled_board_locks_into_a_one_truck_grind() {
        let park = rail_park(10.0);
        let (s, ev) = ollie_at_rail(&park, 40.0 * PI / 180.0);
        let g = s.grind.as_ref().expect("grinding");
        assert_eq!(g.family, Family::FiveO, "{ev:?}");
        assert!(
            matches!(
                g.kind(),
                GrindKind::Crooked
                    | GrindKind::Overcrook
                    | GrindKind::Willy
                    | GrindKind::Overwilly
                    | GrindKind::Feeble
                    | GrindKind::Smith
                    | GrindKind::Salad
            ),
            "{:?}",
            g.graph_name()
        );
    }

    /// A board over the rail at x = 0 (along +Z, 0.5 m up): its deck `deck_up` over the rail, its nose
    /// along `fwd`, its middle at `at` along the rail and `side` across it.
    fn over_rail(fwd: Vec3, along: f32, side: f32, deck_up: f32) -> (Park, Skater) {
        let park = rail_park(10.0);
        let mut s = spawn(&park);
        place(
            &mut s,
            v3(side, 0.5 + deck_up - DECK_HEIGHT, 5.0 + along),
            Vec3::UP,
            fwd,
        );
        s.vel = v3(0.0, -1.0, 2.0);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        (park, s)
    }

    #[test]
    fn the_family_is_the_box_the_rail_crosses() {
        let along = v3(0.0, 0.0, 1.0);
        let across = v3(1.0, 0.0, 0.0);
        let family = |fwd: Vec3, at: f32| {
            let (park, s) = over_rail(fwd, at, 0.0, 0.1);
            s.investigate(&park, Category::Air, None, 0.0)
                .map(|c| (c.family, c.front))
        };
        // Along the rail, over it: both trucks.
        assert_eq!(family(along, 0.0), Some((Family::FiftyFifty, false)));
        // Across it: the deck between the trucks, or a tip past them.
        assert_eq!(family(across, 0.0).map(|f| f.0), Some(Family::Boardslide));
        let (park, s) = over_rail(across, 0.0, -0.33, 0.1);
        let c = s.investigate(&park, Category::Air, None, 0.0).unwrap();
        assert_eq!((c.family, c.front), (Family::Tipslide, true), "the nose");
        // Angled 45 degrees with only the back truck over the rail: a 5-0 on it.
        let diag = v3(0.707, 0.0, 0.707);
        let (park, s) = over_rail(diag, 0.0, 0.0, 0.1);
        let mut s = s;
        s.pos += v3(0.17, 0.0, 0.17);
        let c = s.investigate(&park, Category::Air, None, 0.0).unwrap();
        assert_eq!((c.family, c.front), (Family::FiveO, false));
        // Too far above the rail: nothing.
        let (park, s) = over_rail(along, 0.0, 0.0, 0.4);
        assert!(s.investigate(&park, Category::Air, None, 0.0).is_none());
    }

    #[test]
    fn the_stick_tips_a_50_50_into_a_nosegrind_or_a_5_0() {
        for (stick, front, name) in [(1.0, true, "NOSEGRIND"), (-1.0, false, "5-0")] {
            let park = rail_park(30.0);
            let (mut s, _) = ollie_at_rail(&park, 0.0);
            steps(&mut s, &park, &Pad::default(), None, 6);
            assert_eq!(s.grind.as_ref().map(|g| g.family), Some(Family::FiftyFifty));
            let pad = Pad {
                flick: [0.0, stick],
                ..Pad::default()
            };
            steps(&mut s, &park, &pad, None, 30);
            let g = s.grind.clone().expect("still grinding");
            assert_eq!((g.family, g.front), (Family::FiveO, front));
            // Let go: the free end is held up, the name settles and scores.
            let ev = steps(&mut s, &park, &Pad::default(), None, 20);
            let g = s.grind.clone().expect("still grinding");
            assert!(g.display().ends_with(name), "{} {ev:?}", g.display());
            assert!(!g.graph_name().starts_with("BF_"));
        }
    }

    #[test]
    fn the_sticks_full_to_the_side_drop_a_50_50_onto_one_truck() {
        // Skate 3's 50-50 needs the sticks' x under 0.9: past it the board is on one truck, which the
        // sticks then turn across the rail.
        let park = rail_park(30.0);
        let (mut s, _) = ollie_at_rail(&park, 0.0);
        steps(&mut s, &park, &Pad::default(), None, 10);
        assert_eq!(s.grind.as_ref().map(|g| g.family), Some(Family::FiftyFifty));
        let pad = Pad {
            stick: [1.0, 0.0],
            ..Pad::default()
        };
        let ev = steps(&mut s, &park, &pad, None, 30);
        let g = s.grind.clone().expect("still grinding");
        assert_eq!(g.family, Family::FiveO, "{ev:?}");
        assert!(!g.front, "the back truck without the C-stick up");
        assert!(g.twist.abs() > 0.3, "turned across: {}", g.twist);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    }

    #[test]
    fn a_boardslide_stalling_on_a_rail_hangs_up() {
        // Across a thin rail at a crawl: it slows to a stop and wipes out (Skate 3's slides do on
        // rails, SK8 involuntary.rs); the same on a ledge steps off.
        let (park, mut s) = over_rail(v3(1.0, 0.0, 0.0), 0.0, 0.0, 0.03);
        s.vel = v3(0.0, -0.5, 0.6);
        let ev = steps(&mut s, &park, &Pad::default(), None, 1);
        assert_eq!(
            s.grind.as_ref().map(|g| g.family),
            Some(Family::Boardslide),
            "{ev:?}"
        );
        let ev = steps(&mut s, &park, &Pad::default(), None, 120);
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
        assert_eq!(s.bail_reason, Some("the slide hung up on the rail"));
    }

    #[test]
    fn a_grind_round_a_sharp_bend_falls_off_it() {
        // A rail turning 60 degrees to the side: the speed across the new segment stays the board's
        // (the pins catch it, or it falls off): at 5 m/s it goes on straight off the bend. None of it
        // goes into the rail, so it is no closing speed and no wipeout.
        let mut park = rail_park(4.0);
        park.rails[0].points.push(v3(
            5.0 * libm::sinf(PI / 3.0),
            0.5,
            4.0 + 5.0 * libm::cosf(PI / 3.0),
        ));
        park.rails[0] = crate::rails::Rail::new(park.rails[0].points.clone());
        let (mut s, ev) = ollie_at_rail(&park, 0.0);
        assert!(s.grinding(), "{ev:?}");
        let mut ev = Vec::new();
        for _ in 0..90 {
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            if s.grind.is_none() {
                break;
            }
        }
        assert_eq!(s.grind_exit, Some("fell off"), "{ev:?}");
        assert!(s.pos.z > 4.0 && s.pos.z < 4.5, "at the bend: {:?}", s.pos);
        assert!(
            s.vel.z > 3.0 && s.vel.x.abs() < 0.5,
            "straight on: {:?}",
            s.vel
        );
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    }

    #[test]
    fn frontside_or_backside_comes_from_where_the_rider_took_off() {
        // Parallel to the ledge, beside it on its low side, popping straight up onto it: the chest
        // faces the ledge (frontside) or away from it (backside), by stance.
        let park = ledge_park();
        for (stance, fs) in [(Stance::Goofy, true), (Stance::Regular, false)] {
            let mut s = Skater::spawn_with(&park, Tuning::BUILT_IN, stance);
            s.since_spawn = FAKIE_GRACE;
            place(&mut s, v3(-0.25, 0.0, -9.0), Vec3::UP, v3(0.0, 0.0, 1.0));
            s.speed = 4.0;
            let chest_to_ledge = s.chest().x > 0.0;
            let pad = Pad {
                stick: [0.0, 0.0],
                ..Pad::default()
            };
            let mut ev = steps(&mut s, &park, &pad, Some(Flick::OLLIE), 1);
            // Drift over the ledge's edge on the way up.
            for _ in 0..40 {
                if s.is_airborne() {
                    s.pos.x = (s.pos.x + 0.012).min(0.0);
                }
                ev.extend(steps(&mut s, &park, &pad, None, 1));
                if s.grinding() {
                    break;
                }
            }
            let g = s.grind.clone().unwrap_or_else(|| panic!("{ev:?}"));
            assert_eq!(g.family, Family::FiftyFifty);
            assert_eq!(chest_to_ledge, fs, "goofy faces +x riding +z");
            assert_eq!(g.frontside(), fs, "{}", g.graph_name());
        }
    }

    #[test]
    fn a_tip_over_a_ledges_high_side_is_a_blunt() {
        let park = ledge_park();
        assert_eq!(
            park.rails[0].class_at(5.0).0,
            crate::rails::RailClass::Ledge
        );
        let mut s = spawn(&park);
        // Across the ledge's edge, the board's middle over the top (+x), the tail on the edge.
        place(
            &mut s,
            v3(0.33, 0.4 + 0.1 - DECK_HEIGHT, 0.0),
            Vec3::UP,
            v3(1.0, 0.0, 0.0),
        );
        s.vel = v3(0.0, -1.0, 3.0);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        let c = s.investigate(&park, Category::Air, None, 0.0).unwrap();
        assert_eq!((c.family, c.front), (Family::Backslash, false));
        // The same tip with the board hanging over the low side: a tail slide.
        place(
            &mut s,
            v3(-0.33, 0.4 + 0.1 - DECK_HEIGHT, 0.0),
            Vec3::UP,
            v3(-1.0, 0.0, 0.0),
        );
        let c = s.investigate(&park, Category::Air, None, 0.0).unwrap();
        assert_eq!(c.family, Family::Tipslide);
    }

    #[test]
    fn friction_follows_the_family_and_the_rail_class() {
        use crate::rails::RailClass::*;
        let t = Tuning::BUILT_IN;
        let f = |fam: Family, class| grind::friction_decel(&t, fam, class, 1.0, 0.0);
        assert!(f(Family::Backslash, Thin) > f(Family::Boardslide, Thin));
        assert!(f(Family::Boardslide, Thin) > f(Family::FiftyFifty, Thin));
        assert!(f(Family::Tipslide, Thin) > f(Family::FiftyFifty, Thin));
        // Grinds drag less on ledges than on thin rails; slides more.
        assert!(f(Family::FiftyFifty, Ledge) < f(Family::FiftyFifty, Thin));
        assert!(f(Family::Boardslide, Ledge) > f(Family::Boardslide, Thin));
        // About flatground's old 0.8 m/s^2 for a 50-50 on a thin rail; more the longer it lasts.
        assert!((f(Family::FiftyFifty, Thin) - 0.8).abs() < 0.2);
        assert!(
            grind::friction_decel(&t, Family::FiftyFifty, Thin, 1.0, 3.0)
                > f(Family::FiftyFifty, Thin)
        );
    }

    #[test]
    fn a_long_ledge_grind_leans_off_the_low_side() {
        let park = ledge_park();
        let mut s = spawn(&park);
        place(
            &mut s,
            v3(0.0, 0.4 + 0.05 - DECK_HEIGHT, -9.0),
            Vec3::UP,
            v3(0.0, 0.0, 1.0),
        );
        s.vel = v3(0.0, -0.5, 4.0);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        let mut ev = steps(&mut s, &park, &Pad::default(), None, 2);
        assert!(s.grinding(), "{ev:?}");
        assert_eq!(
            s.grind.as_ref().map(|g| g.class),
            Some(crate::rails::RailClass::Ledge)
        );
        let mut leaned = 0.0f32;
        let mut ticks = 0;
        while s.grinding() && ticks < 400 {
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            if let Some(g) = &s.grind {
                leaned = leaned.max(g.lean.angle);
            }
            ticks += 1;
        }
        // Off after the lean-off's delay (2 s) and its roll (about 1.3 s), before the ledge's end.
        assert!(!s.grinding() && ticks > 150, "{ticks} {ev:?}");
        // Forced off past 15 degrees: there is ground under the low side.
        assert!(leaned > 10.0, "{leaned}");
        assert!(s.vel.x < -0.3, "pushed off the low side: {:?}", s.vel);
        assert!(s.pos.z < 9.0, "{:?}", s.pos);
        let ev = steps(&mut s, &park, &Pad::default(), None, 60);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    }

    #[test]
    fn coming_down_on_a_rail_steeper_than_the_deck_allows_bails() {
        let park = rail_park(10.0);
        let mut s = spawn(&park);
        // The nose pitched 70 degrees down along the rail, the deck's middle on it.
        let fwd = v3(0.0, -0.94, 0.34);
        let up = v3(0.0, 0.34, 0.94);
        place(&mut s, v3(0.0, 0.5, 5.0) - up * DECK_HEIGHT, up, fwd);
        s.vel = v3(0.0, -2.0, 2.0);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        let mut ev = Vec::new();
        let c = s.investigate(&park, Category::Air, None, 0.0);
        if let Some(c) = c {
            assert!(s.engage(&park, c, Category::Air, &mut ev));
            s.select_state();
            assert!(ev.contains(&Event::Bailed), "{c:?} {ev:?}");
        } else {
            // No box crosses it: no grind either.
            assert!(s.is_airborne() && s.air_time == 0.5);
        }
        // Crossing a thin rail fast: a wipeout.
        let (park, mut s) = over_rail(v3(0.0, 0.0, 1.0), 0.0, 0.0, 0.1);
        s.vel = v3(8.0, -1.0, 2.0);
        let mut ev = Vec::new();
        let c = s.investigate(&park, Category::Air, None, 0.0).unwrap();
        assert!(s.engage(&park, c, Category::Air, &mut ev));
        s.select_state();
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
    }

    #[test]
    fn popping_out_steers_sideways_no_faster_than_the_cap() {
        let park = rail_park(30.0);
        let (mut s, _) = ollie_at_rail(&park, 0.0);
        steps(&mut s, &park, &Pad::default(), None, 10);
        let along = s.grind.as_ref().unwrap().speed;
        let pad = Pad {
            stick: [1.0, 0.0],
            ..Pad::default()
        };
        let ev = steps(&mut s, &park, &pad, Some(Flick::OLLIE), 1);
        assert!(ev.iter().any(|e| matches!(e, Event::Popped(_))), "{ev:?}");
        let side = s.vel.x.abs();
        assert!(side > 0.5 && side <= 1.8 + 1e-3, "{:?}", s.vel);
        assert!((s.vel.z - along).abs() < 0.3, "{:?} {along}", s.vel);
        assert!(s.jumper.energy < 0.5);
        // Popping again straight away is weaker on energy: the pop goes off to a side.
        assert_eq!(s.jumper.since_jump, 0);
    }

    #[test]
    fn no_grind_straight_after_popping_out_of_one_on_any_rail() {
        // Two parallel rails 0.3 m apart: popping off one, the board can't take the other at once.
        let mut park = rail_park(30.0);
        park.rails.push(crate::rails::Rail::new(vec![
            v3(0.25, 0.5, 0.0),
            v3(0.25, 0.5, 30.0),
        ]));
        let (mut s, _) = ollie_at_rail(&park, 0.0);
        let first = s.grind.as_ref().unwrap().rail;
        steps(&mut s, &park, &Pad::default(), None, 10);
        steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
        assert!(s.is_airborne());
        // Straight onto the other rail on the next ticks: too soon.
        s.pos.x = 0.25;
        s.vel.y = -1.0;
        let ev = steps(&mut s, &park, &Pad::default(), None, 4);
        assert!(!s.grinding(), "{ev:?}");
        // A moment later it can.
        steps(&mut s, &park, &Pad::default(), None, 8);
        s.pos = v3(0.25, 0.5 + 0.05 - DECK_HEIGHT, s.pos.z);
        s.vel = v3(0.0, -1.0, 4.0);
        s.plan = None;
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        let ev = steps(&mut s, &park, &Pad::default(), None, 1);
        let now = s.grind.as_ref().map(|g| g.rail);
        assert_eq!(now, Some(1), "{ev:?} (first {first})");
    }

    #[test]
    fn rolling_off_a_ledges_top_along_its_edge_is_a_50_50() {
        let park = ledge_park();
        let mut s = spawn(&park);
        // On the ledge's top, rolling along it 8 degrees towards its edge.
        let a = 8.0 * PI / 180.0;
        place(
            &mut s,
            v3(0.35, 0.4, -8.0),
            Vec3::UP,
            v3(-sinf(a), 0.0, cosf(a)),
        );
        s.speed = 4.0;
        s.vel = s.ground_velocity();
        let mut ev = Vec::new();
        for _ in 0..120 {
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            if s.grinding() {
                break;
            }
        }
        let g = s
            .grind
            .clone()
            .unwrap_or_else(|| panic!("{:?} {ev:?}", s.pos));
        assert!(
            matches!(g.family, Family::FiftyFifty | Family::FiveO),
            "{:?}",
            g.family
        );
        assert_eq!(g.entry, Entry::RideFromAbove);
        assert!(!ev.contains(&Event::Bailed));
    }

    /// The quarter pipe with its coping (the deck's edge at z = 3, 4 m up) a rail.
    fn coping_pipe() -> crate::collision::TriMesh {
        let mut m = quarter_pipe();
        let rail = crate::rails::Rail::new(vec![v3(-5.0, 4.0, 3.0), v3(5.0, 4.0, 3.0)]);
        let rail = rail.classified(&m);
        m.rails = vec![rail];
        m
    }

    #[test]
    fn a_crooked_landing_loses_its_sideways_speed_to_the_wheels() {
        // SK8 landing 72 degrees crooked at 5.7 m/s, 6.1 m/s into the ground: the rigid board's
        // wheels take 2.4 m/s of the sideways speed off on the impact (0.41 per m/s into it).
        let park = flat();
        let mut s = spawn(&park);
        let yaw = 72.0f32.to_radians();
        place(
            &mut s,
            v3(0.0, 0.08, 0.0),
            Vec3::UP,
            v3(sinf(yaw), 0.0, cosf(yaw)),
        );
        s.vel = v3(0.0, -6.1, 5.7);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        let mut n = 0;
        while !s.rolling() && n < 10 {
            steps(&mut s, &park, &Pad::default(), None, 1);
            n += 1;
        }
        assert!(s.rolling());
        let side = 5.7 * sinf(yaw) - LAND_SIDE_FRICTION * 6.1;
        assert!((s.slip.abs() - side).abs() < 0.25, "{} vs {side}", s.slip);
    }

    #[test]
    fn rolling_slowly_off_a_deck_drops_in_instead_of_stopping_on_the_coping() {
        // SK8 at 1.4 m/s off a half pipe's deck: the board tips over the coping and rolls into the
        // transition. The deck a deck's height over the coping is outside Skate 3's drop-in depth
        // window (0.06 m), and while the board is still tipping over the edge a rail is ridden onto,
        // not landed on (flatground caught it as a 0 m/s nose blunt or lipslide).
        let park = coping_pipe();
        let mut s = spawn(&park);
        place(&mut s, v3(0.0, 4.0, 3.8), Vec3::UP, v3(0.0, 0.0, -1.0));
        s.speed = 1.4;
        for _ in 0..240 {
            steps(&mut s, &park, &Pad::default(), None, 1);
            assert!(
                !s.grinding(),
                "caught the coping at {:?} {:?}",
                s.pos,
                s.grind.as_ref().map(|g| (g.entry, g.family))
            );
        }
        assert!(
            s.pos.y < 2.0 && s.speed.abs() > 3.0,
            "{:?} {}",
            s.pos,
            s.speed
        );
    }

    #[test]
    fn the_transition_compresses_the_rider_and_springing_back_pumps() {
        // SK8 pumping/controller.rs: the ground's angle asks the animation for a crouch
        // (`CompressionVsGroundAngle` x `CompressionGroundScalar`, `MinCrouchVsGroundAngle`); the
        // animated centre of mass rising over the board on a curving transition pays out speed, with
        // no input at `UnintentionalPumpScalar` of the pump, at most 10 m/s^2 (0.117 m/s a tick).
        let park = quarter_pipe();
        let mut s = spawn(&park);
        let (n, f) = (v3(0.0, 0.866, -0.5), v3(0.0, 0.5, 0.866));
        place(&mut s, v3(0.0, 0.4, 1.5), n, f);
        s.speed = 6.0;
        let mut h = 0.5;
        s.graph = GraphControl {
            pose_com: Some(h),
            ..GraphControl::default()
        };
        // Put down mid-transition with no pitch rate, the board's back wheels lift off the curve for
        // a few ticks before it rides it (a board riding in from the flat never does).
        const SETTLE: usize = 4;
        steps(&mut s, &park, &Pad::default(), None, SETTLE);
        assert!(s.pump.ground_absorption < -1.0, "{:?}", s.pump);
        assert!(s.pump.min_crouch > 0.1, "{:?}", s.pump);
        let plain = {
            let mut p = spawn(&park);
            place(&mut p, v3(0.0, 0.4, 1.5), n, f);
            p.speed = 6.0;
            p.graph = GraphControl {
                pose_com: Some(h),
                ..GraphControl::default()
            };
            steps(&mut p, &park, &Pad::default(), None, SETTLE + 7);
            p.speed
        };
        let mut most = 0.0f32;
        for _ in 0..7 {
            h += 0.02;
            s.graph = GraphControl {
                pose_com: Some(h),
                ..GraphControl::default()
            };
            steps(&mut s, &park, &Pad::default(), None, 1);
            most = most.max(s.pump.dv);
        }
        // (The rigid board gives some of the pump back to the transition's slope: 0.24 here.)
        assert!(s.speed > plain + 0.2, "{} vs {plain}", s.speed);
        // SK8's most a tick; the rigid board's averaged wheel normals read the transition's turn a
        // little lower (0.106).
        assert!((most - 0.7 * 10.0 * DT).abs() < 0.012, "{most}");
    }

    #[test]
    fn riding_up_into_the_coping_rocks_on_it_and_comes_back_in() {
        let park = coping_pipe();
        let (class, high) = park.rails[0].class_at(5.0);
        assert_eq!(class, crate::rails::RailClass::Ledge);
        assert!(high.z > 0.9, "the deck is the high side: {high:?}");
        // Somewhere in the speeds that just reach the lip, a ride-in tip (from below or into the
        // coping), no bail, then rolling back down into the pipe. (Other speeds may air out and come
        // back onto the coping from the air instead.)
        for k in 0..15 {
            let speed = 8.3 + k as f32 * 0.05;
            let mut s = spawn(&park);
            place(&mut s, v3(0.0, 0.0, -3.0), Vec3::UP, v3(0.0, 0.0, 1.0));
            s.speed = speed;
            let mut ev = Vec::new();
            let mut rocked = None;
            let mut back_in = false;
            for _ in 0..300 {
                ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
                if s.grinding() && rocked.is_none() {
                    rocked = s.grind.clone();
                }
                // Back in the pipe after the rock, rolling down it.
                back_in |= rocked.is_some() && s.rolling() && s.pos.y < 3.5;
            }
            let ride_in = rocked.as_ref().is_some_and(|g| {
                g.family == Family::Tipslide
                    && matches!(g.entry, Entry::RideIntoCoping | Entry::RideFromBelow)
            });
            if ride_in && back_in && !ev.contains(&Event::Bailed) {
                return;
            }
        }
        panic!("no speed rocked onto the coping");
    }

    #[test]
    fn the_air_assist_reaches_the_rail_a_little_at_a_time() {
        let park = rail_park(10.0);
        let mut s = spawn(&park);
        // Coming down 0.12 m to the side of the rail, along it.
        place(&mut s, v3(0.12, 0.9, 3.0), Vec3::UP, v3(0.0, 0.0, 1.0));
        s.vel = v3(0.0, -2.0, 4.0);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        let target = GrindTarget {
            rail: 0,
            seg: 0,
            point: v3(0.0, 0.5, 4.0),
            dir: v3(0.0, 0.0, 1.0),
        };
        let t = s.tuning.grind;
        let mut total = 0.0;
        for _ in 0..8 {
            s.grind_air_assist(&park, target, DT);
            let a = s.air_assist.unwrap();
            assert!(a.moved <= t.max_offset_step + 1e-5, "{a:?}");
            assert!(a.offset.length() <= t.max_offset + 1e-5);
            total += a.moved;
            s.pos += s.vel * DT + a.offset * 0.0;
            s.vel.y -= 9.8 * DT;
        }
        let a = s.air_assist.unwrap();
        assert_eq!(a.kind, Some(4), "a 50-50: {a:?}");
        assert!(total > 0.05 && a.offset.x < 0.0, "towards the rail: {a:?}");
        // Out of reach (0.5 m to the side): no lock-on kind, nothing moved.
        let mut s = spawn(&park);
        place(&mut s, v3(0.5, 0.9, 3.0), Vec3::UP, v3(0.0, 0.0, 1.0));
        s.vel = v3(0.0, -2.0, 4.0);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        s.grind_air_assist(&park, target, DT);
        assert_eq!(s.air_assist.and_then(|a| a.kind), None);
    }

    #[test]
    fn an_unfinished_flip_does_not_lock_on() {
        let park = rail_park(10.0);
        let mut s = spawn(&park);
        s.speed = 5.0;
        s.tuning.jump_high = 1.0; // a low pop: still flipping over the rail
        s.tuning.jump_low = 1.0;
        let mut ev = steps(
            &mut s,
            &park,
            &Pad::default(),
            Some(Flick::of(Trick::Laserflip)),
            1,
        );
        for _ in 0..40 {
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            assert!(!s.grinding(), "{ev:?}");
        }
    }

    /// A quarter pipe built from triangles: flat ground towards -z, a 3 m curved transition up to a metre
    /// of vert at z = 3, coping at 4 m and a deck behind. Also a 50-degree face off to the side
    /// (x = 8..14) and a wall across the flat at z = -6 (x = -5..-3).
    fn quarter_pipe() -> crate::collision::TriMesh {
        let mut tris: Vec<([[f32; 3]; 3], u16, u16)> = Vec::new();
        let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            tris.push(([a, b, c], 0, 0));
            tris.push(([a, c, d], 0, 0));
        };
        let (x0, x1) = (-5.0f32, 5.0f32);
        // Flat.
        quad(
            [x0, 0.0, -30.0],
            [x1, 0.0, -30.0],
            [x1, 0.0, 0.0],
            [x0, 0.0, 0.0],
        );
        // Transition: 24 strips of a quarter circle (radius 3, centre z = 0, y = 3).
        let r = 3.0;
        let pt = |k: f32| {
            let th = k * PI / 2.0;
            (r * sinf(th), r - r * cosf(th))
        };
        for i in 0..24 {
            let (za, ya) = pt(i as f32 / 24.0);
            let (zb, yb) = pt((i + 1) as f32 / 24.0);
            quad([x0, ya, za], [x1, ya, za], [x1, yb, zb], [x0, yb, zb]);
        }
        // A metre of vert, then the deck.
        quad(
            [x0, 3.0, 3.0],
            [x1, 3.0, 3.0],
            [x1, 4.0, 3.0],
            [x0, 4.0, 3.0],
        );
        quad(
            [x0, 4.0, 3.0],
            [x1, 4.0, 3.0],
            [x1, 4.0, 6.0],
            [x0, 4.0, 6.0],
        );
        // A steep bank off to the side: rises 50 degrees along +x.
        quad(
            [8.0, 0.0, -10.0],
            [8.0, 0.0, -4.0],
            [11.0, 3.58, -4.0],
            [11.0, 3.58, -10.0],
        );
        quad(
            [5.0, 0.0, -10.0],
            [5.0, 0.0, -4.0],
            [8.0, 0.0, -4.0],
            [8.0, 0.0, -10.0],
        );
        // A wall across part of the flat.
        quad(
            [-5.0, 0.0, -6.0],
            [-3.0, 0.0, -6.0],
            [-3.0, 2.0, -6.0],
            [-5.0, 2.0, -6.0],
        );
        crate::collision::TriMesh::parse(
            &crate::collision::encode(&tris),
            (v3(0.0, 0.0, -10.0), 0.0),
        )
        .unwrap()
    }

    fn run(s: &mut Skater, park: &dyn Terrain, frames: usize) -> (Vec<Event>, f32) {
        let mut ev = Vec::new();
        let mut top: f32 = f32::MIN;
        for _ in 0..frames {
            s.step(park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
            top = top.max(s.pos.y);
        }
        (ev, top)
    }

    #[test]
    fn rolls_up_a_quarter_pipe_and_comes_back_fakie() {
        let park = quarter_pipe();
        let mut s = spawn(&park);
        s.speed = 6.0;
        let (ev, top) = run(&mut s, &park, 60 * 5);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        assert!(top > 1.2 && top < 3.9, "climbed {top}");
        assert!(s.rolling(), "{ev:?}");
        assert!(s.speed < -1.0 && s.fakie, "{} {}", s.speed, s.fakie);
        assert!(s.pos.z < 0.0, "back on the flat");
    }

    #[test]
    fn airs_out_over_the_coping_and_lands_back_in_fakie() {
        let park = quarter_pipe();
        let mut s = spawn(&park);
        s.speed = 10.0;
        let (ev, top) = run(&mut s, &park, 60 * 5);
        assert!(ev.contains(&Event::Launched), "{ev:?}");
        assert!(top > 4.3, "aired out: {top}");
        assert!(!ev.contains(&Event::Bailed), "{ev:?} {:?}", s.bail_reason);
        assert!(s.rolling());
        assert!(
            s.fakie && s.speed < -1.0,
            "rolled back in fakie: {}",
            s.speed
        );
    }

    #[test]
    fn a_head_on_wall_stops_the_board_as_sk8s() {
        // SK8 riding head-on into its platform's 1.5 m wall at 3.1, 4.7 and 6.7 m/s: the board stops
        // within two ticks of reaching it, upright and riding, and moves off it no faster than
        // 0.6 m/s after (0.3 m/s riding, 0.59 in a manual at 6 m/s).
        let park = quarter_pipe();
        for speed in [3.0, 4.5, 6.5] {
            let mut s = spawn(&park);
            s.pos = v3(-4.0, 0.0, -12.0);
            s.set_heading(0.0);
            s.speed = speed;
            let mut ev = Vec::new();
            let mut hit = None;
            for k in 0..240 {
                ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
                if hit.is_none() && s.speed < speed * 0.5 {
                    hit = Some(k);
                }
                if let Some(h) = hit {
                    if k >= h + 2 {
                        assert!(s.speed < 0.05, "{speed} m/s: still {} into it", s.speed);
                    }
                    assert!(s.speed > -0.6, "{speed} m/s: pushed off at {}", s.speed);
                }
            }
            assert!(
                hit.is_some(),
                "{speed} m/s never reached the wall: {:?}",
                s.pos
            );
            assert!(
                !ev.contains(&Event::Bailed) && !ev.contains(&Event::RanOut),
                "{speed} m/s: {ev:?}"
            );
            assert!(s.rolling());
        }
    }

    #[test]
    fn walls_keep_speed_along_them_and_hard_hits_take_you_off() {
        let park = quarter_pipe();
        let wall_run = |speed: f32, yaw: f32| {
            let mut s = spawn(&park);
            s.pos = v3(-4.0, 0.0, -12.0);
            s.set_heading(yaw);
            s.speed = speed;
            let (ev, _) = run(&mut s, &park, 120);
            (s, ev)
        };
        // Head on and slow: stopped and pushed off, no bail.
        let (_, ev) = wall_run(3.0, 0.0);
        assert!(
            !ev.contains(&Event::Bailed) && !ev.contains(&Event::RanOut),
            "{ev:?}"
        );
        // Upright single hits a bit over the limit: a runout, no respawn.
        let (s, ev) = wall_run(9.0, 0.0);
        assert!(ev.contains(&Event::RanOut), "{ev:?}");
        assert!(!ev.contains(&Event::Respawned));
        assert!(s.rolling(), "back on the board");
        // Far over it: a bail.
        let (_, ev) = wall_run(14.0, 0.0);
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
    }

    #[test]
    fn a_glancing_wall_hit_keeps_speed_and_turns_the_board_along_it() {
        let park = quarter_pipe();
        let mut s = spawn(&park);
        // 25 degrees off parallel into the wall across z = -6 (x = -5..-3).
        s.pos = v3(-0.5, 0.0, -7.5);
        s.set_heading(-65.0 * PI / 180.0);
        s.speed = 6.0;
        let mut ev = Vec::new();
        let mut hit = None;
        for _ in 0..60 {
            let before = s.ground_velocity().length();
            s.step(&park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
            if s.collide.is_some() && hit.is_none() {
                hit = Some((before, s.ground_velocity().length()));
            }
        }
        let (before, after) = hit.expect("hit the wall");
        assert!(
            !ev.contains(&Event::Bailed) && !ev.contains(&Event::RanOut),
            "{ev:?}"
        );
        // Only the part into the wall went: most of the speed is kept (the old model reversed it).
        assert!(after > before * 0.8, "{before} -> {after}");
        // The board ends up running along the wall (towards -x), not into it or back off it.
        assert!(s.fwd.x < -0.85, "{:?}", s.fwd);
    }

    #[test]
    fn a_board_tipped_away_from_a_face_turns_to_it_before_landing() {
        let park = quarter_pipe();
        let mut s = spawn(&park);
        // Above the face, the board tipped the other way: its up turns to the face before touchdown
        // (no faster than the per-tick limit, so given a moment of air).
        s.pos = v3(9.5, 3.6, -7.0);
        s.normal = rotate(Vec3::UP, v3(0.0, 0.0, 1.0), -50.0 * PI / 180.0);
        s.fwd = tangent(v3(0.0, 0.0, 1.0), s.normal);
        s.take_off(None);
        s.select_state();
        let mut ev = Vec::new();
        let mut tilt = PI;
        for _ in 0..90 {
            let n = s.plan.map(|p| p.normal);
            ev.extend(run(&mut s, &park, 1).0);
            if !s.is_airborne() {
                break;
            }
            if let Some(n) = n {
                let now = libm::acosf(s.normal.dot(n).clamp(-1.0, 1.0));
                let cap = s.tuning.air_up_max_delta.eval(air::COM_TO_DECK);
                assert!(tilt == PI || tilt - now <= cap + 1e-4, "{tilt} -> {now}");
                tilt = now;
            }
        }
        assert!(tilt < 0.25, "{tilt}");
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    }

    #[test]
    fn a_quarter_pipe_is_no_wallride_but_a_wall_with_a_floor_is() {
        let park = quarter_pipe();
        // On the vert, rolling along it: the floor under it curves up into it, so no wallride.
        let mut s = spawn(&park);
        place(
            &mut s,
            v3(-3.0, 3.4, 3.0),
            v3(0.0, 0.0, -1.0),
            v3(1.0, 0.0, 0.0),
        );
        s.speed = 6.0;
        let (ev, _) = run(&mut s, &park, 90);
        assert!(
            !ev.iter()
                .any(|e| matches!(e, Event::Landed { name, .. } if name == "WALLRIDE")),
            "{ev:?}"
        );
        // Flying into the free-standing wall a metre over the flat: it rides it and jumps off.
        let mut s = spawn(&park);
        s.pos = v3(-3.2, 1.3, -4.9);
        s.take_off(None);
        s.select_state();
        s.vel = v3(-3.0, 1.0, -3.0);
        let mut ev = Vec::new();
        let mut rode = false;
        for _ in 0..90 {
            s.step(&park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
            rode |= s.wallride.is_some();
        }
        assert!(rode, "{ev:?}");
        assert!(ev.contains(&Event::Launched), "jumped off: {ev:?}");
        assert!(!ev.contains(&Event::Bailed), "{ev:?} {:?}", s.bail_reason);
    }

    #[test]
    fn pumping_pays_on_a_curved_transition_not_on_a_flat_bank() {
        let run_pump = |park: &dyn Terrain, at: Vec3, n: Vec3, f: Vec3, pump: bool| {
            let mut s = spawn(park);
            place(&mut s, at, n, f);
            s.speed = 4.0;
            let mut ev = Vec::new();
            for i in 0..30 {
                let crouch = if pump && i < 12 { 1.0 } else { 0.0 };
                let fl = FlickOut {
                    crouch,
                    stick: [0.0, if crouch > 0.0 { 1.0 } else { 0.0 }],
                    ..FlickOut::default()
                };
                s.step(park, &Pad::default(), &fl, DT, &mut ev);
            }
            (s.speed, s.pos.y)
        };
        let qp = quarter_pipe();
        let (n, f) = (v3(0.0, 0.966, -0.259), v3(0.0, 0.259, 0.966));
        let (plain, _) = run_pump(&qp, v3(0.0, 0.11, 0.8), n, f, false);
        let (pumped, y) = run_pump(&qp, v3(0.0, 0.11, 0.8), n, f, true);
        assert!(y > 0.2, "on the transition: {y}");
        assert!(pumped > plain + 0.2, "{pumped} vs {plain}");
        // A flat bank doesn't curve: nothing to pump.
        let bank = bank(20.0);
        let (n, f) = bank_frame(20.0);
        let (plain, _) = run_pump(&bank, v3(0.0, 0.5, 1.37), n, f, false);
        let (pumped, _) = run_pump(&bank, v3(0.0, 0.5, 1.37), n, f, true);
        assert!((pumped - plain).abs() < 0.05, "{pumped} vs {plain}");
    }

    #[test]
    fn hardcore_pumping_over_a_crest_takes_speed() {
        // SK8 pumping/controller.rs `update`: rising while the ground turns away along the travel (a
        // crest) is `PumpEffectFactorAbsorption` x the pumping, at most `Hash_09D1AEE3D7D8A4A7` a
        // second; Skate 3's Hardcore has 9 and 8 m/s², Easy and Normal no absorption at all.
        let drop = libm::tanf(15.0 * PI / 180.0) * 20.0;
        let crest = mesh(&[
            [
                [-5.0, 0.0, -20.0],
                [5.0, 0.0, -20.0],
                [5.0, 0.0, 0.0],
                [-5.0, 0.0, 0.0],
            ],
            [
                [-5.0, 0.0, 0.0],
                [5.0, 0.0, 0.0],
                [5.0, -drop, 20.0],
                [-5.0, -drop, 20.0],
            ],
        ]);
        let ride = |d: Difficulty, absorb: Option<f32>| {
            let mut s = spawn(&crest);
            s.tuning.set_difficulty(d);
            if let Some(a) = absorb {
                s.tuning.pump_max_absorption = a;
            }
            place(&mut s, v3(0.0, 0.0, -1.0), Vec3::UP, v3(0.0, 0.0, 1.0));
            s.speed = 4.0;
            let mut ev = Vec::new();
            let mut taken = 0.0f32;
            for i in 0..40 {
                // Crouched up to the crest, standing up over it.
                let crouch = if i < 12 { 1.0 } else { 0.0 };
                let fl = FlickOut {
                    crouch,
                    ..FlickOut::default()
                };
                s.step(&crest, &Pad::default(), &fl, DT, &mut ev);
                taken = taken.min(s.pump.dv);
            }
            (s.speed, taken)
        };
        let (hard, taken) = ride(Difficulty::Hardcore, None);
        let (free, none) = ride(Difficulty::Hardcore, Some(0.0));
        assert!(taken < 0.0 && none == 0.0, "{taken} {none}");
        assert!(
            taken >= -8.0 * DT - 1e-6,
            "at most 8 m/s² a second: {taken}"
        );
        assert!(hard < free, "{hard} vs {free}");
        let (_, normal) = ride(Difficulty::Normal, None);
        assert_eq!(normal, 0.0, "Normal absorbs nothing");
    }

    /// Flat ground towards -z, then a plane rising at `deg` along +z from z = 0, 10 m wide.
    fn bank(deg: f32) -> crate::collision::TriMesh {
        let rise = libm::tanf(deg * PI / 180.0) * 20.0;
        mesh(&[
            [
                [-5.0, 0.0, -20.0],
                [5.0, 0.0, -20.0],
                [5.0, 0.0, 0.0],
                [-5.0, 0.0, 0.0],
            ],
            [
                [-5.0, 0.0, 0.0],
                [5.0, 0.0, 0.0],
                [5.0, rise, 20.0],
                [-5.0, rise, 20.0],
            ],
        ])
    }

    /// The up and uphill directions of `bank(deg)`'s slope.
    fn bank_frame(deg: f32) -> (Vec3, Vec3) {
        let a = deg * PI / 180.0;
        (v3(0.0, cosf(a), -sinf(a)), v3(0.0, sinf(a), cosf(a)))
    }

    /// A test park from quads (each two triangles).
    fn mesh(quads: &[[[f32; 3]; 4]]) -> crate::collision::TriMesh {
        let mut tris: Vec<([[f32; 3]; 3], u16, u16)> = Vec::new();
        for [a, b, c, d] in quads {
            tris.push(([*a, *b, *c], 0, 0));
            tris.push(([*a, *c, *d], 0, 0));
        }
        crate::collision::TriMesh::parse(
            &crate::collision::encode(&tris),
            (v3(0.0, 0.0, -10.0), 0.0),
        )
        .unwrap()
    }

    /// Flat ground at y = 0 out to `edge` across the way (+x), then a 45 degree bank down 3 m beside it,
    /// then flat again.
    fn drop_off(edge: f32) -> crate::collision::TriMesh {
        let (z0, z1) = (-20.0f32, 20.0f32);
        let fall = 3.0f32;
        let out = edge + fall;
        mesh(&[
            [
                [-10.0, 0.0, z0],
                [edge, 0.0, z0],
                [edge, 0.0, z1],
                [-10.0, 0.0, z1],
            ],
            [
                [edge, 0.0, z0],
                [out, -fall, z0],
                [out, -fall, z1],
                [edge, 0.0, z1],
            ],
            [
                [out, -fall, z0],
                [out + 20.0, -fall, z0],
                [out + 20.0, -fall, z1],
                [out, -fall, z1],
            ],
        ])
    }

    /// An ollie at 4 m/s along +z from the origin: where it lands, on what, and the plan it flew.
    fn ollie_over(park: &dyn Terrain) -> (Vec3, Vec3, Option<Plan>) {
        let mut s = spawn(park);
        place(&mut s, Vec3::ZERO, Vec3::UP, v3(0.0, 0.0, 1.0));
        s.speed = 4.0;
        s.vel = v3(0.0, 0.0, 4.0);
        let mut ev = Vec::new();
        s.pop(Flick::OLLIE, 1.0, [0.0, 0.0], &mut ev);
        let mut plan = None;
        for _ in 0..120 {
            plan = s.plan.or(plan);
            run(&mut s, park, 1);
            if !s.is_airborne() {
                break;
            }
        }
        assert!(s.rolling());
        (s.pos, s.contact_n, plan)
    }

    #[test]
    fn the_flight_assist_reaches_a_near_miss_bank_and_no_further() {
        // Over flat ground the launch velocity itself is the flight (the middle candidate wins).
        let (flat, n, plan) = ollie_over(&drop_off(100.0));
        assert!(n.y > 0.99 && plan.is_some_and(|p| p.index == 0), "{plan:?}");
        assert!(flat.x.abs() < 0.01);
        // A bank dropping away just beside the landing: the flight is nudged onto it.
        let (at, n, plan) = ollie_over(&drop_off(0.3));
        assert!(at.x > 0.3 && n.y < 0.95, "landed at {at:?} on {n:?}");
        assert!(plan.is_some_and(|p| p.index != 0), "{plan:?}");
        // Too far for the cone: it lands on the flat as it would have.
        let (at, n, _) = ollie_over(&drop_off(1.5));
        assert!(at.x.abs() < 0.01 && n.y > 0.99, "landed at {at:?} on {n:?}");
    }

    /// Puts a rolling board at `pos` on a surface with normal `n`, its nose along `f`.
    fn place(s: &mut Skater, pos: Vec3, n: Vec3, f: Vec3) {
        s.pos = pos;
        s.normal = n;
        s.contact_n = n;
        s.fwd = f;
        s.sync_heading();
    }

    #[test]
    fn a_crest_throws_the_board_into_the_air_at_speed_only() {
        // A flat deck a metre up, then a 30-degree slope down along +z.
        let drop = libm::tanf(30.0 * PI / 180.0) * 10.0;
        let park = mesh(&[
            [
                [-5.0, 1.0, -20.0],
                [5.0, 1.0, -20.0],
                [5.0, 1.0, 0.0],
                [-5.0, 1.0, 0.0],
            ],
            [
                [-5.0, 1.0, 0.0],
                [5.0, 1.0, 0.0],
                [5.0, 1.0 - drop, 10.0],
                [-5.0, 1.0 - drop, 10.0],
            ],
        ]);
        let over = |speed: f32| {
            let mut s = spawn(&park);
            place(&mut s, v3(0.0, 1.0, -2.0), Vec3::UP, v3(0.0, 0.0, 1.0));
            s.speed = speed;
            let mut ev = Vec::new();
            let mut lowest_gap: f32 = 0.0;
            for _ in 0..60 {
                s.step(&park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
                let g = park.ground(s.pos.x, s.pos.z, s.pos.y + 0.1).height;
                lowest_gap = lowest_gap.max(s.pos.y - g);
            }
            (ev, lowest_gap, s)
        };
        let (ev, gap, _) = over(8.0);
        assert!(ev.contains(&Event::Launched), "{ev:?}");
        assert!(gap > 0.3, "flew clear of the slope: {gap}");
        let (ev, gap, s) = over(1.5);
        assert!(!ev.contains(&Event::Launched), "{ev:?}");
        assert!(gap < 0.08, "{gap}");
        assert!(s.rolling());
    }

    #[test]
    fn legs_reaching_for_the_landing_never_push_the_board_through_the_ground() {
        // Falling slowly onto flat ground while the animation stands the rider up, 10 cm a tick:
        // the legs push the board down faster than it falls (a nollie's legs reaching for the
        // landing). Tested along the flight alone, the board started each test under the ground.
        // Imported parks' triangles, which (unlike the demo park's height field) aren't found from
        // underneath.
        let (a, b, c, d) = (
            [-20.0, 0.0, -20.0],
            [20.0, 0.0, -20.0],
            [20.0, 0.0, 20.0],
            [-20.0, 0.0, 20.0],
        );
        let park = crate::collision::TriMesh::parse(
            &crate::collision::encode(&[([a, d, c], 0, 0), ([a, c, b], 0, 0)]),
            (v3(0.0, 0.0, 0.0), 0.0),
        )
        .unwrap();
        let mut s = spawn(&park);
        place(&mut s, v3(0.0, 0.3, 0.0), Vec3::UP, v3(0.0, 0.0, 1.0));
        s.vel = v3(0.0, -1.0, 0.0);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.5;
        s.states.time = 0.5;
        s.air_com = Some(0.8);
        s.body_com = Some(0.8);
        let mut ev = Vec::new();
        let mut lowest = f32::MAX;
        for k in 1..=60 {
            s.graph = GraphControl {
                pose_com: Some(0.8 + 0.1 * k as f32),
                ..Default::default()
            };
            s.step(&park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
            lowest = lowest.min(s.pos.y);
            if !s.is_airborne() {
                break;
            }
        }
        assert!(s.rolling(), "{ev:?}, lowest {lowest}");
        assert!(lowest > -0.05, "went under the ground: {lowest}");
    }

    #[test]
    fn a_curb_is_hit_not_climbed() {
        // A 10 cm step across the flat.
        let park = mesh(&[
            [
                [-5.0, 0.0, -20.0],
                [5.0, 0.0, -20.0],
                [5.0, 0.0, 0.0],
                [-5.0, 0.0, 0.0],
            ],
            [
                [-5.0, 0.0, 0.0],
                [5.0, 0.0, 0.0],
                [5.0, 0.1, 0.0],
                [-5.0, 0.1, 0.0],
            ],
            [
                [-5.0, 0.1, 0.0],
                [5.0, 0.1, 0.0],
                [5.0, 0.1, 5.0],
                [-5.0, 0.1, 5.0],
            ],
        ]);
        let mut s = spawn(&park);
        place(&mut s, v3(0.0, 0.0, -2.0), Vec3::UP, v3(0.0, 0.0, 1.0));
        s.speed = 3.0;
        let (_, top) = run(&mut s, &park, 60);
        assert!(top < 0.05, "climbed the curb: {top}");
        assert!(s.pos.z < 0.0);
    }

    #[test]
    fn steering_curves_with_speed_and_slips_when_fast() {
        let park = flat();
        // Yaw rate for a full right turn, holding `speed` (after the tilt has eased in).
        let turn = |speed: f32| {
            let mut s = spawn(&park);
            // Not quite fully over (fully over and stopped is a kick turn).
            let pad = Pad {
                stick: [0.7, 0.0],
                ..Default::default()
            };
            let mut ev = Vec::new();
            let mut slip: f32 = 0.0;
            let mut h0 = 0.0;
            for i in 0..90 {
                s.speed = speed;
                if i == 30 {
                    h0 = s.heading;
                }
                s.step(&park, &pad, &FlickOut::default(), DT, &mut ev);
                slip = slip.max((s.slip / speed.max(0.1)).abs());
            }
            (wrap_angle(s.heading - h0) / 1.0, slip)
        };
        let (stopped, _) = turn(0.0);
        assert!(stopped.abs() < 1e-3, "no turning when stopped: {stopped}");
        let (slow, slow_slip) = turn(1.5);
        let (medium, _) = turn(3.0);
        // Turning right (towards -x) and about twice the yaw rate at twice the speed: the trucks' curve.
        assert!(slow < 0.0 && medium < 0.0);
        assert!(
            medium.abs() > slow.abs() * 1.6 && medium.abs() < slow.abs() * 2.4,
            "{slow} {medium}"
        );
        // Fast: grip runs out and the board drifts.
        let (_, fast_slip) = turn(9.0);
        // The rigid board's wheels slip a little even slowly (SK8: 2 degrees at 3 m/s full stick), and
        // far more fast.
        assert!(slow_slip < 0.03, "{slow_slip}");
        assert!(
            fast_slip > 0.03 && fast_slip > 2.0 * slow_slip,
            "{slow_slip} {fast_slip}"
        );
        // Half a stick turns far less than half of a full one (Skate 3's signed tilt).
        let t = Tuning::BUILT_IN;
        assert!(t.steer_tilt(0.5, 3.0, 0.0, 1.0) < 0.5 * t.steer_tilt(1.0, 3.0, 0.0, 1.0));
        // Rolling backwards the trucks turn the board the other way, but riding fakie Skate 3 negates
        // the turn (`FakieTurn`): the stick still turns the travel its way, so the board yaws as it does
        // rolling forwards.
        let mut s = spawn(&park);
        let pad = Pad {
            stick: [1.0, 0.0],
            ..Default::default()
        };
        for _ in 0..30 {
            s.speed = -2.0;
            steps(&mut s, &park, &pad, None, 1);
        }
        assert!(s.fakie, "rolling backwards at 2 m/s is fakie");
        assert!(s.heading < -0.05, "{}", s.heading);
    }

    #[test]
    fn slow_boards_swing_downhill_on_a_bank() {
        let park = bank(25.0);
        let (n, up) = bank_frame(25.0);
        let across = v3(1.0, 0.0, 0.0);
        let swing = |speed: f32| {
            let mut s = spawn(&park);
            place(
                &mut s,
                v3(0.0, 2.0 * libm::tanf(25.0 * PI / 180.0), 2.0),
                n,
                across,
            );
            s.speed = speed;
            let mut ev = Vec::new();
            for _ in 0..30 {
                s.step(&park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
            }
            // How far the nose has turned downhill (towards -z along the slope).
            -s.fwd.dot(up)
        };
        let slow = swing(1.5);
        let fast = swing(9.0);
        assert!(slow > 0.3, "swung down: {slow}");
        assert!(fast < slow * 0.5, "barely at speed: {fast} vs {slow}");
    }

    #[test]
    fn popping_on_vert_goes_up_not_out() {
        let park = quarter_pipe();
        let mut s = spawn(&park);
        // On the vert, rolling straight up it.
        place(&mut s, v3(0.0, 3.3, 3.0), v3(0.0, 0.0, -1.0), Vec3::UP);
        s.speed = 3.0;
        let mut ev = Vec::new();
        popped(&mut s, &park, &Pad::default(), Flick::OLLIE, &mut ev);
        assert!(s.is_airborne(), "{ev:?}");
        // Straight up (a little back in), not thrown out over the flat.
        assert!(s.vel.y > 2.5, "{:?}", s.vel);
        assert!(s.vel.z.abs() < 0.5, "{:?}", s.vel);
        // On flat ground the same pop goes straight up from the board.
        let mut s = spawn(&park);
        s.pos = v3(0.0, 0.0, -10.0);
        s.speed = 3.0;
        popped(&mut s, &park, &Pad::default(), Flick::OLLIE, &mut ev);
        assert!(s.vel.y > 2.0 && (s.vel.z - 3.0).abs() < 0.2, "{:?}", s.vel);
    }

    #[test]
    fn the_board_keeps_rolling_on_the_pops_tick() {
        let park = flat();
        let mut s = spawn(&park);
        s.pos = v3(0.0, 0.0, -10.0);
        s.speed = 4.0;
        let z = s.pos.z;
        let mut ev = Vec::new();
        popped(&mut s, &park, &Pad::default(), Flick::OLLIE, &mut ev);
        assert!(s.is_airborne(), "{ev:?}");
        // A tick's roll along the travel (the camera's anchor would read a stop otherwise).
        assert!(
            (s.pos.z - z - 4.0 * DT).abs() < 0.01,
            "{} -> {}",
            z,
            s.pos.z
        );
    }

    #[test]
    fn names() {
        assert_eq!(
            landed_name("", Some("KICKFLIP"), 0, false, None).as_deref(),
            Some("KICKFLIP")
        );
        assert_eq!(
            landed_name("FAKIE ", Some("KICKFLIP"), -2, true, None).as_deref(),
            Some("FAKIE FS 360 KICKFLIP")
        );
        assert_eq!(
            landed_name("", None, 1, false, None).as_deref(),
            Some("BS 180 AIR")
        );
        assert_eq!(landed_name("", None, 0, false, None), None);
    }

    #[test]
    fn graph_pushes_drive_only_once_the_foot_is_down() {
        let park = flat();
        let mut s = spawn(&park);
        s.graph = GraphControl::default();
        steps(&mut s, &park, &hold(btn::A), None, 30);
        assert!(s.speed.abs() < 0.05, "no contact yet: {}", s.speed);
        s.graph = GraphControl {
            push_contact: true,
            push_target: Some(3.0),
            ..GraphControl::default()
        };
        steps(&mut s, &park, &hold(btn::A), None, 1);
        s.graph = GraphControl::default();
        steps(&mut s, &park, &hold(btn::A), None, 30);
        assert!(s.speed > 1.0, "the push drove: {}", s.speed);
    }

    #[test]
    fn graph_flicks_pop_only_on_the_graphs_jump() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 3.0;
        s.graph = GraphControl::default();
        let flick = FlickOut {
            flick: Some(Flick::OLLIE),
            strength: 1.0,
            ..FlickOut::default()
        };
        let mut ev = Vec::new();
        for _ in 0..5 {
            s.step(&park, &Pad::default(), &flick, DT, &mut ev);
        }
        assert!(s.rolling(), "a flick alone doesn't pop");
        s.graph = GraphControl {
            pop: Some((Flick::of(crate::gesture::Trick::Kickflip), 1.0)),
            ..GraphControl::default()
        };
        let ev = steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(s.is_airborne());
        assert!(
            ev.iter().any(
                |e| matches!(e, Event::Popped(f) if f.trick == crate::gesture::Trick::Kickflip)
            )
        );
    }

    #[test]
    fn graph_manuals_hold_without_tipping_over_and_pitch_with_the_stick() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        s.graph = GraphControl {
            manual: Some((false, 1.0, false)),
            ..GraphControl::default()
        };
        steps(&mut s, &park, &Pad::default(), None, 600);
        let m = s.manual.expect("still manualing after 10 s");
        let full = m.pitch;
        assert!(full > 0.0);
        // The balance servo settles at the full-lean tilt, give or take its wobble, and stays there.
        let aim = s.tuning.manual_max_tilt * PI / 180.0;
        let wobble = s.tuning.manual_noise_scale * s.tuning.manual_noise_vs_speed.eval(4.0) + 0.01;
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for _ in 0..240 {
            steps(&mut s, &park, &Pad::default(), None, 1);
            let p = s.manual.unwrap().pitch;
            lo = lo.min(p);
            hi = hi.max(p);
        }
        assert!(
            lo > aim - wobble && hi < aim + wobble,
            "pitch {lo}..{hi}, aim {aim}"
        );
        s.graph = GraphControl {
            manual: Some((false, 0.0, false)),
            ..GraphControl::default()
        };
        steps(&mut s, &park, &Pad::default(), None, 60);
        assert!(
            s.manual.unwrap().pitch < full * 0.5,
            "a light lean pitches less"
        );
        s.graph = GraphControl::default();
        steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(s.manual.is_none(), "the graph ended it");
    }

    #[test]
    fn a_graph_manual_drag_against_the_travel_asks_out_only_without_the_corrective_force() {
        // Skate 3's balance is positive on the tail (SK8's `Balance` packet, negated: the stick pulled
        // back), so the drag opposes the travel on the nose rolling forwards or the tail rolling
        // backwards (SK8 manual/controller.rs `apply_correction`; SK8's tail drag at 6.6 m/s never
        // raises it). flatground used to test the tail rolling forwards.
        let park = flat();
        let drag = |nose: bool, speed: f32, corrective: bool, brake: bool| {
            let mut s = spawn(&park);
            s.tuning.manual_corrective = corrective;
            s.speed = speed;
            s.graph = GraphControl {
                manual: Some((nose, 1.0, brake)),
                ..GraphControl::default()
            };
            steps(&mut s, &park, &Pad::default(), None, 1);
            s.manual_exit
        };
        // Skate 3's Normal (corrective force on): dragging never asks the manual to end.
        assert!(!drag(true, 4.0, true, true));
        assert!(!drag(false, -4.0, true, true));
        // Easy (off): the nose dragged rolling forwards, the tail rolling backwards, ask out.
        assert!(drag(true, 4.0, false, true));
        assert!(drag(false, -4.0, false, true));
        // Rolling the manual's own way, stopped, or not dragging: no.
        assert!(!drag(false, 4.0, false, true));
        assert!(!drag(true, 0.0, false, true));
        assert!(!drag(true, 4.0, false, false));
        // Off the manual the flag clears.
        let mut s = spawn(&park);
        s.tuning.manual_corrective = false;
        s.speed = 4.0;
        s.graph = GraphControl {
            manual: Some((true, 1.0, true)),
            ..GraphControl::default()
        };
        steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(s.manual_exit);
        s.graph = GraphControl::default();
        steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(!s.manual_exit && s.manual.is_none());
    }

    /// The speed after each tick of a graph manual drag from `speed` (nose or tail), and whether the
    /// corrective drag acted.
    fn drag_run(nose: bool, speed: f32, ticks: usize) -> Vec<(f32, bool)> {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = speed;
        s.graph = GraphControl {
            manual: Some((nose, 1.0, true)),
            ..GraphControl::default()
        };
        (0..ticks)
            .map(|_| {
                steps(&mut s, &park, &Pad::default(), None, 1);
                (s.speed, s.manual.is_some_and(|m| m.correcting))
            })
            .collect()
    }

    #[test]
    fn a_nose_drag_rolling_forward_fast_takes_the_corrective_drag_as_sk8s() {
        // SK8 (Normal, nose drag from 8.5 m/s): the nose brake slows the board until its deck is within
        // a degree of the brake angle; that tick, at 5.43 m/s, the corrective drag (-400 x the
        // velocity at the truck, in place of the brake) leaves 2.49 m/s. The rigid deck takes SK8's
        // time to reach it (about 1.1 s).
        let run = drag_run(true, 8.5, 240);
        let k = run
            .iter()
            .position(|r| r.1)
            .expect("the corrective drag acts");
        let (before, after) = (run[k - 1].0, run[k].0);
        assert!(before > 5.0, "only over the mode's 5 m/s: {before}");
        assert!(
            (before - 5.43).abs() < 0.4,
            "reached the brake angle at {before} m/s (SK8 5.43)"
        );
        let keep = after / before;
        // (SK8 reads its deck's speed, this the whole board's: within 0.02.)
        assert!((keep - 2.4937 / 5.4305).abs() < 0.02, "kept {keep}");
        // Under 5 m/s the brake drags again.
        assert!(run[k + 1..].iter().all(|r| !r.1));
        assert!(run[k + 1].0 < after && run[k + 1].0 > after - 0.2);
        // The tail dragged rolling forwards (SK8's manual brake) never takes it.
        assert!(drag_run(false, 8.5, 240).iter().all(|r| !r.1));
    }

    #[test]
    fn a_deck_pitched_from_the_riders_up_compresses_the_crouch() {
        // SK8 ground_runtime/pumping.rs: the physical deck pitched further from the rider's up than
        // the posed board reports `CompressionVsDeckAngle` of the difference (quarter turns) to the
        // crouch, and `CompressionDeckScalar` x that as the ground's compression when it outweighs it.
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        steps(&mut s, &park, &Pad::default(), None, 30);
        // (The physical deck, which the angle is measured on, settles within a hair of level.)
        assert!(
            s.pump.deck_absorption < 1e-3,
            "level with the rider's up: {}",
            s.pump.deck_absorption
        );
        // The rider's up leaning back 0.3 rad from the deck's (as over a transition's start).
        let right = s.reckoning.ground_right();
        s.reckoning.up = rotate(Vec3::UP, right, 0.3);
        steps(&mut s, &park, &Pad::default(), None, 1);
        let t = &s.tuning;
        let want = t.compression_vs_deck_angle.eval(0.3 * 2.0 / PI);
        assert!(
            (s.pump.deck_absorption - want).abs() < 0.02,
            "{} against {want}",
            s.pump.deck_absorption
        );
        assert!((s.pump.ground_absorption - t.compression_deck_scale * want).abs() < 0.2);
    }

    #[test]
    fn a_manual_lifted_off_the_ground_bleeds_its_correction() {
        // SK8 manual/controller.rs: the manual's wheels off the ground, the correction bleeds by
        // `TorqueBleedOffWithNoContact` (0.96) a tick and turns the deck by `TorqueScalarWithNoContact`
        // (0) of it.
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        s.graph = GraphControl {
            manual: Some((false, 1.0, false)),
            ..GraphControl::default()
        };
        steps(&mut s, &park, &Pad::default(), None, 10);
        let m = s.manual.unwrap();
        assert!(m.correction > 0.0);
        s.vn = 0.5;
        steps(&mut s, &park, &Pad::default(), None, 1);
        let lifted = s.manual.unwrap();
        assert!((lifted.correction - m.correction * 0.96).abs() < 1e-6);
        assert_eq!(
            lifted.last_step, 0.0,
            "the controller turns the deck no more"
        );
    }

    #[test]
    fn riding_fast_wobbles_the_steering_and_slowing_down_stops_it() {
        let park = flat();
        let mut s = spawn(&park);
        let onset = wobble::SpeedWobble::onset(&s.tuning.wobble, s.riding_com(), 0.7);
        s.speed = onset + 4.0;
        // (The tilt swings through zero: its largest over the run, not one tick's.)
        let mut most = 0.0f32;
        for _ in 0..30 {
            steps(&mut s, &park, &Pad::default(), None, 1);
            most = most.max(s.deck_tilt.abs());
        }
        assert!(
            s.wobble.active && s.wobble.amplitude > 0.0,
            "{:?}",
            s.wobble
        );
        assert!(most > 1e-4, "the wobble tilts the deck straight ahead");
        s.speed = onset - 2.0;
        steps(&mut s, &park, &Pad::default(), None, 2);
        assert!(!s.wobble.active && s.wobble.amplitude == 0.0);
        // In the air it is reset.
        s.speed = onset + 4.0;
        steps(&mut s, &park, &Pad::default(), None, 10);
        assert!(s.wobble.active);
        // Launched clear of the ground (the rigid board's contact point is on it to a micrometre).
        s.pos.y += 0.5;
        s.take_off(None);
        s.select_state();
        steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(!s.wobble.active);
    }

    #[test]
    fn graph_turns_the_rider_round_out_of_fakie() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = -2.0;
        s.fakie = true;
        let was = s.switch;
        s.graph = GraphControl {
            turn_round: true,
            ..GraphControl::default()
        };
        let ev = steps(&mut s, &park, &Pad::default(), None, 1);
        assert_ne!(s.switch, was);
        assert!(!s.fakie && s.speed > 0.0);
        assert!(ev.contains(&Event::Switched));
    }

    /// Pops with the graph and steps until the board is back down, with `air` the graph's view of the
    /// air (its trick, the danger zone, dark) on every tick.
    fn graph_air(air: impl Fn(u32) -> GraphControl) -> (Skater, Vec<Event>) {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 3.0;
        s.graph = GraphControl {
            pop: Some((Flick::OLLIE, 1.0)),
            ..GraphControl::default()
        };
        let mut ev = steps(&mut s, &park, &Pad::default(), None, 1);
        let mut k = 0;
        while s.is_airborne() && k < 300 {
            s.graph = air(k);
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            k += 1;
        }
        (s, ev)
    }

    /// A long flight off flat ground with the graphs asking for a body flip; `grab(k)`: the grab held
    /// on air tick `k`.
    fn body_flip_air(front: bool, grab: impl Fn(u32) -> bool) -> (Skater, Vec<Event>, f32) {
        body_flip_air_in(Difficulty::Normal, front, grab)
    }

    /// [`body_flip_air`] riding difficulty `d`.
    fn body_flip_air_in(
        d: Difficulty,
        front: bool,
        grab: impl Fn(u32) -> bool,
    ) -> (Skater, Vec<Event>, f32) {
        let park = flat();
        let mut s = spawn(&park);
        s.tuning.set_difficulty(d);
        s.speed = 3.0;
        s.pos.y = 0.05;
        s.vel = v3(0.0, 6.5, 3.0);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.0;
        s.states.time = 0.0;
        let mut ev = Vec::new();
        let mut k = 0;
        let mut most = 0.0f32;
        while s.is_airborne() && k < 300 {
            s.graph = GraphControl {
                body_flip: (k < 20).then_some(front),
                grabbing: grab(k),
                ..GraphControl::default()
            };
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            if let Some(f) = &s.body_flip {
                most = most.max(f.angle.abs());
            }
            k += 1;
        }
        (s, ev, most)
    }

    #[test]
    fn a_held_back_flip_goes_round_and_lands() {
        let (s, ev, most) = body_flip_air(false, |_| true);
        assert!(most > 5.5, "turned {most} rad");
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        assert!(s.rolling());
        assert!(landed(&ev).iter().any(|n| n.contains("BACKFLIP")), "{ev:?}");
        // The graphs see it (IsBodyFlipping) only while it turns.
        assert!(s.body_flip.is_none());
    }

    #[test]
    fn letting_go_of_a_flip_upside_down_bails() {
        // The grab let go a third of the way round: it slows and the rider falls upside down.
        let (_, ev, most) = body_flip_air(true, |k| k < 25);
        assert!(most > 1.5 && most < 5.0, "{most}");
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
        // Without a flip asked for, a grab alone doesn't flip.
        let park = flat();
        let mut s = spawn(&park);
        s.vel = v3(0.0, 6.5, 3.0);
        s.pos.y = 0.05;
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.0;
        s.states.time = 0.0;
        s.graph = GraphControl {
            grabbing: true,
            ..GraphControl::default()
        };
        steps(&mut s, &park, &Pad::default(), None, 30);
        assert!(s.body_flip.is_none());
    }

    #[test]
    fn easy_body_flips_go_exactly_once_round_without_the_grab_and_never_bail_upside_down() {
        // Skate 3's Easy: `PerfectBodyFlips` (SK8 known/body.rs: the flip's speed whatever the grab;
        // body_flip.rs: slowed to end exactly one turn round) and no upside-down falling wipeout
        // (`Hash_2B913C786BEFF4CD` off). The same grab let go a third of the way round bails on Normal.
        let (s, ev, most) = body_flip_air_in(Difficulty::Easy, true, |k| k < 25);
        assert!(most > 5.9 && most <= crate::math::TAU, "turned {most} rad");
        assert!(!ev.contains(&Event::Bailed), "{ev:?} {:?}", s.bail_reason);
        assert!(
            landed(&ev).iter().any(|n| n.contains("FRONTFLIP")),
            "{ev:?}"
        );
        // Held all the way it still stops at one turn (Normal's carries on past it).
        let (_, _, most) = body_flip_air_in(Difficulty::Easy, false, |_| true);
        assert!(most <= crate::math::TAU + 1e-4, "turned {most} rad");
        // The upside-down switch alone: Normal's let-go flip with it off doesn't fall upside down.
        let park = flat();
        let mut s = spawn(&park);
        s.tuning.moves.upside_down_bail = false;
        s.speed = 3.0;
        s.pos.y = 0.05;
        s.vel = v3(0.0, 6.5, 3.0);
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = 0.0;
        s.states.time = 0.0;
        for k in 0..300 {
            if !s.is_airborne() {
                break;
            }
            s.graph = GraphControl {
                body_flip: (k < 20).then_some(true),
                grabbing: k < 25,
                ..GraphControl::default()
            };
            steps(&mut s, &park, &Pad::default(), None, 1);
        }
        assert_ne!(s.bail_reason, Some("fell upside down"));
        // Normal and Hardcore check it and turn the flip only as asked.
        let mut t = Tuning::BUILT_IN;
        t.set_difficulty(Difficulty::Easy);
        assert!(!t.moves.upside_down_bail && t.moves.perfect_body_flips);
        t.set_difficulty(Difficulty::Hardcore);
        assert!(t.moves.upside_down_bail && !t.moves.perfect_body_flips);
    }

    #[test]
    fn the_bad_landing_check_follows_the_physics_mode() {
        // Skate 3's `WipeoutCheckForBadLanding` and `Hash_D978550D6DB4E6F7` (SK8 wipeout/air.rs
        // `bad_landing`): the landing angle and impact limits times the mode's scale, 1.5 on Easy.
        let t = Tuning::BUILT_IN;
        let land = |d: Difficulty, deg: f32, speed: f32, down: f32, check: bool| {
            let park = flat();
            let mut s = spawn(&park);
            s.tuning.set_difficulty(d);
            s.tuning.bad_landing = check;
            s.take_off(Some(Flick::OLLIE));
            s.select_state();
            let a = deg * PI / 180.0;
            s.fwd = v3(sinf(a), 0.0, cosf(a));
            s.vel = v3(0.0, -down, speed);
            let mut ev = Vec::new();
            s.states.time = 0.6;
            s.land(Vec3::UP, 0, 0.6, &mut ev);
            s.select_state();
            ev.contains(&Event::Bailed)
        };
        let hard = t.land_max_impact + 1.0;
        assert!(land(Difficulty::Normal, 0.0, 4.0, hard, true));
        assert!(
            !land(Difficulty::Easy, 0.0, 4.0, hard, true),
            "1.5 x the limit"
        );
        assert!(land(
            Difficulty::Easy,
            0.0,
            4.0,
            t.land_max_impact * 1.5 + 0.5,
            true
        ));
        // Too crooked for the speed on Normal, within 1.5 x the angle on Easy.
        let fast = 25.0;
        let limit = t.land_angle.eval(fast) * 180.0 / PI;
        assert!(land(Difficulty::Normal, limit + 5.0, fast, 2.0, true));
        assert!(!land(Difficulty::Easy, limit + 5.0, fast, 2.0, true));
        // With the check off neither bails.
        assert!(!land(Difficulty::Normal, 0.0, 4.0, hard, false));
        assert!(!land(Difficulty::Normal, limit + 5.0, fast, 2.0, false));
    }

    #[test]
    fn a_trick_out_of_a_manual_jumps_from_the_modes_manual_low_end() {
        // Skate 3's `Hash_BE3F74F978D777E5` (SK8 ground_jump: the clip's `TrickFromManual`): on Easy
        // the lowest jump out of a manual is 1.53 m, not the 1.71 m of every other jump there.
        let pop = |d: Difficulty, from_manual: bool| {
            let park = flat();
            let mut s = spawn(&park);
            s.tuning.set_difficulty(d);
            s.speed = 3.0;
            s.graph = GraphControl {
                pop: Some((Flick::OLLIE, 0.0)),
                pop_from_manual: from_manual,
                ..GraphControl::default()
            };
            steps(&mut s, &park, &Pad::default(), None, 1);
            assert!(s.is_airborne());
            s.vel.y
        };
        assert!(pop(Difficulty::Easy, true) < pop(Difficulty::Easy, false) - 0.1);
        // Normal's two low ends are the same (1.33 m).
        assert!((pop(Difficulty::Normal, true) - pop(Difficulty::Normal, false)).abs() < 1e-4);
        let mut t = Tuning::BUILT_IN;
        t.set_difficulty(Difficulty::Easy);
        assert_eq!((t.jump_low_manual, t.jump_low), (1.53, 1.71));
        assert!(t.jump_height_from(1.53, 0.0, 3.0) < t.jump_height(0.0, 3.0));
    }

    #[test]
    fn a_danger_zone_landing_bails_on_the_closing_speed_across_the_rider_too() {
        // SK8 wipeout/air.rs `check` and common.rs `closing`: in the danger zone the speed into the
        // surface, seen in the rider's frame, bails past `Wipeout_AirYTrick` (0.6 m/s) along his up or
        // `Wipeout_AirXZTrick` (0.1) across it. flatground used to check only the speed into the ground.
        let park = flat();
        let touch = |tilt: f32, into: f32, danger: bool| {
            let mut s = spawn(&park);
            s.force_state(crate::skater::State::PhysicsAir);
            s.air_time = 0.5;
            s.states.time = 0.5;
            s.graph = GraphControl {
                danger,
                ..GraphControl::default()
            };
            // The rider comes down leaning `tilt` across his travel.
            s.normal = rotate(Vec3::UP, v3(0.0, 0.0, 1.0), tilt);
            s.vel = v3(0.0, -into, 3.0);
            let mut ev = Vec::new();
            s.states.time = 0.6;
            s.land(Vec3::UP, 0, 0.5, &mut ev);
            s.select_state();
            s.bailed()
        };
        assert!(!touch(0.0, 0.5, true), "upright, 0.5 m/s into the ground");
        assert!(touch(0.0, 0.7, true), "0.7 m/s along his up");
        // 20 degrees off: 0.17 m/s across him.
        assert!(touch(0.35, 0.5, true));
        // 6 degrees: 0.05 across.
        assert!(!touch(0.1, 0.5, true));
        // Caught: no closing check.
        assert!(!touch(0.35, 0.5, false));
    }

    #[test]
    fn graph_landings_bail_only_while_the_clips_say_the_board_is_not_caught() {
        let trick = |late| AirTrick {
            flick: Flick::of(crate::gesture::Trick::Kickflip),
            flips: 1,
            progress: 0.5,
            late,
            id: 3,
        };
        // Still in the danger zone at touchdown: a bail.
        let (s, _) = graph_air(|_| GraphControl {
            air_trick: Some(trick(false)),
            danger: true,
            ..GraphControl::default()
        });
        assert!(s.bailed(), "{:?}", s.state());
        // The same trick caught in time: a clean landing, however long `trick_duration` says.
        let (s, ev) = graph_air(|k| GraphControl {
            air_trick: Some(trick(false)),
            danger: k < 10,
            ..GraphControl::default()
        });
        assert!(s.rolling());
        assert!(ev.iter().any(|e| matches!(e, Event::Landed { .. })));
    }

    #[test]
    fn graph_late_flips_become_the_trick_and_dark_catches_ride_dark() {
        let (s, ev) = graph_air(|k| GraphControl {
            air_trick: (k > 5).then_some(AirTrick {
                flick: Flick::of(crate::gesture::Trick::Kickflip),
                flips: 2,
                progress: 0.5,
                late: true,
                id: 9,
            }),
            dark: k > 5,
            ..GraphControl::default()
        });
        assert!(s.rolling(), "{ev:?}");
        let name = ev
            .iter()
            .find_map(|e| match e {
                Event::Landed { name, .. } => Some(name.clone()),
                _ => None,
            })
            .unwrap();
        assert!(name.contains("LATE DOUBLE KICKFLIP"), "{name}");
        assert!(name.contains("DARK"), "{name}");
        assert!(s.dark, "riding dark while the graph says so");
    }
}
