//! Board physics. The board has a full orientation (its nose direction on the surface and the surface's
//! normal) and a signed speed along its nose. On the ground it follows whatever is under its trucks, found
//! by casting a ray down from each truck: flat ground, banks, curved transitions and vertical walls alike,
//! with gravity acting along the surface, so a quarter pipe slows you down, sends you back fakie or out over
//! the coping. The trucks turn the board about the surface normal. In the air the board keeps its attitude
//! and eases it towards the surface it is going to land on. Deterministic. Skate 3's riding rules on top: stance (goofy or regular, switch,
//! fakie), pushing that grows with how long you hold the button, foot, tail and nose brakes, powerslides,
//! deck-tilt steering, flick tricks whose strength sets the pop, air spin that rewards a wind-up, and
//! landing rules. The numbers come from [`Tuning`] (Skate 3's own when the player's disc supplied them).
//!
//! Conventions: `heading` is where the board's nose points (in the rider's frame: the end the rider's
//! front foot is on), `speed` is along it, negative when rolling tail first (fakie). Riding switch means
//! the rider stands with the other foot forward; the drawn skater is then mirrored.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::flick::FlickOut;
use crate::gesture::{self, Flick, Group, Trick};
use crate::input::{Pad, btn};
use crate::math::{PI, Vec3, atan2f, clamp, cosf, sinf, smooth, sqrtf, v3, wrap_angle, yaw_dir};
#[cfg(test)]
use crate::park::Park;
use crate::park::Terrain;
use crate::tuning::Tuning;

/// Highest ledge you can roll up without popping.
pub const STEP: f32 = 0.12;
/// Distance from the board's middle to each truck (m).
const AXLE: f32 = 0.26;
/// The trucks look for the surface from this far above the board, and this far below it.
const PROBE_UP: f32 = 0.35;
const PROBE_DOWN: f32 = 0.18;
/// Walls are looked for this far above the board (anything lower is rolled up onto).
const WALL_RAY: f32 = STEP + 0.02;
/// A surface turned further than this from the one under the board (cosine) is a wall, not the next bit
/// of a curve.
const WALL_COS: f32 = 0.64;
/// Rolling into a wall faster than this (m/s, into it) is a bail; in the air, the second.
const WALL_BAIL: f32 = 6.0;
const AIR_WALL_BAIL: f32 = 5.5;
/// Landing with the board tilted further than this from the surface (rad) is a bail.
const LAND_TILT: f32 = 40.0 * PI / 180.0;
/// How fast the board turns towards the surface it will land on (rad/s).
const AIR_TILT_RATE: f32 = 3.0;
/// Riding across a face steeper than this (normal y) is a wallride: gravity is held off for a moment.
const WALL_STEEP: f32 = 0.5;
const WALLRIDE_HOLD: f32 = 0.35;
const WALLRIDE_FADE: f32 = 0.55;
const WALLRIDE_POINTS: u32 = 250;
/// Pumping: extending out of a crouch on a transition adds speed (m/s for a full crouch, on a 45-degree
/// face). Skate 3 pumps through its physics; this is flatground's stand-in.
const PUMP_GAIN: f32 = 2.2;
/// Turn rate (rad/s) for a deck tilt of 1. (Skate 3 turns through its truck physics; this is
/// flatground's stand-in.)
const TURN_RATE: f32 = 4.2;
/// Turning on two wheels is gentler.
const MANUAL_TURN: f32 = 0.5;
/// Below this speed a hard sideways stick kick-turns on the spot.
const KICKTURN_SPEED: f32 = 1.0;
const KICKTURN_RATE: f32 = 4.5;
const KICKTURN_STICK: f32 = 0.75;
/// Rolling tail first faster than this is fakie at once; above `FAKIE_SLOW` after `FAKIE_TIME`.
const FAKIE_FAST: f32 = 1.0;
const FAKIE_SLOW: f32 = 0.5;
const FAKIE_TIME: f32 = 0.2;
/// When the pushing foot is on the ground, as a share of the push cycle.
const PUSH_CONTACT: (f32, f32) = (0.18, 0.7);
/// Turning round from fakie to push switch takes this long (the push waits for it).
const SWITCH_TURN_TIME: f32 = 0.15;
/// Powerslide: least speed to start one, how sideways the board turns (rad), and how fast it scrubs speed.
const SLIDE_MIN_SPEED: f32 = 2.0;
const SLIDE_ANGLE: f32 = 1.35;
const SLIDE_DECEL: f32 = 4.5;
/// Down on the ground after a bail before respawning: long enough for the fall and get-up clips.
const BAIL_TIME: f32 = 2.2;
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
const MANUAL_MIN: f32 = 0.25;
const MANUAL_BRAKE: f32 = 0.9;
const MANUAL_ENGAGE: f32 = 0.12;
const MANUAL_SWEET: f32 = 0.55;
const MANUAL_MIN_TIME: f32 = 0.5;
const MANUAL_POINTS_PER_S: f32 = 120.0;
/// Analog trigger past this holds a grab (Skate 3: any pressure).
const GRAB_TRIGGER: f32 = 0.15;
const GRAB_POINTS: u32 = 200;
const GRAB_POINTS_PER_S: f32 = 150.0;
/// Both push buttons pressed within this of each other wind up a hippie jump.
const HIPPY_CHORD: f32 = 0.08;
const HIPPY_MAX_WINDUP: f32 = 0.6;
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
const GRIND_MIN_ENTRY: f32 = 1.5;
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
    /// On a rail (details in `Skater::grind`).
    Grind,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manual {
    /// On the front wheels.
    pub nose: bool,
    /// -1..1: 0 is balanced; reaching -1 or 1 drops the manual.
    pub balance: f32,
    pub time: f32,
    /// 0..1: dragging the tail (or nose) to brake.
    pub brake: f32,
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

/// A push in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Push {
    /// Seconds since this push started (it waits for a switch turn first, then runs one cycle).
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
}

