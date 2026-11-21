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

mod air;
mod board;
pub mod flip;
mod grind;
mod landing;
mod respawn;

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

use crate::flick::FlickOut;
use crate::gesture::{self, Flick, Group, Trick};
use crate::input::{Pad, btn};
use crate::math::{PI, Vec3, atan2f, clamp, cosf, sinf, smooth, sqrtf, v3, wrap_angle, yaw_dir};
#[cfg(test)]
use crate::park::Park;
use crate::park::Terrain;
use crate::tuning::Tuning;

/// Highest ledge a bailed rider walks up (m).
pub const STEP: f32 = 0.12;
/// Below this speed a hard sideways stick kick-turns on the spot.
const KICKTURN_SPEED: f32 = 1.0;
/// Kick turns (ours, measured against SK8's simulation of Skate 3's `KickTurnSteering` at its stock
/// spin scale 0.5): the yaw rate the turn builds to against the board's speed (rad/s: 3.2 standing,
/// 2.7 at 1.8 m/s, 2.0 at 3.4 m/s), the share of the gap it closes a tick, and what is left of it a tick
/// after the stick is let go.
const KICKTURN_RATES: [(f32, f32); 4] = [(0.0, 3.2), (1.8, 2.72), (3.4, 2.0), (5.0, 1.6)];
const KICKTURN_BUILD: f32 = 0.09;
const KICKTURN_RELEASE: f32 = 0.7;
const KICKTURN_SPIN_SCALE: f32 = 0.5;
const KICKTURN_STICK: f32 = 0.75;
/// Rolling tail first faster than this is fakie at once; above `FAKIE_SLOW` after `FAKIE_TIME`; never
/// within `FAKIE_GRACE` of a spawn (Skate 3's riding-fakie rules).
const FAKIE_FAST: f32 = 1.0;
const FAKIE_SLOW: f32 = 0.5;
const FAKIE_TIME: f32 = 0.2;
const FAKIE_GRACE: f32 = 1.0;
/// Turning round from fakie to push switch takes this long (the push waits for it).
const SWITCH_TURN_TIME: f32 = 0.15;
/// A push press is remembered this long while the button is held (Skate 3's new-push window).
const PUSH_MEMORY: f32 = 0.3;
/// How fast the board follows the animated centre of mass in the air: the share of the gap closed a
/// tick (SK8's rider body is physical and follows its animation; fit to its ollies' COM over the deck).
const AIR_LEGS_FOLLOW: f32 = 0.15;
/// Rolled off an edge without a pop, the board meets rails as if still riding for this long (s, ours:
/// SK8's board pivots on its back wheels over a coping for about half a second at walking pace).
const ROLL_OFF_TIME: f32 = 0.3;
/// The wheels' friction on a landing's impact: sideways speed taken off per m/s into the ground.
const LAND_SIDE_FRICTION: f32 = 0.41;
/// The posed board lifted this far (m) means the pop's tail strike has started.
const POP_LIFT_MIN: f32 = 0.002;
/// A repeated push puts the foot down this long after the re-push point (s, ours).
const PUSH_REPEAT_DOWN: f32 = 0.1;
/// Powerslides: the board angle eases at this rate (rad/s, ours); a held slide relaxes after this long
/// (s).
const SLIDE_TURN_RATE: f32 = 8.0;
const SLIDE_RELAX: f32 = 0.5;
/// Stepping off the board after a runout and back on.
const RUNOUT_TIME: f32 = 1.2;
/// How long a landing is remembered (drawn: the landing clip), and how long after it a revert counts
/// (flatground's own rules; with the state graphs the revert gesture decides).
const LANDING_SHOW: f32 = 1.5;
const REVERT_WINDOW: f32 = 0.4;
const REVERT_POINTS: u32 = 100;
/// A spin landing keeps pivoting the board this long (s, ours), fading as a wall hit's turn does.
const SPIN_PIVOT_TIME: f32 = 0.15;
/// Without the state graphs: landing while still grabbing with the C-stick tweaking the grab this far
/// (0..1) wipes out (ours; the graphs have their own rule).
const GRAB_TWEAK_BAIL: f32 = 0.45;
/// The C-stick past this picks a grab's variant (Skate 3's dead zone for `ScoringGrabs`).
const GRAB_VARIANT_DEAD: f32 = 0.5;
/// Manuals: C-stick deflection that holds one (beyond is the tail or nose brake), how long it must be
/// held before the manual starts (so a quick flick doesn't start one), the balance sweet spot.
const MANUAL_MIN: f32 = 0.1;
const MANUAL_BRAKE: f32 = 0.9;
const MANUAL_ENGAGE: f32 = 0.12;
const MANUAL_SWEET: f32 = 0.35;
/// The C-stick deflection a manual balances at (for the log).
pub const MANUAL_BALANCED_AT: f32 = MANUAL_MIN + MANUAL_SWEET * (MANUAL_BRAKE - MANUAL_MIN);
const MANUAL_MIN_TIME: f32 = 0.5;
const MANUAL_POINTS_PER_S: f32 = 120.0;
/// Analog trigger past this holds a grab: any pressure, as Skate 3 and the state graphs' grab intents
/// (`graph::input`) count it; the pad's trigger dead zone (`input::TRIGGER_DEAD_ZONE`) already took out a
/// trigger at rest, so the physics and the graphs agree on when the rider grabs.
const GRAB_TRIGGER: f32 = 0.0;
const GRAB_POINTS: u32 = 200;
/// Points per whole body flip turn (ours).
const BODY_FLIP_POINTS: u32 = 600;
const GRAB_POINTS_PER_S: f32 = 150.0;
/// Both push buttons pressed within this of each other wind up a hippie jump.
const HIPPY_CHORD: f32 = 0.08;
const HIPPY_POINTS: u32 = 150;
/// Most flips a held kickflip or heelflip can reach (Skate 3: four).
const MAX_FLIPS: u8 = 4;

/// Which foot the rider naturally has forward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stance {
    /// Right foot forward: Skate 3's default.
    #[default]
    Goofy,
    Regular,
}

/// How long the built-in board's flick trick animation takes (s). Without the state graphs this is the
/// one clock the drawn board and the landing check share (landing before it ends is landing with the
/// board not caught); with them the clips' own `DANGERZONE` decides.
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Rolling,
    Air {
        time: f32,
    },
    Bailed {
        time: f32,
    },
    /// Stepped off the board after hitting something (Skate 3's runout): back on it where it stopped.
    Runout {
        time: f32,
    },
    /// On a rail (details in `Skater::grind`).
    Grind,
}

