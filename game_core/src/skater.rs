//! The skater: what the rider does (stance, pushing, braking, powerslides, manuals, kick turns, reverts,
//! flick tricks, grabs, grinds, landings, bails) on top of the board's physics (`board`: one rigid board
//! on two trucks with a full velocity, riding whatever is under its wheels: flat ground, banks, curved
//! transitions and walls). In the air the board flies ballistically and eases towards the surface it is
//! going to land on. Deterministic, stepped at Skate 3's 60 Hz. The numbers come from [`Tuning`] (Skate
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

mod board;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub use board::{AXLE, Drive, Pump, WALLRIDE_MIN_HEIGHT, WallRide};

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
/// Landing with the board tilted further than this from the surface (rad) is a bail.
const LAND_TILT: f32 = 40.0 * PI / 180.0;
/// How fast the board turns towards the surface it will land on (rad/s).
const AIR_TILT_RATE: f32 = 3.0;
/// Below this speed a hard sideways stick kick-turns on the spot.
const KICKTURN_SPEED: f32 = 1.0;
const KICKTURN_RATE: f32 = 4.5;
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
/// A repeated push puts the foot down this long after the re-push point (s, ours).
const PUSH_REPEAT_DOWN: f32 = 0.1;
/// Powerslides: the board angle eases at this rate (rad/s, ours); a held slide relaxes after this long
/// (s).
const SLIDE_TURN_RATE: f32 = 8.0;
const SLIDE_RELAX: f32 = 0.5;
/// Down on the ground after a bail before respawning: long enough for the fall and get-up clips.
const BAIL_TIME: f32 = 2.2;
/// Stepping off the board after a runout and back on.
const RUNOUT_TIME: f32 = 1.2;
/// Landing with the board further off the travel direction than this (as sin of the angle) is sketchy:
/// Skate 3's "straight" landing limit.
const STRAIGHT_LANDING: f32 = 0.2;
/// Hitting the ground faster than this (m/s, downwards) is a heavy landing.
const HEAVY_IMPACT: f32 = 7.0;
/// Coming down this sideways (rad) while moving faster than `SIDEWAYS_SPEED` is a bail. (Skate 3's
/// landing-angle limit is lenient at riding speeds and leaves these to its physics; this is flatground's
/// stand-in.)
const SIDEWAYS_BAIL: f32 = 70.0 * PI / 180.0;
const SIDEWAYS_SPEED: f32 = 1.5;
/// How long a landing is remembered (drawn: the landing clip), and how long after it a revert counts.
const LANDING_SHOW: f32 = 1.5;
const REVERT_WINDOW: f32 = 0.4;
const REVERT_TIME: f32 = 0.3;
const REVERT_POINTS: u32 = 100;
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
/// Analog trigger past this holds a grab (Skate 3: any pressure).
const GRAB_TRIGGER: f32 = 0.15;
const GRAB_POINTS: u32 = 200;
const GRAB_POINTS_PER_S: f32 = 150.0;
/// Both push buttons pressed within this of each other wind up a hippie jump.
const HIPPY_CHORD: f32 = 0.08;
const HIPPY_POINTS: u32 = 150;
/// Most flips a held kickflip or heelflip can reach.
const MAX_FLIPS: u8 = 3;
/// How close (m, seen from above) the board has to come down to a rail to lock on.
const GRIND_REACH: f32 = 0.4;
/// Board within this of across the rail: across it (a slide).
const GRIND_ACROSS: f32 = 30.0 * PI / 180.0;
const GRIND_FRICTION: f32 = 0.8;
/// Slides (the deck on the rail) drag more than grinds (the trucks on it).
const SLIDE_FRICTION: f32 = 1.6;
/// Board within this of the rail's direction is along it; beyond `GRIND_ACROSS` from along it is across.
const GRIND_ANGLED: f32 = 20.0 * PI / 180.0;
/// Across the rail with the rail this far towards an end of the board (m): a nose or tail slide.
const TIP_SLIDE: f32 = 0.2;
const GRIND_MIN_SPEED: f32 = 0.6;
const GRIND_MIN_TIME: f32 = 0.15;
const GRIND_POINTS: u32 = 150;
const GRIND_POINTS_PER_S: f32 = 200.0;
/// Spin wind-up history (ticks).
const SPIN_HISTORY: usize = 30;

/// Which foot the rider naturally has forward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stance {
    /// Right foot forward: Skate 3's default.
    #[default]
    Goofy,
    Regular,
}