impl Push {
    /// Seconds into the push itself (after any switch turn).
    pub fn time(&self) -> f32 {
        (self.t - self.delay).max(0.0)
    }
}

/// Braking with the foot (X).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootBrake {
    pub time: f32,
    /// Let go: the foot comes back up (seconds since).
    pub released: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Powerslide {
    /// +1 sliding with the board turned left of travel, -1 right.
    pub side: f32,
    /// 0..1 how far the board has turned sideways.
    pub amount: f32,
    pub time: f32,
    /// Let go: the board straightens out.
    pub releasing: bool,
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
    pub stance: Stance,
    pub pos: Vec3,
    /// Where the board's nose points: a unit vector along the surface (in the air, the board's own nose).
    pub fwd: Vec3,
    /// The nose's direction seen from above (yaw), kept in step with `fwd`.
    pub heading: f32,
    /// Speed along `heading` while rolling (negative: tail first).
    pub speed: f32,
    /// Velocity while in the air.
    pub vel: Vec3,
    pub normal: Vec3,
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
    takeoff: Takeoff,
    pub push: Option<Push>,
    /// Steering is weaker while pushing (1 = full).
    push_steer: f32,
    /// Smoothed turn input, -1..1 (drawn: the rider leans into turns).
    pub steer: f32,
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
    /// Riding across a steep face: seconds on it, and how fast it is sliding down (m/s).
    pub wallride: Option<(f32, f32)>,
    /// The surface the board is expected to land on (its normal), refreshed every few frames in the air.
    land_up: Vec3,
    predict_in: u32,
    /// Crouch on the last frame (for pumping).
    last_crouch: f32,
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
    /// -1 left, +1 right, 0 none.
    pub slide: f32,
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
            stance,
            pos,
            heading,
            speed: 0.0,
            vel: Vec3::ZERO,
            normal: Vec3::UP,
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
            takeoff: Takeoff::default(),
            push: None,
            push_steer: 1.0,
            steer: 0.0,
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
            last_crouch: 0.0,
            grind_stick: 0.0,
            fwd: yaw_dir(heading),
        }
    }

    /// Back to the spawn point, keeping the tuning and stance.
    pub fn respawn(&mut self, park: &dyn Terrain) {
        *self = Skater::spawn_with(park, self.tuning, self.stance);
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
            y += s.side * s.amount * SLIDE_ANGLE;
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
        tangent(self.fwd, self.normal)
    }

    /// Advances one fixed step.
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
            Mode::Bailed { time } => {
                let time = time + dt;
                let h = self.vel.horizontal() * (1.0 - smooth(3.0, dt));
                self.vel = Vec3 { y: 0.0, ..h };
                self.try_move(park, self.vel * dt);
                self.pos.y = park
                    .ground(self.pos.x, self.pos.z, self.pos.y + STEP)
                    .height;
                if time >= BAIL_TIME {
                    self.respawn(park);
                    events.push(Event::Respawned);
                } else {
                    self.mode = Mode::Bailed { time };
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
        self.update_manual(flick, dt, events);
        let st = steering(pad.stick);

        // Fakie: a moment of rolling tail first (not while a revert is turning the board).
        if self.revert.is_none() {
            let back = -self.speed;
            if back > FAKIE_FAST {
                self.fakie = true;
            } else if back > FAKIE_SLOW {
                self.fakie_timer += dt;
                if self.fakie_timer > FAKIE_TIME {
                    self.fakie = true;
                }
            } else {
                self.fakie_timer = 0.0;
                if self.speed >= 0.0 {
                    self.fakie = false;
                }
            }
        }

        // Revert: a powerslide flick just after landing spins the board half round.
        if let Some(r) = &mut self.revert {
            r.time += dt;
            if r.time >= REVERT_TIME {
                self.revert = None;
                self.fwd = -self.fwd;
                self.speed = -self.speed;
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

        // Powerslide: main stick fully back and to one side.
        let sliding_input = st.slide != 0.0 && self.manual.is_none() && self.revert.is_none();
        match &mut self.slide {
            Some(s) if sliding_input && !s.releasing => {
                s.time += dt;
                s.amount = (s.amount + dt * 5.0).min(1.0);
                self.speed = approach(self.speed, 0.0, SLIDE_DECEL * s.amount * dt);
                if self.speed.abs() < 0.3 {
                    s.releasing = true;
                }
            }
            Some(s) => {
                s.releasing = true;
                s.amount -= dt * 4.0;
                if s.amount <= 0.0 {
                    self.slide = None;
                }
            }
            None if sliding_input && self.speed.abs() > SLIDE_MIN_SPEED => {
                self.slide = Some(Powerslide {
                    side: st.slide,
                    amount: 0.0,
                    time: 0.0,
                    releasing: false,
                });
                self.push = None;
            }
            None => {}
        }

        // Kick turn: slow, stick hard to the side.
        let kick = self.speed.abs() < KICKTURN_SPEED
            && pad.stick[0].abs() > KICKTURN_STICK
            && self.manual.is_none()
            && self.slide.is_none()
            && self.revert.is_none()
            && self
                .push
                .is_none_or(|p| p.t > self.tuning.push_cycle * PUSH_CONTACT.1);
        if kick {
            let dir = -pad.stick[0].signum();
            let time = self.kickturn.map_or(0.0, |k| k.1) + dt;
            self.kickturn = Some((dir, time));
            self.fwd = rotate(self.fwd, self.normal, dir * KICKTURN_RATE * dt);
        } else {
            self.kickturn = None;
        }

        // Carving: Skate 3's deck tilt from the stick, eased in.
        let follow = 1.0 - libm::powf(1.0 - t.steer_follow.clamp(0.0, 1.0), ticks);
        let turn_in = if self.slide.is_some() || kick {
            0.0
        } else {
            st.turn
        };
        let turn_in = if st.hard != 0.0 {
            turn_in.signum()
        } else {
            turn_in
        };
        self.steer += (turn_in - self.steer) * follow;
        self.push_steer = if self.push.is_some() {
            (self.push_steer - t.push_steer_drop * ticks).max(t.push_steer_min)
        } else {
            (self.push_steer + t.push_steer_rise * ticks).min(1.0)
        };
        let tilt = t.steer_tilt(self.steer, self.speed, st.hard, self.push_steer);
        let manual = if self.manual.is_some() {
            MANUAL_TURN
        } else {
            1.0
        };
        let roll_k = clamp(self.speed.abs() / 0.6, 0.0, 1.0);
        self.fwd = rotate(
            self.fwd,
            self.normal,
            -tilt * TURN_RATE * manual * roll_k * dt,
        );

        // Pushing (A: the usual push, B: mongo), hippie jump (both), foot brake (X).
        self.update_hippy(pad, dt, events);
        if self.mode != Mode::Rolling {
            return;
        }
        self.update_push(pad, dt, events);
        self.update_brake(pad, dt);

        // Slope pulls you downhill (capped); rolling resistance always slows you, more so when the player
        // has left the controls alone for a while, and on two wheels.
        let fwd = self.surface_forward();
        let g = t.gravity;
        let tangential_gravity = Vec3::UP * -g + self.normal * (g * self.normal.y);
        let slope = clamp(
            tangential_gravity.dot(fwd),
            -t.max_slope_accel,
            t.max_slope_accel,
        );
        self.speed += slope * dt;
        let v = self.speed.abs();
        let mut friction = t.friction.eval(v);
        if self.idle > t.no_input_time {
            friction += t.friction_no_input.eval(v);
        }
        if self.manual.is_some() {
            friction += t.friction_manual.eval(v);
        }
        self.speed = approach(self.speed, 0.0, friction * dt);

        // Pumping: on a transition, standing up out of a crouch adds speed.
        let steepness = (1.0 - self.normal.y).clamp(0.0, 1.0);
        let extend = (self.last_crouch - self.crouch).max(0.0);
        self.last_crouch = self.crouch;
        if steepness > 0.02 && extend > 0.0 && self.manual.is_none() {
            let gain = PUMP_GAIN * extend * (steepness / 0.29).min(1.0);
            self.speed += if self.speed < 0.0 { -gain } else { gain };
        }

        if let Some(f) = flick.flick {
            let f = gesture::select(f, self.flick_group(), self.mirrored());
            if let Some((t, _)) = self.wallride.take() {
                // Popping off a wall: away from it as well as up.
                events.push(Event::Landed {
                    name: "WALLRIDE".into(),
                    points: WALLRIDE_POINTS + (t * 200.0) as u32,
                });
                self.vel = fwd * self.speed + self.normal * 2.5;
                self.pos += self.normal * 0.05;
                self.take_off(Some(f));
                self.vel.y += self.pop_speed(self.tuning.jump_height(flick.strength, 0.0)) * 0.6;
                events.push(Event::Popped(f));
                return;
            }
            self.pop(f, flick.strength, fwd, events);
            return;
        }

        self.follow_surface(park, dt, events);
        if self.mode == Mode::Rolling {
            self.update_wallride(dt, events);
        }
    }

    /// Looks for the surface under a point of the board: a ray along -`n` from a little above it.
    fn probe(park: &dyn Terrain, at: Vec3, n: Vec3) -> Option<crate::park::Hit> {
        park.raycast(at + n * PROBE_UP, -n, PROBE_UP + PROBE_DOWN)
    }

    /// One step along the surface: walls stop the board (or bail it), the trucks find the surface ahead
    /// and the board settles on it, and running out of surface (a lip, a ledge, the coping) launches it.
    fn follow_surface(&mut self, park: &dyn Terrain, dt: f32, events: &mut Vec<Event>) {
        let n = self.normal.normalize_or(Vec3::UP);
        let f = tangent(self.fwd, n);
        let motion = f * (self.speed * dt);
        let len = motion.length();
        if len > 1e-6 {
            let dir = motion * (1.0 / len);
            if let Some(h) = park.raycast(self.pos + n * WALL_RAY, dir, len + AXLE)
                && h.normal.dot(n) < WALL_COS
                && h.normal.dot(dir) < -0.2
            {
                let into = -(f * self.speed).dot(h.normal);
                if into > WALL_BAIL {
                    self.vel = f * self.speed * 0.3;
                    self.bail("ran into a wall", events);
                } else {
                    // Bump off it.
                    self.speed *= -0.15;
                    self.slide = None;
                    self.wallride = None;
                }
                return;
            }
        }
        let p = self.pos + motion;
        let front = Self::probe(park, p + f * AXLE, n);
        let back = Self::probe(park, p - f * AXLE, n);
        let (lead, trail) = if self.speed >= 0.0 {
            (front, back)
        } else {
            (back, front)
        };
        match (front, back) {
            (Some(a), Some(b)) => {
                let nn = (a.normal + b.normal).normalize_or(n);
                self.fwd = tangent((a.point - b.point).normalize_or(f), nn);
                self.normal = nn;
                self.pos = (a.point + b.point) * 0.5;
            }
            _ if lead.is_none() && (trail.is_none() || Self::probe(park, p, n).is_none()) => {
                // Rolled off an edge: a lip, a ledge, the coping. The board leaves the surface.
                self.pos = p + n * 0.05;
                self.vel = f * self.speed;
                self.wallride = None;
                self.take_off(None);
                events.push(Event::Launched);
                return;
            }
            _ => {
                // One truck on: keep its height and carry on.
                let h = lead.or(trail).expect("one truck has a surface");
                let off = if lead.is_some() { 1.0 } else { -1.0 } * self.speed.signum() * AXLE;
                let axle = p + f * off;
                self.pos = p + n * (h.point - axle).dot(n);
            }
        }
        // Hanging under an overhang, or stopped on a near-vertical face: fall off it.
        if self.normal.y < -0.2
            || (self.normal.y < 0.15 && self.speed.abs() < 0.8 && self.wallride.is_none())
        {
            self.vel = self.surface_forward() * self.speed + self.normal * 0.5;
            self.pos += self.normal * 0.02;
            self.take_off(None);
            events.push(Event::Launched);
        }
    }

    /// Riding across a steep face is a wallride: gravity is held off for a moment, then the board slides
    /// down the wall and jumps off it.
    fn update_wallride(&mut self, dt: f32, events: &mut Vec<Event>) {
        let across = self.surface_forward().y.abs() < 0.5;
        if !(self.normal.y < WALL_STEEP && across && self.speed.abs() > 1.0) {
            if let Some((t, _)) = self.wallride.take()
                && t > 0.2
            {
                events.push(Event::Landed {
                    name: "WALLRIDE".into(),
                    points: WALLRIDE_POINTS + (t * 200.0) as u32,
                });
            }
            return;
        }
        let (mut t, mut drop) = self.wallride.unwrap_or((0.0, 0.0));
        t += dt;
        let hold = 1.0 - ((t - WALLRIDE_HOLD) / WALLRIDE_FADE).clamp(0.0, 1.0);
        drop += self.tuning.gravity * (1.0 - hold) * dt;
        let down = tangent(-Vec3::UP, self.normal);
        self.pos += down * (drop * dt);
        self.manual = None;
        self.push = None;
        if hold <= 0.0 || drop > 2.5 {
            // Off the wall: a little hop away from it.
            events.push(Event::Landed {
                name: "WALLRIDE".into(),
                points: WALLRIDE_POINTS + (t * 200.0) as u32,
            });
            self.wallride = None;
            self.vel = self.surface_forward() * self.speed + self.normal * 2.0 + down * drop;
            self.pos += self.normal * 0.05;
            self.take_off(None);
            events.push(Event::Launched);
            return;
        }
        self.wallride = Some((t, drop));
    }

    fn update_push(&mut self, pad: &Pad, dt: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        let blocked = self.manual.is_some()
            || self.slide.is_some()
            || self.revert.is_some()
            || self.hippy.is_some()
            || self.brake.is_some_and(|b| b.released.is_none());
        if blocked {
            self.push = None;
            return;
        }
        let pressed = [btn::A, btn::B].into_iter().find(|b| pad.was_pressed(*b));
        if let Some(p) = &mut self.push {
            p.t += dt;
            if p.turning > 0.0 {
                p.turning -= dt;
                return;
            }
            let held = pad.is_held(p.button);
            let cycle = t.push_cycle.max(0.2);
            let pt = p.time();
            let k = pt / cycle;
            if k < PUSH_CONTACT.0 {
                p.base = self.speed.max(0.0);
            }
            if held && k < PUSH_CONTACT.1 {
                p.held += dt;
            }
            if (PUSH_CONTACT.0..PUSH_CONTACT.1).contains(&k) {
                // The push's speed is spread over the foot's time on the ground, as the push animation
                // drives it in Skate 3, and each push adds less the closer the board is to its top push
                // speed (flatground's stand-in for the animation's own foot speed).
                let room = (t.push_max_speed - p.base).max(0.0) / t.push_max_speed.max(0.01);
                let gain = t.push_gain(p.held, p.base) * room;
                let contact = (PUSH_CONTACT.1 - PUSH_CONTACT.0) * cycle;
                let target = (p.base + gain).min(t.push_max_speed);
                let step = (gain * dt / contact).min(t.push_tick_limit(self.speed) * dt * 60.0);
                let gap = (target - self.speed).max(0.0);
                self.speed += step.min(gap);
            }
            if pt >= cycle {
                // Holding the button keeps pushing; another press starts the next push.
                let next = if held { Some(p.button) } else { pressed };
                let count = p.count + 1;
                self.push = None;
                if let Some(b) = next {
                    self.start_push(b, count, events);
                }
            } else if let Some(b) = pressed
                && k >= PUSH_CONTACT.1
            {
                // Pressing again late in the cycle chains the next push straight away.
                let count = p.count + 1;
                self.start_push(b, count, events);
            }
        } else if let Some(b) = pressed {
            self.start_push(b, 0, events);
        }
    }

    fn start_push(&mut self, button: u32, count: u32, events: &mut Vec<Event>) {
        let mut turning = 0.0;
        if self.fakie || self.speed < -0.3 {
            // Skate 3 turns the rider round to push switch: same travel, the other foot forward.
            self.fwd = -self.fwd;
            self.speed = -self.speed;
            self.switch = !self.switch;
            self.fakie = false;
            self.fakie_timer = 0.0;
            turning = SWITCH_TURN_TIME;
            events.push(Event::Switched);
        }
        self.speed = self.speed.max(0.0);
        // Skate 3 pushes with the mongo set when riding switch (for a rider who pushes regular).
        let mongo = (button == btn::B) != self.switch;
        self.push = Some(Push {
            t: 0.0,
            held: 0.0,
            mongo,
            base: self.speed,
            count,
            button,
            turning,
            delay: turning,
        });
    }

    fn update_brake(&mut self, pad: &Pad, dt: f32) {
        let t = self.tuning;
        let can = self.manual.is_none() && self.slide.is_none() && self.revert.is_none();
        let held = pad.is_held(btn::X) && can;
        match &mut self.brake {
            Some(b) if held && b.released.is_none() => {
                b.time += dt;
                self.push = None;
                self.speed = approach(self.speed, 0.0, t.foot_brake * t.brake_scale * dt);
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
    }

    /// Both push buttons together wind up a hippie jump; letting go jumps off the board and back on.
    fn update_hippy(&mut self, pad: &Pad, dt: f32, events: &mut Vec<Event>) {
        let both = pad.is_held(btn::A) && pad.is_held(btn::B);
        match self.hippy {
            Some(t) if both => self.hippy = Some(t + dt),
            Some(t) => {
                self.hippy = None;
                let k = (t / HIPPY_MAX_WINDUP).clamp(0.0, 1.0);
                let tu = self.tuning;
                let h = tu.hippy_low + (tu.hippy_high - tu.hippy_low) * k;
                let up = sqrtf((2.0 * tu.gravity * (h - tu.jump_floor)).max(0.0));
                let fwd = self.surface_forward();
                self.vel = fwd * self.speed + Vec3::UP * up;
                self.pos += Vec3::UP * 0.02;
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
    /// stick past the manual zone drags the tail (or nose) to brake.
    fn update_manual(&mut self, flick: &FlickOut, dt: f32, events: &mut Vec<Event>) {
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
                    balance: 0.0,
                    time: 0.0,
                    brake: 0.0,
                });
                self.push = None;
                self.brake = None;
            }
            return;
        };
        // How hard the stick leans towards the manual's wheels.
        let toward = if m.nose { -y } else { y };
        if toward < MANUAL_MIN || !vertical {
            self.end_manual(events);
            return;
        }
        m.time += dt;
        m.brake = ((len - MANUAL_BRAKE) * 10.0).clamp(0.0, 1.0);
        let push = ((toward - MANUAL_MIN) / (MANUAL_BRAKE - MANUAL_MIN)).min(1.0);
        // Unstable like a real manual: any tilt grows, the stick past its sweet spot tips you further
        // over, and a steady wobble keeps you correcting.
        m.balance +=
            (m.balance * 1.6 + (push - MANUAL_SWEET) * 2.0 + 0.5 * sinf(m.time * 2.3)) * dt;
        if m.brake > 0.0 {
            let t = self.tuning;
            self.speed = approach(self.speed, 0.0, t.tail_brake * t.brake_scale * m.brake * dt);
        }
        if m.balance.abs() >= 1.0 {
            self.manual = None;
            self.manual_lock = true;
            self.speed *= 0.85;
            events.push(Event::ManualLost);
        }
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

    fn pop(&mut self, f: Flick, strength: f32, fwd: Vec3, events: &mut Vec<Event>) {
        self.end_manual(events);
        let h = self.tuning.jump_height(strength, self.speed);
        self.vel = fwd * self.speed + self.normal * self.pop_speed(h);
        self.pos += self.normal * 0.02;
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
                // A wall: slide along it, or bail if it was hit hard.
                let into = -self.vel.dot(h.normal);
                if into > AIR_WALL_BAIL {
                    self.pos = h.point + h.normal * 0.05;
                    self.vel += h.normal * (into * 0.8);
                    self.bail("hit a wall in the air", events);
                    return;
                }
                self.vel += h.normal * (into * 1.1);
                self.pos = h.point + h.normal * 0.02;
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
    fn try_grind(&mut self, park: &dyn Terrain) -> bool {
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
            let along = vh.dot(dh);
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
            self.spin_rate = 0.0;
            self.pos = p;
            self.grind = Some(Grind {
                rail: i,
                s: n.s,
                dir,
                speed: along.abs().max(GRIND_MIN_ENTRY),
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
        self.grind = None;
        self.regrind_block = Some((g.rail, 0.4));
        self.vel = travel * g.speed.max(0.0);
        self.pos += Vec3::UP * 0.02;
        // Fakie onto the rail (board backwards): still fakie leaving it.
        self.fakie = backwards;
        match flick.flick {
            Some(f) => {
                let f = gesture::select(f, Group::Square, self.mirrored());
                let h = self.tuning.jump_height(flick.strength, g.speed);
                self.vel += Vec3::UP * self.pop_speed(h);
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
        events.push(Event::Bailed);
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
        self.vel = Vec3::ZERO;
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
        let mut tap = Skater::spawn(&park);
        steps(&mut tap, &park, &press(btn::A), None, 1);
        steps(&mut tap, &park, &Pad::default(), None, 60);
        let mut held = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
    fn top_push_speed_takes_several_pushes() {
        let park = flat();
        for tuning in [Tuning::BUILT_IN, {
            // Skate 3's own push numbers have a stronger full push.
            let mut t = Tuning::BUILT_IN;
            t.push_max_speed = 8.5;
            t.push_dv = crate::tuning::Curve::new(&[(0.0, 0.75), (1.0, 4.5)]);
            t
        }] {
            let mut s = Skater::spawn_with(&park, tuning, Stance::Goofy);
            steps(&mut s, &park, &press(btn::A), None, 1);
            let mut at = Vec::new();
            for _ in 0..5 {
                steps(&mut s, &park, &hold(btn::A), None, 40);
                at.push(s.speed);
            }
            let top = tuning.push_max_speed;
            // Two pushes in (about 1.3 s) it is still well short of the top...
            assert!(at[1] < top - 1.5, "{at:?}");
            // ...and keeps gaining a little less each time.
            assert!(
                at[4] > at[2] && at[4] - at[3] < at[2] - at[1] + 0.2,
                "{at:?}"
            );
        }
    }

    #[test]
    fn b_pushes_mongo_and_x_brakes() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        steps(&mut s, &park, &press(btn::B), None, 1);
        assert!(s.push.is_some_and(|p| p.mongo));
        steps(&mut s, &park, &hold(btn::B), None, 120);
        assert!(s.speed > 2.0);
        let v = s.speed;
        steps(&mut s, &park, &hold(btn::X), None, 30);
        assert!(s.brake.is_some());
        assert!(s.speed < v - 1.0, "foot brake: {v} -> {}", s.speed);
        steps(&mut s, &park, &hold(btn::X), None, 300);
        assert_eq!(s.speed, 0.0);
    }

    #[test]
    fn powerslide_scrubs_speed_and_turns_the_board() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = 6.0;
        let a = 30.0 * PI / 180.0;
        let pad = Pad {
            stick: [sinf(a), -cosf(a)],
            ..Default::default()
        };
        steps(&mut s, &park, &pad, None, 30);
        let sl = s.slide.expect("sliding");
        assert!(sl.amount > 0.9 && s.board_yaw_offset().abs() > 1.0);
        assert!(s.speed < 4.5, "{}", s.speed);
        steps(&mut s, &park, &Pad::default(), None, 30);
        assert!(s.slide.is_none());
    }

    #[test]
    fn ollie_lands_on_flat_and_a_quicker_flick_pops_higher() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = 5.0;
        let ev = steps(&mut s, &park, &Pad::default(), Some(Flick::OLLIE), 120);
        assert_eq!(ev[0], Event::Popped(Flick::OLLIE));
        assert_eq!(landed(&ev), ["OLLIE"], "{ev:?}");
        assert_eq!(s.mode, Mode::Rolling);
        assert!(s.pos.z > 3.0);

        let apex = |strength: f32| {
            let mut s = Skater::spawn(&park);
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
                let mut s = Skater::spawn(&park);
                s.speed = 4.0;
                let ev = steps(&mut s, &park, &Pad::default(), Some(f), 120);
                assert!(!ev.contains(&Event::Bailed), "{f:?}: {ev:?}");
                assert_eq!(landed(&ev).len(), 1, "{f:?}: {ev:?}");
            }
        }
        // Nollie names.
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
        s.speed = -0.7;
        steps(&mut s, &park, &Pad::default(), None, 5);
        assert!(!s.fakie);
        steps(&mut s, &park, &Pad::default(), None, 15);
        assert!(s.fakie);
    }

    #[test]
    fn revert_after_landing_fakie_rides_forward_again() {
        let park = flat();
        let mut s = Skater::spawn(&park);
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
            let mut s = Skater::spawn(&park);
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
            let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
            let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
        let mut s = Skater::spawn(&park);
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
    fn a_slow_wall_bump_bounces_and_a_fast_one_bails() {
        let park = quarter_pipe();
        let wall_run = |speed: f32| {
            let mut s = Skater::spawn(&park);
            s.pos = v3(-4.0, 0.0, -12.0);
            s.speed = speed;
            run(&mut s, &park, 120)
        };
        let (ev, _) = wall_run(3.0);
        assert!(!ev.contains(&Event::Bailed), "{ev:?}");
        let (ev, _) = wall_run(9.0);
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
    }

    #[test]
    fn landing_tilted_against_a_steep_face_bails() {
        let park = quarter_pipe();
        let mut s = Skater::spawn(&park);
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
    fn riding_across_the_vert_is_a_wallride_that_jumps_off() {
        let park = quarter_pipe();
        let mut s = Skater::spawn(&park);
        // On the vert, rolling along it.
        s.pos = v3(-3.0, 3.4, 3.0);
        s.normal = v3(0.0, 0.0, -1.0);
        s.fwd = v3(1.0, 0.0, 0.0);
        s.speed = 6.0;
        let (ev, _) = run(&mut s, &park, 90);
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::Landed { name, .. } if name == "WALLRIDE")),
            "{ev:?}"
        );
    }

    #[test]
    fn pumping_on_a_transition_adds_speed() {
        let park = quarter_pipe();
        let speed_after = |pump: bool| {
            let mut s = Skater::spawn(&park);
            s.pos = v3(0.0, 0.11, 0.8);
            s.normal = v3(0.0, 0.966, -0.259);
            s.fwd = v3(0.0, 0.259, 0.966);
            s.speed = 4.0;
            let mut ev = Vec::new();
            for i in 0..30 {
                let crouch = if pump && i < 12 { 1.0 } else { 0.0 };
                let f = FlickOut {
                    crouch,
                    stick: [0.0, if crouch > 0.0 { 1.0 } else { 0.0 }],
                    ..FlickOut::default()
                };
                s.step(&park, &Pad::default(), &f, DT, &mut ev);
            }
            (s.speed, s.pos.y)
        };
        let (plain, _) = speed_after(false);
        let (pumped, y) = speed_after(true);
        assert!(y > 0.2, "on the transition: {y}");
        assert!(pumped > plain + 0.3, "{pumped} vs {plain}");
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