impl Mode {
    /// Off the board: a bail or a runout (drawn the same for now), and seconds since.
    pub fn off_board(&self) -> Option<f32> {
        match *self {
            Mode::Bailed { time } | Mode::Runout { time } => Some(time),
            _ => None,
        }
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

    pub fn points(&self) -> u32 {
        self.flick.points() + (self.flips.saturating_sub(1)) as u32 * 250
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

/// A push in progress: the foot comes down (`down` s; the first push of a run winds up with the `into`
/// clip), stays on the ground (`contact` s) while the board chases `target`, then swings back
/// (`recover` s). The next push may start `out` of the way through the swing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Push {
    /// Seconds since this push started (it waits for a switch turn first).
    pub t: f32,
    /// How long the button has been held during this push.
    pub held: f32,
    /// The mongo push set (front foot).
    pub mongo: bool,
    /// Speed when the foot came down.
    pub base: f32,
    /// Pushes in a row (0 = the first, which starts from riding).
    pub count: u32,
    /// The button doing the pushing.
    pub button: u32,
    /// Seconds left of the turn from fakie to switch before the foot goes down.
    pub turning: f32,
    /// How long that turn took (the push's own clock starts after it).
    pub delay: f32,
    pub down: f32,
    pub contact: f32,
    pub recover: f32,
    /// Speed the board chases while the foot is down (set when it touches).
    pub target: Option<f32>,
    /// Re-push point as a share of the swing back.
    pub out: f32,
    /// A later press waiting for the re-push point.
    pub queued: Option<u32>,
    /// Started by holding the button on from the last push (its strength is capped).
    pub held_repeat: bool,
}

impl Push {
    /// Seconds into the push itself (after any switch turn).
    pub fn time(&self) -> f32 {
        (self.t - self.delay).max(0.0)
    }

    /// Which part of the push it is (0 foot coming down, 1 on the ground, 2 swinging back) and the
    /// seconds into that part.
    pub fn stage(&self) -> (usize, f32) {
        let t = self.time();
        if t < self.down {
            (0, t)
        } else if t < self.down + self.contact {
            (1, t - self.down)
        } else {
            (2, t - self.down - self.contact)
        }
    }

    /// The pushing foot is on the ground.
    pub fn in_contact(&self) -> bool {
        self.turning <= 0.0 && self.stage().0 == 1
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
    Landed {
        name: String,
        points: u32,
    },
    Bailed,
    /// Stepped off the board after a hit (no respawn).
    RanOut,
    Respawned,
    /// Tipped out of a manual (no points, a little speed lost).
    ManualLost,
    /// Turned round from fakie to push switch.
    Switched,
}

/// Stance and how the rider took off (for names), the surface left and whether the pop was re-aimed up
/// a vert face (for the landing prediction).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Takeoff {
    fakie: bool,
    switch: bool,
    hippy: bool,
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
    pub mode: Mode,
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
    pub push: Option<Push>,
    /// The last push press, while it is remembered (button, seconds since).
    push_press: Option<(u32, f32)>,
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
    /// 0..1, how far the skater is crouched (drawn).
    pub crouch: f32,
    /// The crouch is for a nollie (stick held forward).
    pub crouch_nose: bool,
    pub landing: Option<Landing>,
    pub manual: Option<Manual>,
    /// How long the C-stick has been in the manual zone.
    manual_engage: f32,
    /// After tipping out of a manual, the stick has to come back before another starts.
    manual_lock: bool,
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
    /// What Skate 3's state graphs decided this tick, when they run (`crate::graph`); `None` rides on
    /// flatground's own rules.
    pub graph: Option<GraphControl>,
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
    air_com: Option<f32>,
    /// How far the legs have lifted the board over the flight's path (m, world up), and how fast it
    /// moved last tick (m/s).
    pub air_lift: f32,
    air_lift_rate: f32,
}

/// The state graphs' decisions for the physics (see `graph::drive::Driver::control`).
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
    /// The grab's scored name (`ScoringGrabs`: the variant the C-stick picks), and the tweak's size
    /// (`TweakX`, `TweakY`).
    pub grab_name: Option<&'static str>,
    pub tweak: f32,
    /// The stick's spin as the graph hands it to the physics (`PhysBodySpin`; Skate 3's body spin
    /// controller reads it on the ground and in the air).
    pub body_spin: f32,
    /// The playing clips carry `Anticipating` (the ollie wind-up): Skate 3's camera reads it as
    /// `IsPreparingToJump`.
    pub anticipating: bool,
    /// A body flip the graphs ask for (`FrontFlip`: true, `BackFlip`: false) and a grab held
    /// (`Grabbing`, `WantsLeftAirGrab`, `WantsRightAirGrab`).
    pub body_flip: Option<bool>,
    pub grabbing: bool,
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
            mode: Mode::Rolling,
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
            push: None,
            push_press: None,
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
            crouch_nose: false,
            landing: None,
            manual: None,
            manual_engage: 0.0,
            manual_lock: false,
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
            graph: None,
            body_flip: None,
            flipped: None,
            graph_push: None,
            body_com: None,
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

    /// Seconds until the board lands: the known landing's, else straight down onto the ground below.
    pub fn time_to_land(&self, park: &dyn Terrain) -> f32 {
        if let (Mode::Air { time }, Some(p)) = (self.mode, &self.plan) {
            return p.remaining(time).max(0.0);
        }
        let h = park
            .ground(self.pos.x, self.pos.z, self.pos.y + STEP)
            .height;
        let g = self.tuning.gravity.max(0.1);
        let vy = self.vel.y;
        (vy + sqrtf((vy * vy + 2.0 * g * (self.pos.y - h).max(0.0)).max(0.0))) / g
    }

    pub fn is_airborne(&self) -> bool {
        matches!(self.mode, Mode::Air { .. })
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
        let f = match (self.mode, self.chord) {
            (Mode::Rolling, Some(ch)) if ch.dot(self.fwd) > 0.9 => ch,
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

    /// Sets the air spin to `spin` (rad), turning the board and rider about the board's up to match.
    pub fn set_spin(&mut self, spin: f32) {
        let d = spin - self.spin;
        self.fwd = rotate(self.fwd, self.normal, d);
        self.spin = spin;
        self.sync_heading();
    }

    /// The stick's spin input: the state graph's `PhysBodySpin` when it runs, else the main stick's x.
    fn spin_input(&self, pad: &Pad) -> f32 {
        self.graph.map_or(pad.stick[0], |g| g.body_spin)
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

    /// The drawn rider is the mirror image of the clips (they are authored goofy).
    pub fn mirrored(&self) -> bool {
        (self.stance == Stance::Regular) != self.switch
    }

    /// Direction of travel (yaw): the board's heading, turned round when rolling tail first.
    pub fn travel_yaw(&self) -> f32 {
        match self.mode {
            Mode::Air { .. } | Mode::Bailed { .. } if self.vel.horizontal().length() > 0.5 => {
                let h = self.vel.horizontal();
                atan2f(h.x, h.z)
            }
            _ => {
                let f = if self.speed < 0.0 {
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
        let touched = pad.held != 0
            || pad.stick != [0.0, 0.0]
            || flick.stick != [0.0, 0.0]
            || pad.trig_l > 0.0
            || pad.trig_r > 0.0;
        self.idle = if touched { 0.0 } else { self.idle + dt };
        self.since_spawn += dt;
        self.dark = self.graph.is_some_and(|g| g.dark);
        let rolling = self.mode == Mode::Rolling;
        let crouch = if rolling && self.manual.is_none() {
            flick
                .crouch
                .max(if self.hippy.is_some() { 1.0 } else { 0.0 })
        } else {
            0.0
        };
        self.crouch += (crouch - self.crouch) * smooth(18.0, dt);
        self.grind_pitch_input = pad.flick[1];
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
        if self.mode != Mode::Grind {
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
            self.body.ground(self.spin_input(pad));
            self.air_com = None;
            self.air_lift = 0.0;
        }
        // The rider's body over the board: the animation's while riding; following it only so fast
        // from the pop's tail strike (SK8's GroundAnimation) until the jump's top (measured: SK8's
        // COM over the deck lags its animation there, and follows it again coming down).
        self.body_com = match (self.graph.and_then(|g| g.pose_com), self.body_com) {
            (Some(c), Some(d))
                if (self.is_airborne() && self.vel.y > 0.0)
                    || self.graph.is_some_and(|g| g.pose_lift > POP_LIFT_MIN) =>
            {
                Some(d + (c - d) * (AIR_LEGS_FOLLOW * dt * 60.0).min(1.0))
            }
            (c, _) => c,
        };
        if self.graph.is_some_and(|g| g.wipeout)
            && matches!(self.mode, Mode::Rolling | Mode::Air { .. })
        {
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
        self.step_mode(park, pad, flick, dt, events);
        if self.mode == Mode::Rolling {
            // The drawn board follows the ground smoothly; the velocity is the ground's.
            let k = 1.0
                - libm::powf(
                    1.0 - self.tuning.ground_normal_blend.clamp(0.0, 1.0),
                    dt * 60.0,
                );
            self.normal = self
                .normal
                .lerp(self.contact_n, k)
                .normalize_or(self.contact_n);
            self.vel = self.ground_velocity() + self.contact_n * self.vn;
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
        }
        self.sync_heading();
    }

    fn step_mode(
        &mut self,
        park: &dyn Terrain,
        pad: &Pad,
        flick: &FlickOut,
        dt: f32,
        events: &mut Vec<Event>,
    ) {
        match self.mode {
            Mode::Rolling => self.roll(park, pad, flick, dt, events),
            Mode::Air { time } => self.fly(park, pad, flick, time + dt, dt, events),
            Mode::Grind => self.slide_rail(park, pad, flick, dt, events),
            Mode::Bailed { time } | Mode::Runout { time } => {
                let time = time + dt;
                let h = self.vel.horizontal() * (1.0 - smooth(3.0, dt));
                self.vel = Vec3 { y: 0.0, ..h };
                self.try_move(park, self.vel * dt);
                self.pos.y = park
                    .ground(self.pos.x, self.pos.z, self.pos.y + STEP)
                    .height;
                let reset = matches!(self.mode, Mode::Bailed { .. }) && {
                    // Skate 3's push buttons (A, B) put the rider back early once settled.
                    let press = pad.was_pressed(btn::A) || pad.was_pressed(btn::B);
                    let speed = self.vel.length();
                    let t = self.tuning;
                    self.recovery.update(&t, time, speed, press, dt)
                };
                match self.mode {
                    Mode::Bailed { .. } if reset => {
                        self.respawn(park);
                        events.push(Event::Respawned);
                    }
                    Mode::Runout { .. } if time >= RUNOUT_TIME => {
                        // Back on the board where it stopped.
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
                        self.mode = Mode::Rolling;
                    }
                    Mode::Bailed { .. } => self.mode = Mode::Bailed { time },
                    _ => self.mode = Mode::Runout { time },
                }
            }
        }
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
        let tail_brake = self.update_manual(flick, dt, events);
        let st = steering(pad.stick);
        self.update_fakie(dt);
        if self.graph.is_some_and(|g| g.turn_round) && (self.fakie || self.speed < 0.0) {
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
        } else if let Some(fs) = self.graph.and_then(|g| g.revert) {
            self.revert = Some(Revert {
                side: if fs { 1.0 } else { -1.0 },
                time: 0.0,
                turned: 0.0,
                rate: 0.0,
            });
            self.slide = None;
            self.landing = None;
        } else if self.graph.is_none()
            && st.slide != 0.0
            && self.landing.is_some_and(|l| l.time < REVERT_WINDOW)
            && self.manual.is_none()
            && self.speed.abs() > 1.0
        {
            self.revert = Some(Revert {
                side: st.slide,
                time: 0.0,
                turned: 0.0,
                rate: 0.0,
            });
            self.landing = None;
        }

        self.update_slide(&st, dt);

        // Kick turn: slow, stick hard to the side.
        // With the state graphs, its KickTurn state (speed and stick gates) turns the board.
        let graph_kick = self.graph.and_then(|g| g.kickturn);
        let kick = match (self.graph, graph_kick) {
            (Some(_), Some(_)) => pad.stick[0] != 0.0 && self.manual.is_none(),
            (Some(_), None) => false,
            (None, _) => {
                self.speed.abs() < KICKTURN_SPEED
                    && pad.stick[0].abs() > KICKTURN_STICK
                    && self.manual.is_none()
                    && self.slide.is_none()
                    && self.revert.is_none()
                    && self.push.is_none_or(|p| p.stage().0 == 2)
            }
        };
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
        if self.mode != Mode::Rolling {
            return;
        }
        // With the state graphs, a push only drives while the push clip's foot is down, and the foot
        // brake only once the brake clip's foot touches.
        let push_target = match self.graph {
            Some(g) if g.push_posed => {
                // Skate 3 (SK8 contact_events.rs, push.rs): while the push clip's foot is down the
                // board chases the posed trajectory's speed, within the push's change a tick.
                self.graph_push = None;
                g.push_target
                    .filter(|_| g.push_contact)
                    .map(|v| v.min(self.tuning.push_max_speed))
            }
            Some(g) => self.graph_push(g.push_contact, g.push_target, dt),
            None => self.update_push(pad, dt, events),
        };
        let foot_brake = match self.graph {
            Some(g) => {
                self.update_brake(pad, dt);
                self.brake.is_some() && g.brake_contact
            }
            None => self.update_brake(pad, dt),
        };
        let brake = t.brake_scale
            * t.braking_scalar
            * if tail_brake {
                t.tail_brake
            } else if foot_brake {
                t.foot_brake
            } else {
                0.0
            };

        // The trick to pop: the state graph's (its takeoff clip reached the jump), else the flick.
        let pop = match self.graph {
            Some(g) => g.pop,
            None => flick.flick.map(|f| {
                (
                    gesture::select(f, self.flick_group(), self.mirrored()),
                    flick.strength,
                )
            }),
        };
        if let Some((f, strength)) = pop {
            if let Some(w) = self.wallride.take() {
                // Popping off a wall: away from it as well as up.
                if w.time > 0.0 {
                    events.push(Event::Landed {
                        name: "WALLRIDE".into(),
                        points: 250 + (w.time * 200.0) as u32,
                    });
                }
                let n = self.contact_n;
                self.vel = self.ground_velocity() + n * 2.5;
                self.pos += n * 0.05;
                self.take_off(Some(f));
                self.vel.y += self.pop_speed(self.tuning.jump_height(strength, 0.0)) * 0.6;
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
            pushing: match self.graph {
                Some(g) => g.pushing,
                None => push_target.is_some(),
            },
            brake,
            manual: self.manual.is_some(),
            slide: self
                .slide
                .as_ref()
                .filter(|s| !s.releasing)
                .map(|s| s.angle),
            slide_time: self.slide.as_ref().map_or(0.0, |s| s.time),
            pivot: kick || self.revert.is_some(),
            keep_velocity: self.revert.is_some(),
            intentional_pump: flick.crouch > 0.0 || self.crouch > 0.05,
        };
        self.ride(park, &drive, dt, events);
        // Riding onto a rail: into coping, off a ledge's top, up a bank into a ledge.
        if self.mode == Mode::Rolling {
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
            self.revert = None;
            self.fakie = self.speed < 0.0;
            self.fakie_timer = 0.0;
            events.push(Event::Landed {
                name: "REVERT".into(),
                points: REVERT_POINTS,
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
                self.push = None;
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

    /// Pushing: returns the speed the board chases while the pushing foot is on the ground.
    fn update_push(&mut self, pad: &Pad, dt: f32, events: &mut Vec<Event>) -> Option<f32> {
        let t = self.tuning;
        // A press is remembered for a moment while the button is held.
        let pressed = [btn::A, btn::B].into_iter().find(|b| pad.was_pressed(*b));
        if let Some(b) = pressed {
            self.push_press = Some((b, 0.0));
        } else if let Some((b, s)) = &mut self.push_press {
            *s += dt;
            if *s > PUSH_MEMORY || !pad.is_held(*b) {
                self.push_press = None;
            }
        }
        let both = pad.is_held(btn::A) && pad.is_held(btn::B);
        let steep =
            libm::acosf(self.contact_n.y.clamp(-1.0, 1.0)) > t.push_slope_limit * PI / 180.0;
        let blocked = self.manual.is_some()
            || self.slide.is_some()
            || self.revert.is_some()
            || self.hippy.is_some()
            || self.wallride.is_some()
            || both
            || steep
            || self.brake.is_some_and(|b| b.released.is_none());
        if blocked {
            self.push = None;
            return None;
        }
        let Some(mut p) = self.push else {
            if let Some((b, _)) = self.push_press.take() {
                self.start_push(b, 0, events);
            }
            return None;
        };
        p.t += dt;
        if p.turning > 0.0 {
            p.turning -= dt;
            self.push = Some(p);
            return None;
        }
        let held = pad.is_held(p.button);
        if let Some(b) = pressed {
            p.queued = Some(b);
        }
        let (stage, _) = p.stage();
        if stage == 0 {
            p.base = self.speed;
            if held {
                p.held += dt;
            }
        }
        let mut target = None;
        if stage == 1 {
            if held {
                p.held += dt;
            }
            // How hard: from how long the button was held (a held repeat is capped).
            let mut dv = t.push_gain(p.held, p.base.max(0.0));
            if p.held_repeat {
                dv = dv.min(t.push_hold_cap);
            }
            let (_, _, _, gentle_end, strong_end, slow_b, fast_b) = self.clips.blend(p.base);
            let lo = gentle_end.min(strong_end);
            let end = (dv + p.base.max(0.0)).clamp(lo, strong_end.max(lo));
            let end = end.min(t.push_max_speed);
            let tgt = p.target.map_or(end, |old| old.max(end));
            p.target = Some(tgt);
            target = Some(tgt);
            // Where in the swing back the next push may start (Skate 3's out factor).
            let dv0 = t.push_dv.eval(0.0);
            let dv1 = t.push_dv.eval(1.0);
            let first = slow_b + dv0;
            let speed_k = if fast_b - first > 1e-3 {
                ((p.base.max(0.0) - first) / (fast_b - first)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let strength_k = if dv1 - dv0 > 1e-3 {
                ((dv - dv0) / (dv1 - dv0)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let w = t.push_out_speed_weight.clamp(0.0, 1.0);
            p.out = (strength_k * (1.0 - w) + speed_k * w)
                .max(0.0)
                .min(t.push_out_max);
        }
        let pt = p.time();
        let repush = p.down + p.contact + p.out * p.recover;
        let end = p.down + p.contact + p.recover;
        let next = if held { Some(p.button) } else { p.queued };
        if stage == 2 && pt >= repush && next.is_some() {
            let count = p.count + 1;
            let by_hold = held && p.queued.is_none();
            self.push = None;
            self.start_push(next.unwrap_or(btn::A), count, events);
            if let Some(q) = &mut self.push {
                q.held_repeat = by_hold;
            }
        } else if pt >= end {
            self.push = None;
        } else {
            self.push = Some(p);
        }
        target
    }

    fn start_push(&mut self, button: u32, count: u32, events: &mut Vec<Event>) {
        let mut turning = 0.0;
        if self.fakie || self.speed < -FAKIE_SLOW {
            // Skate 3 turns the rider round to push switch: same travel, the other foot forward.
            self.flip_board();
            self.switch = !self.switch;
            self.fakie = false;
            self.fakie_timer = 0.0;
            turning = SWITCH_TURN_TIME;
            events.push(Event::Switched);
        }
        // Skate 3 pushes with the mongo set when riding switch (for a rider who pushes regular).
        let mongo = (button == btn::B) != self.switch;
        let (into, contact, recover, ..) = self.clips.blend(self.speed.max(0.0));
        self.push_press = None;
        self.push = Some(Push {
            mongo,
            base: self.speed,
            count,
            button,
            turning,
            delay: turning,
            down: if count == 0 { into } else { PUSH_REPEAT_DOWN },
            contact,
            recover,
            out: self.tuning.push_out_max,
            ..Push::default()
        });
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
                self.push = None;
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
                self.push = None;
            }
            None => {}
        }
        false
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
                events.push(Event::Launched);
            }
            None if both
                && self.since_a.max(self.since_b) <= HIPPY_CHORD
                && self.manual.is_none()
                && self.slide.is_none() =>
            {
                self.hippy = Some(0.0);
                self.push = None;
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

    /// Manuals from the C-stick: held lightly back (towards the player) or forward, for a moment. The
    /// stick past the manual zone drags the tail (or nose) to brake: returns whether it is.
    fn update_manual(&mut self, flick: &FlickOut, dt: f32, events: &mut Vec<Event>) -> bool {
        if let Some(g) = self.graph {
            return self.graph_manual(g.manual, dt, events);
        }
        let [x, y] = flick.stick;
        let len = sqrtf(x * x + y * y);
        // Mostly up or down only.
        let vertical = y.abs() > 1.5 * x.abs();
        if len < MANUAL_MIN {
            self.manual_lock = false;
        }
        let Some(m) = &mut self.manual else {
            let zone = vertical && (MANUAL_MIN..MANUAL_BRAKE).contains(&len);
            self.manual_engage = if zone { self.manual_engage + dt } else { 0.0 };
            if self.manual_engage >= MANUAL_ENGAGE
                && !self.manual_lock
                && self.speed.abs() > 0.5
                && self.slide.is_none()
                && self.revert.is_none()
            {
                self.manual = Some(Manual {
                    // Stick away from the player: the front wheels.
                    nose: y < 0.0,
                    ..Manual::default()
                });
                self.push = None;
                self.brake = None;
            }
            return false;
        };
        // How hard the stick leans towards the manual's wheels.
        let toward = if m.nose { -y } else { y };
        if toward < MANUAL_MIN || !vertical {
            self.end_manual(events);
            return false;
        }
        m.time += dt;
        // Skate 3's tail brake: full force at once in the brake zone, the deck tipped to its angle.
        m.brake = if len > MANUAL_BRAKE { 1.0 } else { 0.0 };
        let pitch = m.brake * self.tuning.manual_brake_tilt * PI / 180.0;
        m.pitch = approach(m.pitch, pitch, 3.0 * dt);
        let push = ((toward - MANUAL_MIN) / (MANUAL_BRAKE - MANUAL_MIN)).min(1.0);
        // Unstable like a real manual: any tilt grows, the stick past its sweet spot tips you further
        // over, and a steady wobble keeps you correcting.
        m.balance +=
            (m.balance * 1.6 + (push - MANUAL_SWEET) * 2.0 + 0.5 * sinf(m.time * 2.3)) * dt;
        let braking = m.brake > 0.0;
        if m.balance.abs() >= 1.0 {
            self.manual = None;
            self.manual_lock = true;
            self.speed *= 0.85;
            events.push(Event::ManualLost);
            return false;
        }
        braking
    }

    /// A manual the state graph holds (engaged and left as the graph says; it never tips over): the deck
    /// pitch servoes to a target that grows with the stick's lean (SK8's manual controller aims at
    /// (0.25 + 0.75 lean) of the most tilt), or to the brake angle while dragging.
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
            self.push = None;
            self.brake = None;
        }
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
        // PID on the deck pitch, per 1/60 s tick (the deck turns by the accumulated correction).
        let ticks = (dt * 60.0).max(0.0);
        let error = (target - m.pitch).clamp(-t.manual_max_error, t.manual_max_error);
        m.filtered_error = 0.95 * m.filtered_error + 0.05 * error;
        let last_turn = if started { m.correction } else { 0.0 };
        let derivative = (last_turn * t.manual_d).clamp(-t.manual_d_limit, t.manual_d_limit);
        m.correction += derivative + m.filtered_error * t.manual_i + error * t.manual_p;
        m.pitch += m.correction * ticks;
        m.balance = 0.0;
        brake
    }

    /// Ends a manual on purpose (stick released, or popped out of): scores it if it was long enough.
    fn end_manual(&mut self, events: &mut Vec<Event>) {
        self.manual_engage = 0.0;
        if let Some(m) = self.manual.take()
            && m.time >= MANUAL_MIN_TIME
        {
            events.push(Event::Landed {
                name: (if m.nose { "NOSE MANUAL" } else { "MANUAL" }).into(),
                points: (m.time * MANUAL_POINTS_PER_S) as u32,
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
        let h = self.tuning.jump_height(strength, self.speed);
        // Skate 3 pops the centre of mass from where the animation has it at the jump (SK8
        // ground_jump `animation_com_position`): over the deck, which the tail strike has already
        // pitched up off its riding height. Without the imported rider, from `AbsoluteMinHeight`.
        let posed = self
            .graph
            .and_then(|g| Some((self.body_com.or(g.pose_com)?, g.pose_lift)));
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
        self.takeoff.on_vert = on_vert;
        self.takeoff.lean = stick[1];
        events.push(Event::Popped(f));
    }

    fn take_off(&mut self, f: Option<Flick>) {
        let pose = self.approach_pose();
        self.approach.take_off(pose);
        self.air_assist = None;
        self.mode = Mode::Air { time: 0.0 };
        // Rolling off a ledge mid-manual just ends it (no points: it wasn't finished).
        self.manual = None;
        self.manual_engage = 0.0;
        self.landing = None;
        self.grab = None;
        self.grabbed = None;
        self.push = None;
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
            ground: Some(self.contact_n),
            on_vert: false,
            lean: 0.0,
        };
        self.trick = f.map(ActiveTrick::new);
        self.air_com = self.body_com;
        self.air_lift = 0.0;
        self.air_lift_rate = 0.0;
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

    fn fly(
        &mut self,
        park: &dyn Terrain,
        pad: &Pad,
        flick: &FlickOut,
        time: f32,
        dt: f32,
        events: &mut Vec<Event>,
    ) {
        let t = self.tuning;
        let now = time - dt;
        self.update_prediction(park, now);

        // Body spin (Skate 3's controller): the stick, its wind-up, a let-go spin carrying on; with a
        // known landing and the stick let go, the assist turns the body towards the landing direction
        // (or the other way round when that is nearer); without one the stick's spin goes through the
        // assist's path.
        let stick = self.spin_input(pad);
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
        // The spin turns the body about its own up.
        let turn = spin * dt;
        self.spin_rate = spin;
        self.spin += turn;
        self.fwd = rotate(self.fwd, self.normal, turn);
        // The legs: the board follows the animated centre of mass's height over it, as fast as the
        // rider's body follows its animation (SK8's physical rider; measured, 0.15 of the gap a tick).
        if let (Some(c0), Some(d)) = (self.air_com, self.body_com) {
            self.air_lift_rate = (c0 - d - self.air_lift) / dt.max(1e-4);
            self.air_lift = c0 - d;
        }
        if let Some(g) = self.graph {
            self.follow_graph_trick(&g);
        } else if let Some(tr) = &mut self.trick {
            tr.t += dt;
            // Holding a kickflip or heelflip on the stick keeps it flipping.
            let flip_only = matches!(tr.flick.trick, Trick::Kickflip | Trick::Heelflip);
            let one = trick_duration(tr.flick.trick);
            if flip_only && flick.holding_flip && tr.flips < MAX_FLIPS && tr.t > tr.duration - 0.1 {
                tr.flips += 1;
                tr.duration += one;
            }
        }
        self.update_grab(pad, flick, dt);

        // A flight steered onto a rail: the grind air assist moves and turns the board towards it.
        if let Some(target) = self.plan.as_ref().and_then(|p| p.grind) {
            self.grind_air_assist(park, target, dt);
        }
        // Rolled off an edge without a pop, the board is still tipping over it on its back wheels for a
        // moment (SK8's rigid board stays on the ground pivoting): a rail it meets is ridden onto, not
        // landed on from the air (a slow roll off a deck drops in rather than stopping on the coping).
        let category = if self.trick.is_none() && !self.takeoff.hippy && now < ROLL_OFF_TIME {
            Category::Ground
        } else {
            Category::Air
        };
        // Fly: the known landing's flight (its start error blended away), else ballistically.
        match &self.plan {
            Some(p) => {
                let assist = self.air_assist.map_or(Vec3::ZERO, |a| a.offset);
                let next = p.target(time, t.traj_error_blend) + assist;
                self.vel = (next - self.pos) * (1.0 / dt.max(1e-4));
            }
            None => self.vel.y -= t.gravity * dt,
        }
        let from = self.pos;
        let delta = self.vel * dt;
        let len = delta.length();
        let mut moved = false;
        // The legs reaching for the landing hold the board under the flight's path: it touches down
        // when the board does (SK8: the deck under the flying centre of mass).
        let sink = Vec3::UP * self.air_lift.min(0.0);
        if len > 1e-6 {
            let dir = delta * (1.0 / len);
            if let Some(h) = park.raycast(from + sink, dir, len + 0.01)
                && self.vel.dot(h.normal) < 0.0
            {
                let landable = self.normal.dot(h.normal) > 0.5 || h.normal.y > 0.6;
                if landable {
                    self.pos = h.point;
                    if self.try_grind(park, category, events) {
                        return;
                    }
                    self.land(h.normal, h.surface, time, events);
                    return;
                }
                // A wall: a wallride if there is a floor well below, else off it (or off the board).
                if h.normal.y.abs() < 0.5 && self.try_air_wallride(park, &h, events) {
                    return;
                }
                self.pos = h.point + h.normal * 0.02;
                // Off the predicted flight: ask again.
                self.plan = None;
                self.selector = air::Selector::default();
                match self.hit_wall(h.normal, self.vel, true, events) {
                    Some(v) => self.vel = v,
                    None => return,
                }
                moved = true;
            }
        }
        if !moved {
            self.pos = from + delta;
        }

        if self.try_grind(park, category, events) {
            return;
        }
        if self.pos.y < park.kill_height() {
            // Fell out of the world: start over.
            self.respawn(park);
            events.push(Event::Respawned);
        } else {
            self.mode = Mode::Air { time };
        }
    }

    /// The side the rider faces (the toes): for a goofy rider riding normally the board's left
    /// (up x nose), the other way when mirrored.
    pub fn toe_side(&self) -> Vec3 {
        let side = self.normal.cross(self.fwd).normalize_or(v3(1.0, 0.0, 0.0));
        if self.mirrored() { -side } else { side }
    }

    /// The body flip's tick (SK8 KnownAir `CalculateBodyFlipSpeed`, Reckoning `UpdateBodyFlip`,
    /// `CheckUpsideDownFallingWipeout`): starts with a flip asked for and a grab held once the known
    /// flight is long enough, turns while the grab is held (or early on), slows let go. Returns whether
    /// the rider is falling upside down without the grab (a bail).
    fn update_body_flip(&mut self, now: f32, dt: f32) -> bool {
        let m = self.tuning.moves;
        let g = self.graph.unwrap_or_default();
        let grabbing = g.grabbing || self.grab.is_some();
        if self.body_flip.is_none()
            && let (Some(front), true, Some(p)) = (g.body_flip, grabbing, &self.plan)
            && p.contact.time > m.flip_start_time.eval(p.normal.y)
        {
            self.body_flip = Some(flip::BodyFlip::begin(self.normal, self.toe_side(), front));
        }
        let Some(mut f) = self.body_flip else {
            return false;
        };
        let target = self.plan.as_ref().map_or(f.speed, |p| {
            flip::BodyFlip::target_speed(&m, p.contact.time)
        });
        let requested = if grabbing || now < m.flip_min_grab_time {
            target
        } else {
            0.0
        };
        f.update(&m, requested, dt);
        self.body_flip = Some(f);
        let landing_up = self.plan.as_ref().map_or(Vec3::UP, |p| p.normal);
        !grabbing && flip::falling_upside_down(&m, f.apply(self.normal), landing_up)
    }

    /// The name and points of the jump so far (without spin and grab), for grinds and landings.
    fn trick_label(&self) -> Option<(String, u32)> {
        if let Some(tr) = &self.trick {
            return Some((tr.name(), tr.points()));
        }
        self.takeoff
            .hippy
            .then(|| (String::from("HIPPIE JUMP"), HIPPY_POINTS))
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
        if self.mode == Mode::Grind || self.grind.is_some() {
            self.grind_exit = Some("wiped out");
            self.grind = None;
        }
        if !self.is_airborne() && self.mode != Mode::Grind && self.vel == Vec3::ZERO {
            self.vel = self.surface_forward() * self.speed;
        }
        self.wallride = None;
        self.mode = Mode::Bailed { time: 0.0 };
        self.trick = None;
        self.grab = None;
        self.grabbed = None;
        self.manual = None;
        self.landing = None;
        self.push = None;
        self.brake = None;
        self.slide = None;
        self.revert = None;
        self.kickturn = None;
        self.kick_rate = 0.0;
        self.hippy = None;
        self.collide = None;
        self.plan = None;
        self.recovery = Recovery::default();
        events.push(Event::Bailed);
    }

    /// Stepped off the board after a hit the rider could run out of: no bail, back on the board where it
    /// stops (Skate 3's runout).
    fn run_out(&mut self, reason: &'static str, events: &mut Vec<Event>) {
        self.bail_reason = Some(reason);
        self.wallride = None;
        self.mode = Mode::Runout { time: 0.0 };
        self.trick = None;
        self.grab = None;
        self.grabbed = None;
        self.manual = None;
        self.landing = None;
        self.push = None;
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
        if let Some((name, points)) = self.trick_label() {
            events.push(Event::Landed {
                name: format!("{}{name}", self.stance_prefix()),
                points,
            });
        }
        self.mode = Mode::Rolling;
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
    fn update_grab(&mut self, pad: &Pad, flick: &FlickOut, dt: f32) {
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
        // The C-stick in gesture space has y towards the player: up is -y.
        let [cx, cy] = flick.stick;
        let (x, y) = if self.mirrored() {
            (-cx, -cy)
        } else {
            (cx, -cy)
        };
        let graph = self.graph;
        let name_of = |kind: GrabKind| match graph.and_then(|g| g.grab_name) {
            Some(n) => n,
            None => {
                let (base, variants) = kind.names();
                select_grab(base, variants, x, y)
            }
        };
        let tweak = match graph {
            Some(g) => g.tweak,
            None => sqrtf(cx * cx + cy * cy).min(1.0),
        };
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
        // The board isn't caught yet: the clips' `DANGERZONE` with the state graphs, else the built-in
        // board's trick animation.
        let danger = match self.graph {
            Some(g) => g.danger,
            None => self.trick.is_some_and(|t| t.progress() < 1.0),
        };
        let stairs = crate::park::surface_category(surface) == crate::park::STAIRS;
        let max_impact = if stairs {
            t.land_max_stairs
        } else {
            t.land_max_impact
        };
        let limit = t.land_angle.eval(speed);
        let tweaked = self.graph.is_none() && self.grab.is_some_and(|g| g.tweak >= GRAB_TWEAK_BAIL);
        let reason =
            if danger && air_time * 60.0 > t.danger_ignore_ticks && impact > t.danger_max_impact {
                Some("landed before the board was caught")
            } else if crooked > limit {
                Some("landed too crooked for the speed")
            } else if impact > max_impact {
                Some(if stairs {
                    "hit the stairs too hard"
                } else {
                    "hit the ground too hard"
                })
            } else if tweaked {
                Some("landed still tweaking a grab")
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
        let along = vt * t.landing_speed_vs_ny.eval(n.y);
        self.contact_n = n;
        self.fwd = axis;
        self.set_ground_velocity(along);
        // Skate 3's rigid board comes down on its wheels: their friction against the touchdown's
        // impact takes the sideways speed off a crooked landing (measured on SK8: about 0.4 of the
        // speed into the ground, a 70-degree landing at 6 m/s keeping 0.75 m/s).
        let into = (-(self.vel + Vec3::UP * self.air_lift_rate).dot(n)).max(0.0);
        let scrub = (LAND_SIDE_FRICTION * into).min(self.slip.abs());
        self.slip -= scrub * self.slip.signum();
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
        let mut label = self.trick_label();
        if let Some((front, turns)) = self.flipped.take() {
            let flip = match (front, turns) {
                (true, 1) => String::from("FRONTFLIP"),
                (false, 1) => String::from("BACKFLIP"),
                (true, n) => format!("{n}X FRONTFLIP"),
                (false, n) => format!("{n}X BACKFLIP"),
            };
            let (name, points) =
                label.map_or((flip.clone(), 0), |(l, p)| (format!("{flip} {l}"), p));
            label = Some((name, points + BODY_FLIP_POINTS * turns));
        }
        let fs = (halves as f32) * self.frontside_sign(self.takeoff.fakie) > 0.0;
        if let Some(name) = landed_name(
            self.stance_prefix(),
            label.as_ref().map(|l| l.0.as_str()),
            halves,
            fs,
            grab.map(|g| g.name),
        ) {
            let base = label.map_or(0, |l| l.1);
            let spin_bonus = halves.unsigned_abs() * 150;
            let grab_bonus = grab.map_or(0, |g| GRAB_POINTS + (g.time * GRAB_POINTS_PER_S) as u32);
            let air_bonus = 1.0 + air_time.min(2.0);
            let mut points = ((base + spin_bonus + grab_bonus) as f32 * air_bonus) as u32;
            let name = if matches!(kind, LandKind::Sketchy { .. }) {
                points /= 2;
                format!("SKETCHY {name}")
            } else {
                name
            };
            events.push(Event::Landed { name, points });
        }
        self.mode = Mode::Rolling;
        self.trick = None;
        self.spin = 0.0;
        self.spin_rate = 0.0;
        self.plan = None;
        self.takeoff.hippy = false;
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

    fn flicked(f: Flick) -> FlickOut {
        FlickOut {
            flick: Some(f),
            strength: 1.0,
            ..FlickOut::default()
        }
    }

    fn steps(
        s: &mut Skater,
        park: &dyn Terrain,
        pad: &Pad,
        flick: Option<Flick>,
        n: usize,
    ) -> Vec<Event> {
        let mut ev = Vec::new();
        for i in 0..n {
            let f = match flick {
                Some(f) if i == 0 => flicked(f),
                _ => FlickOut::default(),
            };
            s.step(park, pad, &f, DT, &mut ev);
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
    fn a_tap_pushes_less_than_a_hold_and_pushing_caps() {
        let park = flat();
        let mut tap = spawn(&park);
        steps(&mut tap, &park, &press(btn::A), None, 1);
        steps(&mut tap, &park, &Pad::default(), None, 60);
        let mut held = spawn(&park);
        steps(&mut held, &park, &press(btn::A), None, 1);
        steps(&mut held, &park, &hold(btn::A), None, 30);
        steps(&mut held, &park, &Pad::default(), None, 30);
        assert!(tap.speed > 0.5, "{}", tap.speed);
        assert!(
            held.speed > tap.speed + 1.0,
            "{} vs {}",
            held.speed,
            tap.speed
        );
        // Holding A keeps pushing, up to the top push speed.
        let mut s = spawn(&park);
        steps(&mut s, &park, &press(btn::A), None, 1);
        steps(&mut s, &park, &hold(btn::A), None, 60 * 3);
        assert!(s.speed <= s.tuning.push_max_speed + 1e-3);
        assert!(s.speed > s.tuning.push_max_speed - 1.0, "{}", s.speed);
        assert!(s.pos.z > 5.0, "rolled forward along +z");
        // Rolling resistance slows a coasting board, more once the controls are left alone.
        let v = s.speed;
        steps(&mut s, &park, &Pad::default(), None, 60 * 5);
        assert!(s.speed < v - 0.3, "{} -> {}", v, s.speed);
    }

    #[test]
    fn pushes_follow_the_clip_timing_and_caps() {
        let park = flat();
        let c = ClipTiming::BUILT_IN;
        // From a standstill nothing happens until the foot comes down (the push's wind-up).
        let mut s = spawn(&park);
        steps(&mut s, &park, &press(btn::A), None, 1);
        let wind_up = (c.push[0].into * 60.0) as usize;
        steps(&mut s, &park, &hold(btn::A), None, wind_up - 2);
        assert!(s.speed.abs() < 1e-3, "{}", s.speed);
        // Then the board chases the push's end speed, a limited amount a tick.
        let mut last = s.speed;
        for _ in 0..6 {
            steps(&mut s, &park, &hold(btn::A), None, 1);
            assert!(s.speed - last <= s.tuning.push_tick_limit_slow + 1e-4);
            last = s.speed;
        }
        assert!(s.speed > 0.5);
        // A full first push ends at the strong slow clip's end speed.
        steps(&mut s, &park, &Pad::default(), None, 40);
        assert!((s.speed - c.push[0].vel_e).abs() < 0.15, "{}", s.speed);
        // A tap ends at the gentle one's.
        let mut tap = spawn(&park);
        steps(&mut tap, &park, &press(btn::A), None, 1);
        steps(&mut tap, &park, &Pad::default(), None, 60);
        assert!((tap.speed - c.push[2].vel_e).abs() < 0.15, "{}", tap.speed);
        // Holding on: each repeat adds at most the held-push cap, and never past the top push speed.
        let mut s = spawn(&park);
        steps(&mut s, &park, &press(btn::A), None, 1);
        let mut at = Vec::new();
        let mut gains = Vec::new();
        let mut base = 0.0;
        for _ in 0..(60 * 6) {
            let before = s.push.map(|p| p.count);
            steps(&mut s, &park, &hold(btn::A), None, 1);
            if let (Some(b), Some(p)) = (before, s.push)
                && p.count != b
            {
                at.push(s.speed);
                gains.push(s.speed - base);
                base = s.speed;
            }
            assert!(s.speed <= s.tuning.push_max_speed + 1e-3);
        }
        assert!(at.len() >= 3, "{at:?}");
        assert!(
            gains[1..]
                .iter()
                .all(|g| *g <= s.tuning.push_hold_cap + 0.05),
            "{gains:?}"
        );
        assert!(s.speed > s.tuning.push_max_speed - 0.5, "{}", s.speed);
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

    #[test]
    fn a_press_during_a_push_waits_for_the_repush_point() {
        let park = flat();
        let mut s = spawn(&park);
        steps(&mut s, &park, &press(btn::A), None, 1);
        let into = (s.clips.push[0].into * 60.0) as usize;
        steps(&mut s, &park, &Pad::default(), None, into + 1);
        let p = s.push.expect("pushing");
        assert_eq!(p.stage().0, 1, "{p:?}");
        // A tap while the foot is down is kept, and starts the next push only at the re-push point.
        steps(&mut s, &park, &press(btn::A), None, 1);
        assert_eq!(
            s.push.map(|p| (p.count, p.queued.is_some())),
            Some((0, true))
        );
        let mut next_at = None;
        for i in 0..90 {
            steps(&mut s, &park, &Pad::default(), None, 1);
            if next_at.is_none() && s.push.is_some_and(|p| p.count == 1) {
                next_at = Some(i);
            }
        }
        let next_at = next_at.expect("the next push started");
        assert!(next_at as f32 >= p.contact * 60.0 - 3.0, "{next_at}");
        // Both buttons cancel a push.
        let mut s = spawn(&park);
        steps(&mut s, &park, &press(btn::A), None, 1);
        steps(&mut s, &park, &hold(btn::A | btn::B), None, 1);
        assert!(s.push.is_none());
    }

    #[test]
    fn b_pushes_mongo_and_the_brake_waits_for_the_foot() {
        let park = flat();
        let mut s = spawn(&park);
        steps(&mut s, &park, &press(btn::B), None, 1);
        assert!(s.push.is_some_and(|p| p.mongo));
        steps(&mut s, &park, &hold(btn::B), None, 120);
        assert!(s.speed > 2.0);
        // Nothing until the braking foot reaches the ground...
        let v = s.speed;
        let down = (s.clips.brake_down * 60.0) as usize;
        steps(&mut s, &park, &hold(btn::X), None, down - 1);
        assert!(s.brake.is_some());
        assert!(s.speed > v - 0.05, "{v} -> {}", s.speed);
        // ...then it slows the board and holds it still.
        steps(&mut s, &park, &hold(btn::X), None, 40);
        assert!(s.speed < v - 1.0, "foot brake: {v} -> {}", s.speed);
        steps(&mut s, &park, &hold(btn::X), None, 300);
        assert_eq!(s.speed, 0.0);
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
        (wrap_angle(s.travel_yaw() - y0) / 0.5, s.speed)
    }

    #[test]
    fn carves_turn_as_fast_as_sk8_and_cost_no_speed() {
        // SK8: full stick 1.75 rad/s at 6.5 m/s and 1.83 at 8.3 m/s; half a stick 0.42 at 6.65 m/s and
        // 0.48 at 8.46 m/s (a 16 m and a 17.5 m radius). The speed model keeps the speed through them.
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
            assert!(end > speed * 0.98, "{speed} m/s carving ends at {end}");
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
            s.graph = Some(GraphControl {
                pushing,
                ..GraphControl::default()
            });
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
        assert_eq!(s.mode, Mode::Rolling);
        assert!(s.pos.z > 3.0);

        let apex = |strength: f32| {
            let mut s = spawn(&park);
            s.speed = 4.0;
            let mut ev = Vec::new();
            let f = FlickOut {
                flick: Some(Flick::OLLIE),
                strength,
                ..FlickOut::default()
            };
            s.step(&park, &Pad::default(), &f, DT, &mut ev);
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
    fn every_flick_trick_lands_on_flat() {
        let park = flat();
        for t in gesture::TRICKS {
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

    #[test]
    fn regular_stance_mirrors_the_flick() {
        let park = flat();
        let mut s = Skater::spawn_with(&park, Tuning::BUILT_IN, Stance::Regular);
        s.speed = 4.0;
        let ev = steps(
            &mut s,
            &park,
            &Pad::default(),
            Some(Flick::of(Trick::Kickflip)),
            120,
        );
        assert_eq!(landed(&ev), ["HEELFLIP"]);
    }

    #[test]
    fn holding_the_kickflip_flips_it_again() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        // A big pop so there is time for two flips.
        s.tuning.jump_high = 2.4;
        let mut ev = Vec::new();
        s.step(
            &park,
            &Pad::default(),
            &flicked(Flick::of(Trick::Kickflip)),
            DT,
            &mut ev,
        );
        let holding = FlickOut {
            holding_flip: true,
            ..FlickOut::default()
        };
        for _ in 0..30 {
            s.step(&park, &Pad::default(), &holding, DT, &mut ev);
        }
        for _ in 0..90 {
            s.step(&park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
        }
        assert_eq!(landed(&ev), ["DOUBLE KICKFLIP"], "{ev:?}");
    }

    /// Touches down on flat ground moving at `speed` along +z (and `down` m/s into it) with the board
    /// `deg` degrees off the travel (+ nose towards +x), spinning at `spin` rad/s.
    fn touch_down(deg: f32, speed: f32, down: f32, spin: f32) -> (Skater, Vec<Event>) {
        let park = flat();
        let mut s = spawn(&park);
        s.take_off(Some(Flick::OLLIE));
        let a = deg * PI / 180.0;
        s.fwd = v3(sinf(a), 0.0, cosf(a));
        s.vel = v3(0.0, -down, speed);
        s.body.speed = spin;
        let mut ev = Vec::new();
        s.land(Vec3::UP, 0, 0.6, &mut ev);
        (s, ev)
    }

    #[test]
    fn a_sideways_landing_at_riding_speed_rides_away() {
        // Skate 3's landing angle limit is past a right angle below about 9 m/s: no sideways bail.
        let (s, ev) = touch_down(80.0, 5.0, 3.0, 0.0);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        assert_eq!(s.mode, Mode::Rolling);
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
        let t = Tuning::BUILT_IN;
        let park = flat();
        let mut s = spawn(&park);
        s.take_off(Some(Flick::OLLIE));
        s.trick = None;
        let a = 0.98; // about 56 degrees
        let n = v3(0.0, cosf(a), -sinf(a));
        s.fwd = tangent(v3(0.0, 0.0, 1.0), n);
        s.normal = n;
        s.vel = s.fwd * 5.0 - n * 2.0;
        let mut ev = Vec::new();
        s.land(n, 0, 0.6, &mut ev);
        let gain = t.landing_speed_vs_ny.eval(n.y);
        assert!(gain > 1.05);
        assert!((s.speed - 5.0 * gain).abs() < 1e-3, "{}", s.speed);
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
        let ev = steps(&mut s, &park, &press(btn::A), None, 1);
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
            if s.mode == Mode::Rolling {
                break;
            }
        }
        assert!(s.fakie);
        let a = 30.0 * PI / 180.0;
        let slide = Pad {
            stick: [sinf(a), -cosf(a)],
            ..Default::default()
        };
        let mut ev = steps(&mut s, &park, &slide, None, 3);
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
        let nudge = Pad {
            stick: [0.0001, 0.0],
            ..Default::default()
        };
        let ev = steps(&mut s, &park, &nudge, None, 100);
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
        let left = Pad {
            stick: [-1.0, 0.0],
            ..Default::default()
        };
        steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
        steps(&mut s, &park, &left, None, 8);
        let at_release = s.spin;
        let rate = s.spin_rate;
        assert!(rate > 1.0, "{rate}");
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
            let mut ev = Vec::new();
            for _ in 0..150 {
                ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
                if !s.is_airborne() && s.mode == Mode::Rolling {
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

    /// Holds a manual (C-stick lightly back) for `secs`, correcting the balance when `correct`.
    fn hold_manual(s: &mut Skater, park: &Park, secs: f32, correct: bool) -> Vec<Event> {
        let mut ev = Vec::new();
        for _ in 0..(secs / DT) as usize {
            s.speed = 5.0; // stay rolling
            let b = s.manual.map_or(0.0, |m| m.balance);
            let push = if correct {
                MANUAL_SWEET - 1.2 * b
            } else {
                MANUAL_SWEET
            };
            let y = MANUAL_MIN + push.clamp(0.02, 0.98) * (MANUAL_BRAKE - MANUAL_MIN);
            let f = FlickOut {
                stick: [0.0, y],
                ..FlickOut::default()
            };
            s.step(park, &Pad::default(), &f, DT, &mut ev);
        }
        ev
    }

    #[test]
    fn manuals_score_when_balanced_and_tip_over_when_not() {
        let park = flat();
        let mut s = spawn(&park);
        let ev = hold_manual(&mut s, &park, 2.0, true);
        assert!(!ev.contains(&Event::ManualLost), "{ev:?}");
        let m = s.manual.expect("in a manual");
        assert!(!m.nose && m.time > 1.7);
        let ev = steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(
            matches!(&ev[..], [Event::Landed { name, points }] if name == "MANUAL" && *points >= 200),
            "{ev:?}"
        );
        let ev = hold_manual(&mut s, &park, 5.0, false);
        assert!(ev.contains(&Event::ManualLost), "{ev:?}");
        // Stick forward: a nose manual.
        let mut s = spawn(&park);
        s.speed = 5.0;
        let f = FlickOut {
            stick: [0.0, -0.5],
            ..FlickOut::default()
        };
        let mut ev = Vec::new();
        for _ in 0..12 {
            s.step(&park, &Pad::default(), &f, DT, &mut ev);
        }
        assert!(s.manual.is_some_and(|m| m.nose));
        // Pushed past the manual zone: the nose drags and brakes.
        let f = FlickOut {
            stick: [0.0, -1.0],
            ..FlickOut::default()
        };
        let v = s.speed;
        for _ in 0..20 {
            s.step(&park, &Pad::default(), &f, DT, &mut ev);
        }
        assert!(s.manual.is_some_and(|m| m.brake > 0.5), "{ev:?}");
        assert!(s.speed < v - 0.5);
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
        assert!(s.hippy.is_some() && s.push.is_none());
        ev.extend(steps(&mut s, &park, &Pad::default(), None, 120));
        assert_eq!(landed(&ev), ["HIPPIE JUMP"], "{ev:?}");
    }

    #[test]
    fn grabs_follow_the_hands_and_landing_tweaking_one_bails() {
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
        // The C-stick picks the variant (up on the front hand: a nosegrab) and tweaks it; landing still
        // tweaking it wipes out.
        let mut s = spawn(&park);
        s.speed = 5.0;
        steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 5);
        let up = FlickOut {
            stick: [0.0, -0.9],
            ..FlickOut::default()
        };
        let pad = Pad {
            trig_l: 1.0,
            ..Default::default()
        };
        let mut ev = Vec::new();
        for _ in 0..80 {
            s.step(&park, &pad, &up, DT, &mut ev);
        }
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
        assert_eq!(s.bail_reason, Some("landed still tweaking a grab"));
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
            s.step(&park, &pad, &flicked(Flick::OLLIE), DT, &mut ev);
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
            if s.mode == Mode::Grind {
                break;
            }
        }
        (s, ev)
    }

    #[test]
    fn ollie_onto_a_rail_grinds_and_scores_off_the_end() {
        let park = rail_park(4.0);
        let (mut s, ev) = ollie_at_rail(&park, 0.0);
        assert_eq!(s.mode, Mode::Grind, "{ev:?}");
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
        assert_eq!(s.mode, Mode::Rolling);
        assert!(s.pos.z > 4.0);
    }

    #[test]
    fn board_across_the_rail_is_a_boardslide_and_a_flick_pops_off() {
        let park = rail_park(20.0);
        let (mut s, _) = ollie_at_rail(&park, PI / 2.0);
        assert_eq!(s.grind.as_ref().map(|g| g.family), Some(Family::Boardslide));
        steps(&mut s, &park, &Pad::default(), None, 20);
        assert_eq!(s.mode, Mode::Grind);
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
        s.mode = Mode::Air { time: 0.5 };
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
                if s.mode == Mode::Grind {
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
        s.mode = Mode::Air { time: 0.5 };
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
        s.mode = Mode::Air { time: 0.5 };
        let mut ev = steps(&mut s, &park, &Pad::default(), None, 2);
        assert_eq!(s.mode, Mode::Grind, "{ev:?}");
        assert_eq!(
            s.grind.as_ref().map(|g| g.class),
            Some(crate::rails::RailClass::Ledge)
        );
        let mut leaned = 0.0f32;
        let mut ticks = 0;
        while s.mode == Mode::Grind && ticks < 400 {
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            if let Some(g) = &s.grind {
                leaned = leaned.max(g.lean.angle);
            }
            ticks += 1;
        }
        // Off after the lean-off's delay (2 s) and its roll (about 1.3 s), before the ledge's end.
        assert!(s.mode != Mode::Grind && ticks > 150, "{ticks} {ev:?}");
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
        s.mode = Mode::Air { time: 0.5 };
        let mut ev = Vec::new();
        let c = s.investigate(&park, Category::Air, None, 0.0);
        if let Some(c) = c {
            assert!(s.engage(&park, c, Category::Air, &mut ev));
            assert!(ev.contains(&Event::Bailed), "{c:?} {ev:?}");
        } else {
            // No box crosses it: no grind either.
            assert_eq!(s.mode, Mode::Air { time: 0.5 });
        }
        // Crossing a thin rail fast: a wipeout.
        let (park, mut s) = over_rail(v3(0.0, 0.0, 1.0), 0.0, 0.0, 0.1);
        s.vel = v3(8.0, -1.0, 2.0);
        let mut ev = Vec::new();
        let c = s.investigate(&park, Category::Air, None, 0.0).unwrap();
        assert!(s.engage(&park, c, Category::Air, &mut ev));
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
        assert!(s.mode != Mode::Grind, "{ev:?}");
        // A moment later it can.
        steps(&mut s, &park, &Pad::default(), None, 8);
        s.pos = v3(0.25, 0.5 + 0.05 - DECK_HEIGHT, s.pos.z);
        s.vel = v3(0.0, -1.0, 4.0);
        s.plan = None;
        s.mode = Mode::Air { time: 0.5 };
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
            if s.mode == Mode::Grind {
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
        s.mode = Mode::Air { time: 0.5 };
        let mut n = 0;
        while s.mode != Mode::Rolling && n < 10 {
            steps(&mut s, &park, &Pad::default(), None, 1);
            n += 1;
        }
        assert_eq!(s.mode, Mode::Rolling);
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
            assert!(s.mode != Mode::Grind, "caught the coping at {:?}", s.pos);
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
        s.graph = Some(GraphControl {
            pose_com: Some(h),
            ..GraphControl::default()
        });
        steps(&mut s, &park, &Pad::default(), None, 2);
        assert!(s.pump.ground_absorption < -1.0, "{:?}", s.pump);
        assert!(s.pump.min_crouch > 0.1, "{:?}", s.pump);
        let plain = {
            let mut p = spawn(&park);
            place(&mut p, v3(0.0, 0.4, 1.5), n, f);
            p.speed = 6.0;
            p.graph = Some(GraphControl {
                pose_com: Some(h),
                ..GraphControl::default()
            });
            steps(&mut p, &park, &Pad::default(), None, 9);
            p.speed
        };
        let mut most = 0.0f32;
        for _ in 0..7 {
            h += 0.02;
            s.graph = Some(GraphControl {
                pose_com: Some(h),
                ..GraphControl::default()
            });
            steps(&mut s, &park, &Pad::default(), None, 1);
            most = most.max(s.pump.dv);
        }
        assert!(s.speed > plain + 0.3, "{} vs {plain}", s.speed);
        assert!((most - 0.7 * 10.0 * DT).abs() < 0.01, "{most}");
    }

    #[test]
    fn riding_up_into_the_coping_rocks_on_it_and_comes_back_in() {
        let park = coping_pipe();
        let (class, high) = park.rails[0].class_at(5.0);
        assert_eq!(class, crate::rails::RailClass::Ledge);
        assert!(high.z > 0.9, "the deck is the high side: {high:?}");
        let mut rocked = None;
        for k in 0..30 {
            let speed = 7.0 + k as f32 * 0.1;
            let mut s = spawn(&park);
            place(&mut s, v3(0.0, 0.0, -3.0), Vec3::UP, v3(0.0, 0.0, 1.0));
            s.speed = speed;
            let mut ev = Vec::new();
            for _ in 0..300 {
                ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
                if s.mode == Mode::Grind && rocked.is_none() {
                    rocked = s.grind.clone();
                }
            }

            if let Some(g) = &rocked {
                assert!(
                    matches!(g.entry, Entry::RideIntoCoping | Entry::RideFromBelow),
                    "{:?}",
                    g.entry
                );
                assert!(!ev.contains(&Event::Bailed), "{ev:?}");
                // Back in the pipe, rolling down it.
                assert!(
                    s.mode == Mode::Rolling && s.pos.y < 3.5,
                    "{:?} {:?}",
                    s.mode,
                    s.pos
                );
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
        s.mode = Mode::Air { time: 0.5 };
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
        s.mode = Mode::Air { time: 0.5 };
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
            assert_ne!(s.mode, Mode::Grind, "{ev:?}");
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
        assert_eq!(s.mode, Mode::Rolling, "{ev:?}");
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
        assert_eq!(s.mode, Mode::Rolling);
        assert!(
            s.fakie && s.speed < -1.0,
            "rolled back in fakie: {}",
            s.speed
        );
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
        assert_eq!(s.mode, Mode::Rolling, "back on the board");
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
        s.pos = v3(-3.2, 1.3, -5.8);
        s.take_off(None);
        s.vel = v3(-3.0, 1.0, -1.0);
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
        assert_eq!(s.mode, Mode::Rolling);
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
        assert_eq!(s.mode, Mode::Rolling);
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
        assert!(slow_slip < 0.03, "{slow_slip}");
        assert!(fast_slip > 0.05, "{fast_slip}");
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
        s.step(&park, &Pad::default(), &flicked(Flick::OLLIE), DT, &mut ev);
        assert!(s.is_airborne(), "{ev:?}");
        // Straight up (a little back in), not thrown out over the flat.
        assert!(s.vel.y > 2.5, "{:?}", s.vel);
        assert!(s.vel.z.abs() < 0.5, "{:?}", s.vel);
        // On flat ground the same pop goes straight up from the board.
        let mut s = spawn(&park);
        s.pos = v3(0.0, 0.0, -10.0);
        s.speed = 3.0;
        s.step(&park, &Pad::default(), &flicked(Flick::OLLIE), DT, &mut ev);
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
        s.step(&park, &Pad::default(), &flicked(Flick::OLLIE), DT, &mut ev);
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
        s.graph = Some(GraphControl::default());
        steps(&mut s, &park, &hold(btn::A), None, 30);
        assert!(s.speed.abs() < 0.05, "no contact yet: {}", s.speed);
        s.graph = Some(GraphControl {
            push_contact: true,
            push_target: Some(3.0),
            ..GraphControl::default()
        });
        steps(&mut s, &park, &hold(btn::A), None, 1);
        s.graph = Some(GraphControl::default());
        steps(&mut s, &park, &hold(btn::A), None, 30);
        assert!(s.speed > 1.0, "the push drove: {}", s.speed);
    }

    #[test]
    fn graph_flicks_pop_only_on_the_graphs_jump() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 3.0;
        s.graph = Some(GraphControl::default());
        steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 5);
        assert_eq!(s.mode, Mode::Rolling, "a flick alone doesn't pop");
        s.graph = Some(GraphControl {
            pop: Some((Flick::of(crate::gesture::Trick::Kickflip), 1.0)),
            ..GraphControl::default()
        });
        let ev = steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(matches!(s.mode, Mode::Air { .. }));
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
        s.graph = Some(GraphControl {
            manual: Some((false, 1.0, false)),
            ..GraphControl::default()
        });
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
        s.graph = Some(GraphControl {
            manual: Some((false, 0.0, false)),
            ..GraphControl::default()
        });
        steps(&mut s, &park, &Pad::default(), None, 60);
        assert!(
            s.manual.unwrap().pitch < full * 0.5,
            "a light lean pitches less"
        );
        s.graph = Some(GraphControl::default());
        steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(s.manual.is_none(), "the graph ended it");
    }

    #[test]
    fn graph_turns_the_rider_round_out_of_fakie() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = -2.0;
        s.fakie = true;
        let was = s.switch;
        s.graph = Some(GraphControl {
            turn_round: true,
            ..GraphControl::default()
        });
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
        s.graph = Some(GraphControl {
            pop: Some((Flick::OLLIE, 1.0)),
            ..GraphControl::default()
        });
        let mut ev = steps(&mut s, &park, &Pad::default(), None, 1);
        let mut k = 0;
        while matches!(s.mode, Mode::Air { .. }) && k < 300 {
            s.graph = Some(air(k));
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 1));
            k += 1;
        }
        (s, ev)
    }

    /// A long flight off flat ground with the graphs asking for a body flip; `grab(k)`: the grab held
    /// on air tick `k`.
    fn body_flip_air(front: bool, grab: impl Fn(u32) -> bool) -> (Skater, Vec<Event>, f32) {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 3.0;
        s.pos.y = 0.05;
        s.vel = v3(0.0, 6.5, 3.0);
        s.mode = Mode::Air { time: 0.0 };
        let mut ev = Vec::new();
        let mut k = 0;
        let mut most = 0.0f32;
        while matches!(s.mode, Mode::Air { .. }) && k < 300 {
            s.graph = Some(GraphControl {
                body_flip: (k < 20).then_some(front),
                grabbing: grab(k),
                ..GraphControl::default()
            });
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
        assert_eq!(s.mode, Mode::Rolling);
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
        s.mode = Mode::Air { time: 0.0 };
        s.graph = Some(GraphControl {
            grabbing: true,
            ..GraphControl::default()
        });
        steps(&mut s, &park, &Pad::default(), None, 30);
        assert!(s.body_flip.is_none());
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
        assert!(matches!(s.mode, Mode::Bailed { .. }), "{:?}", s.mode);
        // The same trick caught in time: a clean landing, however long `trick_duration` says.
        let (s, ev) = graph_air(|k| GraphControl {
            air_trick: Some(trick(false)),
            danger: k < 10,
            ..GraphControl::default()
        });
        assert_eq!(s.mode, Mode::Rolling);
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
        assert_eq!(s.mode, Mode::Rolling, "{ev:?}");
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