/// How long a flick trick takes (s): landing before it finishes is a bail.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandKind {
    Clean,
    /// A big drop.
    Heavy,
    /// Board a bit crooked to the travel direction; `frontside` when it is turned towards the rider's
    /// chest.
    Sketchy {
        frontside: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub kind: LandKind,
    /// Seconds since touching down.
    pub time: f32,
    /// How far the board was off the travel direction (rad), and how fast it hit the ground (m/s, down).
    pub crooked: f32,
    pub impact: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrabKind {
    /// Rear hand, toe side (frontside).
    Indy,
    /// Front hand, heel side (backside).
    Melon,
    /// Both hands.
    Mute,
}

impl GrabKind {
    pub fn name(self) -> &'static str {
        match self {
            GrabKind::Indy => "INDY",
            GrabKind::Melon => "MELON",
            GrabKind::Mute => "MUTE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grab {
    pub kind: GrabKind,
    /// Seconds held.
    pub time: f32,
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
}

impl ActiveTrick {
    /// 0..1 progress of the board animation.
    pub fn progress(&self) -> f32 {
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
        match self.flips {
            2 => format!("DOUBLE {base}"),
            3 => format!("TRIPLE {base}"),
            _ => base,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Revert {
    pub side: f32,
    pub time: f32,
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

/// Stance and how the rider took off (for names).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Takeoff {
    fakie: bool,
    switch: bool,
    hippy: bool,
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
    /// Extra yaw spun in the air, relative to `heading` at take-off.
    pub spin: f32,
    pub spin_rate: f32,
    /// Strongest stick wind-up around take-off (0..1).
    spin_windup: f32,
    spin_history: [f32; SPIN_HISTORY],
    spin_at: usize,
    last_stick_x: f32,
    pub trick: Option<ActiveTrick>,
    /// Riding with the other foot forward (the drawn rider is mirrored).
    pub switch: bool,
    /// Rolling tail first (Skate 3's fakie detection: a moment of backwards speed).
    pub fakie: bool,
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
    /// The deck's tilt that turns the trucks (rad), and seconds since the wheels came down (the trucks'
    /// turn ramps back in).
    pub deck_tilt: f32,
    truck_ramp: f32,
    /// The board's yaw rate last tick (rad/s).
    pub yaw_rate: f32,
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
    /// The rail just left and how long until it can be locked onto again.
    regrind_block: Option<(usize, f32)>,
    pub brake: Option<FootBrake>,
    pub slide: Option<Powerslide>,
    /// Last tick's powerslide reading of the stick (entry window side, continuous amount).
    last_slide: (f32, f32),
    pub revert: Option<Revert>,
    /// Kick-turning on the spot: direction (+1 left), seconds.
    pub kickturn: Option<(f32, f32)>,
    /// Winding up a hippie jump: seconds held.
    pub hippy: Option<f32>,
    /// When each push button last went down (seconds ago), for the hippie jump chord.
    since_a: f32,
    since_b: f32,
    /// Why the last bail happened (for the log).
    pub bail_reason: Option<&'static str>,
    /// Riding a wall.
    pub wallride: Option<WallRide>,
    /// The surface the board is expected to land on (its normal), refreshed every few frames in the air.
    land_up: Vec3,
    predict_in: u32,
    /// The C-stick's y in gesture space this frame (+ towards the player): picks 5-0s and nosegrinds.
    grind_stick: f32,
    /// The numbers behind the last landing check (for the log).
    pub last_landing: Option<LandingCheck>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrindKind {
    /// Board along the rail, both trucks on it.
    FiftyFifty,
    /// Along it on the back truck (C-stick held back) / the front truck (held forward).
    FiveO,
    Nosegrind,
    /// Angled across it on the front truck: nose over the far side, or over the near side.
    Crooked,
    Willy,
    /// Angled across it on the back truck: nose over the far side, or over the near side.
    Feeble,
    Smith,
    /// Across it on the deck: the front end went over the rail (boardslide) or the tail did (lipslide).
    Boardslide,
    Lipslide,
    /// Across it on the nose or the tail.
    Noseslide,
    Tailslide,
}

impl GrindKind {
    pub fn name(self) -> &'static str {
        match self {
            GrindKind::FiftyFifty => "50-50",
            GrindKind::FiveO => "5-0",
            GrindKind::Nosegrind => "NOSEGRIND",
            GrindKind::Crooked => "CROOKED GRIND",
            GrindKind::Willy => "WILLY GRIND",
            GrindKind::Feeble => "FEEBLE GRIND",
            GrindKind::Smith => "SMITH GRIND",
            GrindKind::Boardslide => "BOARDSLIDE",
            GrindKind::Lipslide => "LIPSLIDE",
            GrindKind::Noseslide => "NOSESLIDE",
            GrindKind::Tailslide => "TAILSLIDE",
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
        )
    }

    /// Which way the board sits on the rail, from its angle to it (0 along, pi/2 across), where along the
    /// board the rail crosses (m, + towards the nose), whether the nose points to the far side of the
    /// rail, and the C-stick (+y towards the player).
    pub fn from_contact(angle: f32, cross: f32, nose_far: bool, stick_y: f32) -> GrindKind {
        if angle < GRIND_ANGLED {
            if stick_y > 0.35 {
                GrindKind::FiveO
            } else if stick_y < -0.35 {
                GrindKind::Nosegrind
            } else {
                GrindKind::FiftyFifty
            }
        } else if angle < PI / 2.0 - GRIND_ACROSS {
            match (cross >= 0.0, nose_far) {
                (true, true) => GrindKind::Crooked,
                (true, false) => GrindKind::Willy,
                (false, true) => GrindKind::Feeble,
                (false, false) => GrindKind::Smith,
            }
        } else if cross > TIP_SLIDE {
            GrindKind::Noseslide
        } else if cross < -TIP_SLIDE {
            GrindKind::Tailslide
        } else if nose_far {
            GrindKind::Boardslide
        } else {
            GrindKind::Lipslide
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grind {
    /// Index into the terrain's rails.
    pub rail: usize,
    /// Distance along the rail.
    pub s: f32,
    /// +1 travelling towards the rail's end, -1 towards its start.
    pub dir: f32,
    pub speed: f32,
    pub kind: GrindKind,
    /// Frontside: the rider faced the rail coming onto it.
    pub frontside: bool,
    pub time: f32,
    /// The trick that took the skater onto the rail (named with the grind).
    pub into: Option<(String, u32)>,
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
            spin_windup: 0.0,
            spin_history: [0.0; SPIN_HISTORY],
            spin_at: 0,
            last_stick_x: 0.0,
            trick: None,
            switch: false,
            fakie: false,
            fakie_timer: 0.0,
            since_spawn: 0.0,
            takeoff: Takeoff::default(),
            push: None,
            push_press: None,
            push_steer: 1.0,
            steer: 0.0,
            deck_tilt: 0.0,
            truck_ramp: 1.0,
            yaw_rate: 0.0,
            collide: None,
            wall_clock: f32::INFINITY,
            vn: 0.0,
            gap_time: 0.0,
            lead_rise: 0.0,
            pump: Pump::default(),
            idle: 0.0,
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
            brake: None,
            slide: None,
            last_slide: (0.0, 0.0),
            revert: None,
            kickturn: None,
            hippy: None,
            since_a: f32::INFINITY,
            since_b: f32::INFINITY,
            bail_reason: None,
            last_landing: None,
            wallride: None,
            land_up: Vec3::UP,
            predict_in: 0,
            grind_stick: 0.0,
            fwd: yaw_dir(heading),
        }
    }

    /// Back to the spawn point, keeping the tuning, clip timing and stance.
    pub fn respawn(&mut self, park: &dyn Terrain) {
        let clips = self.clips;
        *self = Skater::spawn_with(park, self.tuning, self.stance);
        self.clips = clips;
    }

    pub fn is_airborne(&self) -> bool {
        matches!(self.mode, Mode::Air { .. })
    }

    /// The board's frame in the world: right, up (the surface normal or the board's own up in the air),
    /// nose.
    pub fn board_frame(&self) -> crate::math::Mat3 {
        let up = self.normal.normalize_or(Vec3::UP);
        let f = tangent(self.fwd, up);
        crate::math::Mat3::from_cols(up.cross(f), up, f)
    }

    /// Points the nose at `yaw` (seen from above), along the surface under the board.
    pub fn set_heading(&mut self, yaw: f32) {
        self.fwd = tangent(yaw_dir(yaw), self.normal);
        self.heading = yaw;
    }

    /// Sets the air spin to `spin` (rad), turning the board and rider about the vertical to match.
    pub fn set_spin(&mut self, spin: f32) {
        let d = spin - self.spin;
        self.fwd = rotate(self.fwd, Vec3::UP, d);
        self.normal = rotate(self.normal, Vec3::UP, d);
        self.spin = spin;
        self.sync_heading();
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
        if let Some(r) = &self.revert {
            // The revert turns the board through half a turn; `heading` flips at the end.
            y += r.side * PI * (r.time / REVERT_TIME).min(1.0);
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
            || pad.trig_l > 0.1
            || pad.trig_r > 0.1;
        self.idle = if touched { 0.0 } else { self.idle + dt };
        self.since_spawn += dt;
        let rolling = self.mode == Mode::Rolling;
        let crouch = if rolling && self.manual.is_none() {
            flick
                .crouch
                .max(if self.hippy.is_some() { 1.0 } else { 0.0 })
        } else {
            0.0
        };
        self.crouch += (crouch - self.crouch) * smooth(18.0, dt);
        self.grind_stick = flick.stick[1];
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
        self.record_spin_input(pad.stick[0]);
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
            Mode::Grind => self.slide_rail(park, flick, dt, events),
            Mode::Bailed { time } | Mode::Runout { time } => {
                let time = time + dt;
                let h = self.vel.horizontal() * (1.0 - smooth(3.0, dt));
                self.vel = Vec3 { y: 0.0, ..h };
                self.try_move(park, self.vel * dt);
                self.pos.y = park
                    .ground(self.pos.x, self.pos.z, self.pos.y + STEP)
                    .height;
                match self.mode {
                    Mode::Bailed { .. } if time >= BAIL_TIME => {
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

    /// Keeps the last ticks of the main stick's sideways movement (for the spin wind-up).
    fn record_spin_input(&mut self, x: f32) {
        let d = 1.5 * (x - self.last_stick_x);
        self.last_stick_x = x;
        // Only movement towards where the stick is pointing counts.
        let d = if d * x > 0.1 { d } else { 0.0 };
        self.spin_history[self.spin_at] = d;
        self.spin_at = (self.spin_at + 1) % SPIN_HISTORY;
    }

    /// The strongest wind-up in the history, weighted by how long ago it was (`now` = seconds since
    /// take-off).
    fn windup_peak(&self, now: f32, dt: f32) -> f32 {
        let mut peak: f32 = 0.0;
        for k in 0..SPIN_HISTORY {
            // k = 0 is the newest sample.
            let i = (self.spin_at + SPIN_HISTORY - 1 - k) % SPIN_HISTORY;
            let t = now - k as f32 * dt;
            peak = peak.max(self.spin_history[i].abs() * self.tuning.spin_windup.eval(t));
        }
        peak.min(1.0)
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

        // Revert: a powerslide flick just after landing spins the board half round.
        if let Some(r) = &mut self.revert {
            r.time += dt;
            if r.time >= REVERT_TIME {
                self.revert = None;
                self.flip_board();
                self.fakie = self.speed < 0.0;
                self.fakie_timer = 0.0;
                events.push(Event::Landed {
                    name: "REVERT".into(),
                    points: REVERT_POINTS,
                });
            }
        } else if st.slide != 0.0
            && self.landing.is_some_and(|l| l.time < REVERT_WINDOW)
            && self.manual.is_none()
            && self.speed.abs() > 1.0
        {
            self.revert = Some(Revert {
                side: st.slide,
                time: 0.0,
            });
            self.landing = None;
        }

        self.update_slide(&st, dt);

        // Kick turn: slow, stick hard to the side.
        let kick = self.speed.abs() < KICKTURN_SPEED
            && pad.stick[0].abs() > KICKTURN_STICK
            && self.manual.is_none()
            && self.slide.is_none()
            && self.revert.is_none()
            && self.push.is_none_or(|p| p.stage().0 == 2);
        if kick {
            let dir = -pad.stick[0].signum();
            let time = self.kickturn.map_or(0.0, |k| k.1) + dt;
            self.kickturn = Some((dir, time));
            self.fwd = rotate(self.fwd, self.contact_n, dir * KICKTURN_RATE * dt);
        } else {
            self.kickturn = None;
        }

        // Steering: Skate 3's reading of the stick, eased in. A powerslide's trucks hold a steady turn.
        let follow = 1.0 - libm::powf(1.0 - t.steer_follow.clamp(0.0, 1.0), ticks);
        let turn_in = match &self.slide {
            Some(s) if !s.releasing => s.side * t.slide_turn_value,
            Some(_) => 0.0,
            None if kick => 0.0,
            None if st.hard != 0.0 => st.turn.signum(),
            None => st.turn,
        };
        self.steer += (turn_in - self.steer) * follow;

        // Pushing (A: the usual push, B: mongo), hippie jump (both), foot brake (X).
        self.update_hippy(pad, dt, events);
        if self.mode != Mode::Rolling {
            return;
        }
        let push_target = self.update_push(pad, dt, events);
        let foot_brake = self.update_brake(pad, dt);
        let brake = t.brake_scale
            * t.braking_scalar
            * if tail_brake {
                t.tail_brake
            } else if foot_brake {
                t.foot_brake
            } else {
                0.0
            };

        if let Some(f) = flick.flick {
            let f = gesture::select(f, self.flick_group(), self.mirrored());
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
                self.vel.y += self.pop_speed(self.tuning.jump_height(flick.strength, 0.0)) * 0.6;
                events.push(Event::Popped(f));
                return;
            }
            self.pop(f, flick.strength, pad.stick, events);
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
            brake,
            manual: self.manual.is_some(),
            slide: self
                .slide
                .as_ref()
                .filter(|s| !s.releasing)
                .map(|s| s.angle),
            pivot: kick || self.revert.is_some(),
            intentional_pump: flick.crouch > 0.0 || self.crouch > 0.05,
        };
        self.ride(park, &drive, dt, events);
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
        self.vel = self.pop_velocity(self.pop_speed(h), stick);
        self.pos += self.contact_n * 0.02;
        self.take_off(Some(f));
        events.push(Event::Popped(f));
    }

    fn take_off(&mut self, f: Option<Flick>) {
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
        self.hippy = None;
        self.wallride = None;
        self.collide = None;
        self.vn = 0.0;
        self.gap_time = 0.0;
        self.lead_rise = 0.0;
        self.pump = Pump::default();
        self.predict_in = 0;
        self.land_up = self.normal;
        self.spin = 0.0;
        self.spin_rate = 0.0;
        self.spin_windup = 0.0;
        self.takeoff = Takeoff {
            fakie: self.fakie,
            switch: self.switch,
            hippy: false,
        };
        self.trick = f.map(|f| ActiveTrick {
            flick: f,
            t: 0.0,
            duration: trick_duration(f.trick),
            flips: 1,
        });
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
        // Body spin: Skate 3's model. A quick wind-up of the stick around take-off unlocks the full spin
        // speed; without one it is a fraction of it. The rider and board turn about the vertical.
        self.spin_windup = self.spin_windup.max(self.windup_peak(time, dt));
        let scale = t.spin_no_windup + (1.0 - t.spin_no_windup) * self.spin_windup;
        let target = -scale * t.spin_speed.eval(time) * pad.stick[0];
        let reversing = target * self.spin_rate < 0.0;
        let limit = if reversing {
            t.spin_reverse_change
        } else {
            t.spin_change.eval(time)
        } * dt
            * 60.0;
        self.spin_rate += clamp(target - self.spin_rate, -limit, limit);
        let turn = self.spin_rate * dt;
        self.spin += turn;
        self.fwd = rotate(self.fwd, Vec3::UP, turn);
        self.normal = rotate(self.normal, Vec3::UP, turn);
        if let Some(tr) = &mut self.trick {
            tr.t += dt;
            // Holding a kickflip or heelflip on the stick keeps it flipping.
            let flip_only = matches!(tr.flick.trick, Trick::Kickflip | Trick::Heelflip);
            let one = trick_duration(tr.flick.trick);
            if flip_only && flick.holding_flip && tr.flips < MAX_FLIPS && tr.t > tr.duration - 0.1 {
                tr.flips += 1;
                tr.duration += one;
            }
        }
        self.update_grab(pad, dt);

        // Ease the board towards the surface it is heading for.
        if self.predict_in == 0 {
            self.land_up = self.predict_landing(park);
            self.predict_in = 4;
        } else {
            self.predict_in -= 1;
        }
        self.normal = turn_towards(self.normal, self.land_up, AIR_TILT_RATE * dt);
        self.fwd = tangent(self.fwd, self.normal);

        self.vel.y -= t.gravity * dt;
        let from = self.pos;
        let delta = self.vel * dt;
        let len = delta.length();
        let mut moved = false;
        if len > 1e-6 {
            let dir = delta * (1.0 / len);
            if let Some(h) = park.raycast(from, dir, len + 0.01)
                && self.vel.dot(h.normal) < 0.0
            {
                let landable = self.normal.dot(h.normal) > 0.5 || h.normal.y > 0.6;
                if landable {
                    self.pos = h.point;
                    if self.try_grind(park) {
                        return;
                    }
                    self.land(h.normal, time, events);
                    return;
                }
                // A wall: a wallride if there is a floor well below, else off it (or off the board).
                if h.normal.y.abs() < 0.5 && self.try_air_wallride(park, &h, events) {
                    return;
                }
                self.pos = h.point + h.normal * 0.02;
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

        if self.try_grind(park) {
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

    /// The normal of the surface the board will come down on, found by following its flight ahead
    /// (level ground if nothing is found). Only surfaces the board can turn to in time count: ground
    /// facing up, or (on a vert air) the face it left.
    fn predict_landing(&self, park: &dyn Terrain) -> Vec3 {
        let g = self.tuning.gravity;
        let (mut p, mut v) = (self.pos, self.vel);
        let step = 0.06;
        for _ in 0..25 {
            let next_v = v - Vec3::UP * (g * step);
            let d = (v + next_v) * (0.5 * step);
            let len = d.length();
            if len > 1e-6
                && let Some(h) = park.raycast(p, d * (1.0 / len), len)
            {
                return if h.normal.y > 0.5 || self.normal.dot(h.normal) > 0.5 {
                    h.normal
                } else {
                    Vec3::UP
                };
            }
            p += d;
            v = next_v;
        }
        Vec3::UP
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

    /// Coming down onto a rail locks on. Which grind or slide it is comes from how the board meets it, as
    /// in Skate 3: its angle to the rail, where along the board the rail crosses, which way the nose
    /// points, and (along it) the C-stick for 5-0s and nosegrinds.
    ///
    /// Phase-6 hook: the same lock-on test is what a grind from riding (into coping, onto a curb or ledge)
    /// will call, with the ground velocity.
    pub(crate) fn try_grind(&mut self, park: &dyn Terrain) -> bool {
        let free = self.trick.is_none_or(|t| t.progress() >= 1.0);
        if self.vel.y > 0.0 || self.grab.is_some() || !free {
            return false;
        }
        let nose = self.fwd.horizontal().normalize_or(yaw_dir(self.heading));
        for (i, rail) in park.rails().iter().enumerate() {
            if self.regrind_block.is_some_and(|(r, _)| r == i) {
                continue;
            }
            let Some(n) = rail.nearest(self.pos) else {
                continue;
            };
            let (p, d) = rail.at(n.s);
            if n.dist_h > GRIND_REACH || !(-0.15..=0.3).contains(&(self.pos.y - p.y)) {
                continue;
            }
            let dh = d.horizontal().normalize_or(d);
            let along_dot = nose.dot(dh);
            let angle = libm::acosf(along_dot.abs().min(1.0));
            // Where the board's long axis crosses the rail (m from the middle, + towards the nose).
            let cross2 = |a: Vec3, b: Vec3| a.x * b.z - a.z * b.x;
            let denom = cross2(nose, dh);
            let cross = if denom.abs() > 0.2 {
                cross2(p - self.pos, dh) / denom
            } else {
                0.0
            };
            // Coming at the rail from one side: the nose over the far side, and facing it or not.
            let vh = self.vel.horizontal();
            let perp = vh - dh * vh.dot(dh);
            let across_motion = perp.length() > 0.3;
            let to_far = perp.normalize_or(Vec3::ZERO);
            let nose_far = !across_motion || nose.dot(to_far) > 0.0;
            let chest = {
                let right = self.normal.cross(self.fwd).normalize_or(Vec3::ZERO);
                if self.mirrored() { -right } else { right }
            };
            let frontside = across_motion && chest.dot(to_far) > 0.0;
            let kind = GrindKind::from_contact(angle, cross, nose_far, self.grind_stick);
            // Speed along the rail: the whole 3-D velocity along it (Skate 3), no minimum.
            let along = self.vel.dot(d);
            let dir = if along >= 0.0 { 1.0 } else { -1.0 };
            let into = self
                .trick_label()
                .map(|(n, p)| (format!("{}{n}", self.stance_prefix()), p));
            // Settle the board on the rail the way it came in: along it (either way round), across it, or
            // angled.
            let travel = d * dir;
            let side = v3(-travel.z, 0.0, travel.x).normalize_or(Vec3::ZERO);
            self.fwd = if angle < GRIND_ANGLED {
                if along_dot * dir >= 0.0 {
                    travel
                } else {
                    -travel
                }
            } else if angle > PI / 2.0 - GRIND_ACROSS {
                if nose.dot(side) >= 0.0 { side } else { -side }
            } else {
                nose
            };
            self.normal = Vec3::UP;
            self.contact_n = Vec3::UP;
            self.spin_rate = 0.0;
            self.pos = p;
            self.grind = Some(Grind {
                rail: i,
                s: n.s,
                dir,
                speed: along.abs(),
                kind,
                frontside,
                time: 0.0,
                into,
            });
            self.trick = None;
            self.grabbed = None;
            self.takeoff.hippy = false;
            self.mode = Mode::Grind;
            return true;
        }
        false
    }

    /// One step along the rail: gravity along it, a little friction; off the end, out of speed, or
    /// popped off with a flick.
    fn slide_rail(
        &mut self,
        park: &dyn Terrain,
        flick: &FlickOut,
        dt: f32,
        events: &mut Vec<Event>,
    ) {
        let (Some(mut g), Some(rail)) = (
            self.grind.clone(),
            self.grind.as_ref().and_then(|g| park.rails().get(g.rail)),
        ) else {
            // The rail went away (a park reload): just drop.
            self.grind = None;
            self.take_off(None);
            return;
        };
        let (_, d) = rail.at(g.s);
        let friction = if g.kind.is_slide() {
            SLIDE_FRICTION
        } else {
            GRIND_FRICTION
        };
        g.speed += (-self.tuning.gravity * d.y * g.dir - friction) * dt;
        g.s += g.dir * g.speed * dt;
        g.time += dt;
        let (p, d) = rail.at(g.s);
        let travel = d * g.dir;
        self.pos = p;
        self.speed = g.speed;
        let off_end = g.s <= 0.0 || g.s >= rail.length();
        let done = flick.flick.is_some() || off_end || g.speed < GRIND_MIN_SPEED;
        self.grind = Some(g.clone());
        if !done {
            return;
        }
        // Leaving the rail: score it, square the board back up and fly off along it.
        if g.time >= GRIND_MIN_TIME {
            let side = if g.frontside { "FS" } else { "BS" };
            let name = match &g.into {
                Some((t, _)) => format!("{t} {side} {}", g.kind.name()),
                None => format!("{side} {}", g.kind.name()),
            };
            let points = g.into.as_ref().map_or(0, |t| t.1)
                + GRIND_POINTS
                + (g.time * GRIND_POINTS_PER_S) as u32;
            events.push(Event::Landed { name, points });
        }
        // Square the board back up along the rail, nose first or tail first, whichever is nearer.
        let backwards = self.fwd.dot(travel) < 0.0;
        self.fwd = if backwards { -travel } else { travel };
        self.normal = Vec3::UP;
        self.contact_n = Vec3::UP;
        self.grind = None;
        self.regrind_block = Some((g.rail, 0.4));
        self.vel = travel * g.speed.max(0.0);
        self.pos += Vec3::UP * 0.02;
        // Fakie onto the rail (board backwards): still fakie leaving it.
        self.fakie = backwards;
        match flick.flick {
            Some(f) => {
                let f = gesture::select(f, Group::Square, self.mirrored());
                let t = self.tuning;
                let h = t.jump_height(flick.strength, g.speed);
                let response = t.vertical_response.eval(0.0).max(t.jump_min_scalar);
                self.vel += Vec3::UP * (self.pop_speed(h) * response);
                self.take_off(Some(f));
                events.push(Event::Popped(f));
            }
            None => self.take_off(None),
        }
    }

    fn bail(&mut self, reason: &'static str, events: &mut Vec<Event>) {
        self.bail_reason = Some(reason);
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
        self.hippy = None;
        self.collide = None;
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
    /// right: which grab that is depends on the stance. Letting go keeps the longest grab of the jump.
    fn update_grab(&mut self, pad: &Pad, dt: f32) {
        let (l, r) = (pad.trig_l > GRAB_TRIGGER, pad.trig_r > GRAB_TRIGGER);
        // The clips' goofy rider has the right hand at the back.
        let (rear, front) = if self.mirrored() { (l, r) } else { (r, l) };
        let held = match (rear, front) {
            (true, true) => Some(GrabKind::Mute),
            (true, false) => Some(GrabKind::Indy),
            (false, true) => Some(GrabKind::Melon),
            (false, false) => None,
        };
        let free = self.trick.is_none_or(|t| t.progress() >= 1.0);
        match (&mut self.grab, held) {
            (Some(g), Some(kind)) => {
                g.time += dt;
                // Adding the second hand turns it into a mute grab.
                if kind == GrabKind::Mute {
                    g.kind = kind;
                }
            }
            (Some(g), None) => {
                let g = *g;
                if self.grabbed.is_none_or(|b| g.time > b.time) {
                    self.grabbed = Some(g);
                }
                self.grab = None;
            }
            (None, Some(kind)) if free => self.grab = Some(Grab { kind, time: 0.0 }),
            (None, _) => {}
        }
    }

    fn land(&mut self, normal: Vec3, air_time: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        let n = normal.normalize_or(Vec3::UP);
        // How the board meets the surface: tilted against it, and turned against the travel along it.
        let tilt = libm::acosf(self.normal.dot(n).clamp(-1.0, 1.0));
        let vt = self.vel - n * self.vel.dot(n);
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
        let off = atan2f(axis.cross(travel).dot(n), axis.dot(travel).clamp(-1.0, 1.0));
        let backwards = off.abs() > PI / 2.0;
        let crooked_signed = if backwards { wrap_angle(off + PI) } else { off };
        let crooked = crooked_signed.abs();
        let trick_done = self.trick.is_none_or(|t| t.progress() >= 1.0);
        // Still holding a grab when the wheels touch down also takes you out.
        let reason = if !trick_done {
            Some("landed before the trick finished")
        } else if self.grab.is_some() {
            Some("still holding the grab")
        } else if tilt > LAND_TILT {
            Some("landed off-axis (board not flat on the surface)")
        } else if crooked > t.land_angle.eval(speed) {
            Some("landed too crooked for the speed")
        } else if crooked > SIDEWAYS_BAIL && speed > SIDEWAYS_SPEED {
            Some("landed sideways")
        } else if impact > t.land_max_impact {
            Some("hit the ground too hard")
        } else {
            None
        };
        self.last_landing = Some(LandingCheck {
            crooked,
            impact,
            speed,
            limit: t.land_angle.eval(speed),
        });
        self.normal = n;
        if let Some(r) = reason {
            self.vel = vt;
            self.bail(r, events);
            return;
        }
        let side = sinf(crooked);
        let kind = if side > STRAIGHT_LANDING {
            LandKind::Sketchy {
                frontside: crooked_signed * self.frontside_sign(backwards) > 0.0,
            }
        } else if impact > HEAVY_IMPACT {
            LandKind::Heavy
        } else {
            LandKind::Clean
        };
        self.landing = Some(Landing {
            kind,
            time: 0.0,
            crooked,
            impact,
        });
        self.fwd = if backwards { -travel } else { travel };
        // A crooked board scrubs the sideways part of the speed.
        let keep = cosf(crooked)
            * if matches!(kind, LandKind::Sketchy { .. }) {
                0.9
            } else {
                0.97
            };
        self.speed = speed * keep * if backwards { -1.0 } else { 1.0 };
        self.slip = 0.0;
        self.vn = 0.0;
        self.gap_time = 0.0;
        self.lead_rise = 0.0;
        self.contact_n = n;
        self.truck_ramp = 0.0;
        self.pump = Pump::default();
        self.vel = self.fwd * self.speed;
        self.fakie = backwards;
        self.fakie_timer = 0.0;
        // Half-turns spun with the body, named frontside or backside for the rider's stance.
        let halves = libm::roundf(self.spin / PI) as i32;
        let grab = self.grabbed.take();
        let label = self.trick_label();
        let fs = (halves as f32) * self.frontside_sign(self.takeoff.fakie) > 0.0;
        if let Some(name) = landed_name(
            self.stance_prefix(),
            label.as_ref().map(|l| l.0.as_str()),
            halves,
            fs,
            grab.map(|g| g.kind),
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

/// The unit vector `from` turned towards `to` by at most `max` radians.
fn turn_towards(from: Vec3, to: Vec3, max: f32) -> Vec3 {
    let d = from.dot(to).clamp(-1.0, 1.0);
    let angle = libm::acosf(d);
    if angle <= max || angle < 1e-5 {
        return to;
    }
    let axis = from.cross(to);
    if axis.length() < 1e-6 {
        return from;
    }
    rotate(from, axis.normalize_or(Vec3::UP), max)
}

fn approach(v: f32, target: f32, by: f32) -> f32 {
    if v > target {
        (v - by).max(target)
    } else {
        (v + by).min(target)
    }
}

/// "FAKIE FS 360 KICKFLIP INDY" style names. `None` for nothing worth naming (rolled off a lip, no spin,
/// no grab).
fn landed_name(
    stance: &str,
    trick: Option<&str>,
    halves: i32,
    frontside: bool,
    grab: Option<GrabKind>,
) -> Option<String> {
    use core::fmt::Write;
    let mut s = String::from(stance);
    if halves != 0 {
        let dir = if frontside { "FS" } else { "BS" };
        let _ = write!(s, "{dir} {} ", halves.unsigned_abs() * 180);
    }
    match (trick, grab) {
        (Some(d), Some(g)) => {
            let _ = write!(s, "{d} {}", g.name());
        }
        (Some(d), None) => s.push_str(d),
        (None, Some(g)) => s.push_str(g.name()),
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

    fn steps(s: &mut Skater, park: &Park, pad: &Pad, flick: Option<Flick>, n: usize) -> Vec<Event> {
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

    #[test]
    fn landing_sideways_bails_and_respawns() {
        let park = flat();
        let mut s = spawn(&park);
        s.speed = 4.0;
        let mut ev = steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
        s.set_spin(PI / 2.0);
        ev.extend(steps(&mut s, &park, &Pad::default(), None, 80));
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
        let ev = steps(&mut s, &park, &Pad::default(), None, 150);
        assert!(ev.contains(&Event::Respawned));
        assert_eq!(s.stance, Stance::Goofy);
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
        ev.extend(steps(&mut s, &park, &Pad::default(), None, 30));
        assert_eq!(landed(&ev), ["REVERT"], "{ev:?}");
        assert!(!s.fakie && s.speed > 0.0);
    }

    #[test]
    fn slightly_crooked_landing_is_sketchy() {
        let park = flat();
        let land = |spin: f32| {
            let mut s = spawn(&park);
            s.speed = 5.0;
            let mut ev = steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
            s.set_spin(spin);
            ev.extend(steps(&mut s, &park, &Pad::default(), None, 100));
            let p = ev.iter().find_map(|e| match e {
                Event::Landed { name, points } => Some((name.clone(), *points)),
                _ => None,
            });
            (s, p.expect("landed"))
        };
        let (s, (name, points)) = land(25.0 * PI / 180.0);
        assert_eq!(name, "SKETCHY OLLIE");
        assert!(matches!(
            s.landing.map(|l| l.kind),
            Some(LandKind::Sketchy { .. })
        ));
        let (clean, (_, full)) = land(0.0);
        assert_eq!(points, full / 2);
        assert!(s.speed < clean.speed);
        assert_eq!(clean.landing.map(|l| l.kind), Some(LandKind::Clean));
    }

    #[test]
    fn big_drops_land_heavy_and_bigger_ones_bail() {
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
        let (s, _) = drop(3.0);
        assert_eq!(s.landing.map(|l| l.kind), Some(LandKind::Heavy));
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
    fn grabs_follow_the_hands_and_landing_while_grabbing_bails() {
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
        let ev = steps(&mut s, &park, &grab(1.0), None, 80);
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
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

    /// Flat ground with a 0.5 m high rail along +Z from z = 0 to z = `len`.
    fn rail_park(len: f32) -> Park {
        let mut p = flat();
        p.spawn = Vec3 {
            x: 0.0,
            y: 0.0,
            z: -3.0,
        };
        p.rails = vec![crate::rails::Rail::new(vec![
            Vec3 {
                x: 0.0,
                y: 0.5,
                z: 0.0,
            },
            Vec3 {
                x: 0.0,
                y: 0.5,
                z: len,
            },
        ])];
        p
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
        let (mut s, _) = ollie_at_rail(&park, 0.0);
        assert_eq!(s.mode, Mode::Grind);
        let g = s.grind.clone().unwrap();
        assert_eq!(g.kind, GrindKind::FiftyFifty);
        assert!(g.dir > 0.0 && (s.pos.y - 0.5).abs() < 1e-5);
        let ev = steps(&mut s, &park, &Pad::default(), None, 120);
        assert_eq!(landed(&ev), ["OLLIE BS 50-50"], "{ev:?}");
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        assert_eq!(s.mode, Mode::Rolling);
        assert!(s.pos.z > 4.0);
    }

    #[test]
    fn board_across_the_rail_is_a_boardslide_and_a_flick_pops_off() {
        let park = rail_park(20.0);
        let (mut s, _) = ollie_at_rail(&park, PI / 2.0);
        assert_eq!(
            s.grind.as_ref().map(|g| g.kind),
            Some(GrindKind::Boardslide)
        );
        steps(&mut s, &park, &Pad::default(), None, 20);
        assert_eq!(s.mode, Mode::Grind);
        let ev = steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 1);
        assert!(
            matches!(&ev[..], [Event::Landed { name, .. }, Event::Popped(_)] if name == "OLLIE BS BOARDSLIDE"),
            "{ev:?}"
        );
        assert!(s.is_airborne() && s.vel.y > 0.0);
        let ev = steps(&mut s, &park, &Pad::default(), None, 90);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
    }

    #[test]
    fn an_angled_board_locks_into_an_angled_grind() {
        let park = rail_park(10.0);
        let (s, _) = ollie_at_rail(&park, 45.0 * PI / 180.0);
        let kind = s.grind.as_ref().map(|g| g.kind);
        assert!(
            matches!(
                kind,
                Some(GrindKind::Crooked | GrindKind::Willy | GrindKind::Feeble | GrindKind::Smith)
            ),
            "{kind:?}"
        );
    }

    #[test]
    fn grinds_come_from_how_the_board_meets_the_rail() {
        use GrindKind::*;
        let deg = |d: f32| d * PI / 180.0;
        assert_eq!(
            GrindKind::from_contact(deg(5.0), 0.0, true, 0.0),
            FiftyFifty
        );
        assert_eq!(GrindKind::from_contact(deg(5.0), 0.0, true, 0.6), FiveO);
        assert_eq!(
            GrindKind::from_contact(deg(5.0), 0.0, true, -0.6),
            Nosegrind
        );
        assert_eq!(GrindKind::from_contact(deg(40.0), 0.2, true, 0.0), Crooked);
        assert_eq!(GrindKind::from_contact(deg(40.0), -0.2, true, 0.0), Feeble);
        assert_eq!(GrindKind::from_contact(deg(40.0), -0.2, false, 0.0), Smith);
        assert_eq!(
            GrindKind::from_contact(deg(85.0), 0.0, true, 0.0),
            Boardslide
        );
        assert_eq!(
            GrindKind::from_contact(deg(85.0), 0.0, false, 0.0),
            Lipslide
        );
        assert_eq!(
            GrindKind::from_contact(deg(85.0), 0.3, true, 0.0),
            Noseslide
        );
        assert_eq!(
            GrindKind::from_contact(deg(85.0), -0.3, true, 0.0),
            Tailslide
        );
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
    fn landing_tilted_against_a_steep_face_bails() {
        let park = quarter_pipe();
        let mut s = spawn(&park);
        // Just above the face, the board tipped the other way: no time to turn it flat.
        s.pos = v3(9.5, 2.1, -7.0);
        s.normal = rotate(Vec3::UP, v3(0.0, 0.0, 1.0), -50.0 * PI / 180.0);
        s.take_off(None);
        let (ev, _) = run(&mut s, &park, 90);
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
        assert_eq!(
            s.bail_reason,
            Some("landed off-axis (board not flat on the surface)")
        );
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
        // Rolling backwards the trucks turn the board the other way.
        let mut s = spawn(&park);
        let pad = Pad {
            stick: [1.0, 0.0],
            ..Default::default()
        };
        for _ in 0..30 {
            s.speed = -2.0;
            steps(&mut s, &park, &pad, None, 1);
        }
        assert!(s.heading > 0.05, "{}", s.heading);
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
}
