//! Board physics. Deliberately simple and deterministic: a point that rolls along the park's height
//! field with a heading, gets launched off lips and ollies, spins in the air and has to land with the
//! board lined up with where it is travelling.

use alloc::format;
use alloc::string::String;

use crate::flick::Flick;
use crate::input::{Pad, btn};
use crate::math::{PI, Vec3, atan2f, clamp, sinf, smooth, wrap_angle, yaw_dir};
use crate::park::Park;

/// Gravity in the air (a bit above 9.81 for a snappier feel).
pub const GRAVITY: f32 = 14.0;
/// Top speed you can reach by pushing on flat ground (m/s). Hills can take you faster.
pub const PUSH_MAX: f32 = 7.5;
const PUSH_KICK: f32 = 2.2;
const PUSH_COOLDOWN: f32 = 0.5;
const ROLL_FRICTION: f32 = 0.35;
const BRAKE: f32 = 6.0;
/// Vertical pop of an ollie (m/s).
pub const POP: f32 = 4.6;
/// Highest ledge you can roll up without popping.
pub const STEP: f32 = 0.12;
/// Board turn rate at full lock (rad/s).
const CARVE: f32 = 2.4;
/// Air spin rate at full lock (rad/s).
const AIR_SPIN: f32 = 7.0;
/// How far off (rad) the board can be from the travel direction and still land.
const LAND_TOLERANCE: f32 = 35.0 * PI / 180.0;
/// Down on the ground after a bail before respawning: long enough for the fall and get-up clips.
const BAIL_TIME: f32 = 2.2;
/// Landing further off than this (but still within `LAND_TOLERANCE`) is sketchy: slower, half points.
const SKETCHY_ANGLE: f32 = 20.0 * PI / 180.0;
/// Hitting the ground faster than this (m/s, downwards) is a heavy landing (a big drop).
const HEAVY_IMPACT: f32 = 8.0;
/// How long a landing is remembered (drawn: the landing clip).
const LANDING_SHOW: f32 = 1.5;
/// Main stick Y past this starts a manual (pulled down) or nose manual (pushed up) while rolling.
const MANUAL_START: f32 = 0.5;
/// Easing the stick back under this ends the manual (and scores it).
const MANUAL_HOLD: f32 = 0.3;
/// Where the stick balances a manual (0..1 of the way towards the wheels); further tips it over.
const MANUAL_SWEET: f32 = 0.75;
const MANUAL_MIN_TIME: f32 = 0.5;
const MANUAL_POINTS_PER_S: f32 = 120.0;
/// Analog trigger past this holds a grab.
const GRAB_TRIGGER: f32 = 0.5;
const GRAB_POINTS: u32 = 200;
const GRAB_POINTS_PER_S: f32 = 150.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrickDef {
    pub name: &'static str,
    pub points: u32,
    /// Seconds the trick animation takes; landing before it finishes is a bail.
    pub duration: f32,
    /// Board rolls around its long axis (kickflip +1, heelflip -1).
    pub flip_turns: f32,
    /// Board spins under the feet, in half turns (shove-it = 1).
    pub shove_halves: f32,
}

pub fn trick_def(f: Flick) -> TrickDef {
    let d = |name, points, duration, flip_turns, shove_halves| TrickDef {
        name,
        points,
        duration,
        flip_turns,
        shove_halves,
    };
    match f {
        Flick::Ollie => d("OLLIE", 100, 0.0, 0.0, 0.0),
        Flick::Nollie => d("NOLLIE", 120, 0.0, 0.0, 0.0),
        Flick::Kickflip => d("KICKFLIP", 300, 0.42, 1.0, 0.0),
        Flick::Heelflip => d("HEELFLIP", 300, 0.42, -1.0, 0.0),
        Flick::PopShoveIt => d("POP SHOVE-IT", 250, 0.38, 0.0, 1.0),
        Flick::FsShoveIt => d("FS SHOVE-IT", 250, 0.38, 0.0, -1.0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Rolling,
    Air { time: f32 },
    Bailed { time: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandKind {
    Clean,
    /// A big drop.
    Heavy,
    /// Board a bit crooked to the travel direction.
    Sketchy,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub kind: LandKind,
    /// Seconds since touching down.
    pub time: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrabKind {
    /// R trigger: frontside (toe-side) grab.
    Indy,
    /// L trigger: backside (heel-side) grab.
    Melon,
    /// Both triggers.
    Mute,
}

impl GrabKind {
    pub fn from_triggers(l: bool, r: bool) -> Option<GrabKind> {
        match (l, r) {
            (true, true) => Some(GrabKind::Mute),
            (true, false) => Some(GrabKind::Melon),
            (false, true) => Some(GrabKind::Indy),
            (false, false) => None,
        }
    }

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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveTrick {
    pub flick: Flick,
    pub def: TrickDef,
    pub t: f32,
}

impl ActiveTrick {
    /// 0..1 progress of the board animation.
    pub fn progress(&self) -> f32 {
        if self.def.duration <= 0.0 {
            1.0
        } else {
            (self.t / self.def.duration).min(1.0)
        }
    }
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skater {
    pub pos: Vec3,
    /// Direction of travel on the ground (yaw). The body faces sideways to this.
    pub heading: f32,
    /// Speed along `heading` while rolling (negative = rolling backwards).
    pub speed: f32,
    /// Velocity while in the air.
    pub vel: Vec3,
    pub normal: Vec3,
    pub mode: Mode,
    /// Extra yaw spun in the air, relative to `heading` at take-off.
    pub spin: f32,
    spin_rate: f32,
    pub trick: Option<ActiveTrick>,
    /// Riding with the other foot forward after a 180.
    pub fakie: bool,
    takeoff_fakie: bool,
    push_cooldown: f32,
    /// Seconds since the last push (drawn: plays the push animation).
    pub since_push: f32,
    /// 0..1, how far the skater is crouched (drawn; also used for the push animation later).
    pub crouch: f32,
    /// -1..1, the steering input, smoothed (drawn: the rider leans into turns).
    pub steer: f32,
    /// The last landing, for a moment after it (drawn).
    pub landing: Option<Landing>,
    pub manual: Option<Manual>,
    /// After tipping out of a manual, the stick has to come back before another starts.
    manual_lock: bool,
    /// The grab being held in the air.
    pub grab: Option<Grab>,
    /// The longest grab of this jump that was let go of (scored on landing).
    grabbed: Option<Grab>,
}

impl Skater {
    pub fn spawn(park: &Park) -> Skater {
        Skater {
            pos: park.spawn,
            heading: park.spawn_yaw,
            speed: 0.0,
            vel: Vec3::ZERO,
            normal: Vec3::UP,
            mode: Mode::Rolling,
            spin: 0.0,
            spin_rate: 0.0,
            trick: None,
            fakie: false,
            takeoff_fakie: false,
            push_cooldown: 0.0,
            since_push: f32::INFINITY,
            crouch: 0.0,
            steer: 0.0,
            landing: None,
            manual: None,
            manual_lock: false,
            grab: None,
            grabbed: None,
        }
    }

    pub fn is_airborne(&self) -> bool {
        matches!(self.mode, Mode::Air { .. })
    }

    /// Ground direction of travel, following the slope under the board.
    fn surface_forward(&self) -> Vec3 {
        let f = yaw_dir(self.heading);
        (f - self.normal * f.dot(self.normal)).normalize_or(f)
    }

    /// Advances one fixed step. `flick` is this frame's completed flick, `loaded` whether the flick
    /// stick is held back (crouch).
    pub fn step(
        &mut self,
        park: &Park,
        pad: &Pad,
        flick: Option<Flick>,
        loaded: bool,
        dt: f32,
        events: &mut alloc::vec::Vec<Event>,
    ) {
        self.crouch += ((if loaded { 1.0 } else { 0.0 }) - self.crouch) * smooth(18.0, dt);
        self.since_push += dt;
        let steer_in = if self.mode == Mode::Rolling {
            pad.stick[0]
        } else {
            0.0
        };
        self.steer += (steer_in - self.steer) * smooth(8.0, dt);
        if let Some(l) = &mut self.landing {
            l.time += dt;
            if l.time > LANDING_SHOW {
                self.landing = None;
            }
        }
        match self.mode {
            Mode::Rolling => self.roll(park, pad, flick, dt, events),
            Mode::Air { time } => self.fly(park, pad, time + dt, dt, events),
            Mode::Bailed { time } => {
                let time = time + dt;
                // Slide to a stop.
                let h = self.vel.horizontal() * (1.0 - smooth(3.0, dt));
                self.vel = Vec3 { y: 0.0, ..h };
                self.try_move(park, self.vel * dt);
                self.pos.y = park.surface(self.pos.x, self.pos.z).height;
                if time >= BAIL_TIME {
                    *self = Skater::spawn(park);
                    events.push(Event::Respawned);
                } else {
                    self.mode = Mode::Bailed { time };
                }
            }
        }
    }

    fn roll(
        &mut self,
        park: &Park,
        pad: &Pad,
        flick: Option<Flick>,
        dt: f32,
        events: &mut alloc::vec::Vec<Event>,
    ) {
        self.update_manual(pad, dt, events);
        // Carve: left stick X. Sharper at low speed, never zero so you can turn from a standstill.
        // Half as sharp on two wheels.
        let manual = if self.manual.is_some() { 0.5 } else { 1.0 };
        let lock = clamp(1.4 - self.speed.abs() * 0.08, 0.6, 1.4) * manual;
        self.heading = wrap_angle(self.heading - pad.stick[0] * CARVE * lock * dt);

        // Push / brake.
        self.push_cooldown = (self.push_cooldown - dt).max(0.0);
        if pad.was_pressed(btn::A) && self.push_cooldown == 0.0 && self.speed < PUSH_MAX {
            self.speed = (self.speed + PUSH_KICK).min(PUSH_MAX);
            self.push_cooldown = PUSH_COOLDOWN;
            self.since_push = 0.0;
        }
        if pad.is_held(btn::B) {
            self.speed = approach(self.speed, 0.0, BRAKE * dt);
        }

        // Slope pulls you downhill; rolling friction always slows you.
        let fwd = self.surface_forward();
        let tangential_gravity = Vec3::UP * -GRAVITY + self.normal * (GRAVITY * self.normal.y);
        self.speed += tangential_gravity.dot(fwd) * dt;
        self.speed = approach(self.speed, 0.0, ROLL_FRICTION * dt);

        if let Some(f) = flick {
            self.pop(f, fwd, events);
            return;
        }

        let delta = fwd * (self.speed * dt);
        let expected_y = self.pos.y + delta.y;
        if !self.try_move(park, delta) {
            self.speed *= -0.15; // bump off the wall
        }
        let s = park.surface(self.pos.x, self.pos.z);
        if s.height < expected_y - 0.06 {
            // The ground fell away faster than the board's path: off a lip or a ledge.
            self.pos.y = expected_y;
            self.vel = fwd * self.speed;
            self.take_off(None);
            events.push(Event::Launched);
        } else {
            self.pos.y = s.height;
            self.normal = s.normal;
        }
    }

    /// Moves horizontally by `delta`, refusing to go into anything more than a step up.
    /// Slides along a wall when only one axis is blocked. Returns false when fully blocked.
    fn try_move(&mut self, park: &Park, delta: Vec3) -> bool {
        let ok = |x: f32, z: f32| park.surface(x, z).height <= self.pos.y + STEP;
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

    /// Starts, balances and ends manuals from the main stick's Y while rolling.
    fn update_manual(&mut self, pad: &Pad, dt: f32, events: &mut alloc::vec::Vec<Event>) {
        let y = pad.stick[1];
        if y.abs() < MANUAL_HOLD {
            self.manual_lock = false;
        }
        let Some(m) = &mut self.manual else {
            if y.abs() > MANUAL_START && !self.manual_lock && self.speed.abs() > 0.5 {
                self.manual = Some(Manual {
                    nose: y > 0.0,
                    balance: 0.0,
                    time: 0.0,
                });
            }
            return;
        };
        // How hard the stick leans towards the manual's wheels (0..1).
        let push = if m.nose { y } else { -y };
        if push < MANUAL_HOLD {
            self.end_manual(events);
            return;
        }
        m.time += dt;
        // Unstable like a real manual: any tilt grows, the stick past its sweet spot tips you further
        // over, and a steady wobble keeps you correcting.
        m.balance +=
            (m.balance * 1.6 + (push - MANUAL_SWEET) * 2.0 + 0.5 * sinf(m.time * 2.3)) * dt;
        if m.balance.abs() >= 1.0 {
            self.manual = None;
            self.manual_lock = true;
            self.speed *= 0.85;
            events.push(Event::ManualLost);
        }
    }

    /// Ends a manual on purpose (stick released, or popped out of): scores it if it was long enough.
    fn end_manual(&mut self, events: &mut alloc::vec::Vec<Event>) {
        if let Some(m) = self.manual.take()
            && m.time >= MANUAL_MIN_TIME
        {
            events.push(Event::Landed {
                name: (if m.nose { "NOSE MANUAL" } else { "MANUAL" }).into(),
                points: (m.time * MANUAL_POINTS_PER_S) as u32,
            });
        }
    }

    fn pop(&mut self, f: Flick, fwd: Vec3, events: &mut alloc::vec::Vec<Event>) {
        self.end_manual(events);
        self.vel = fwd * self.speed + self.normal * POP;
        self.pos += self.normal * 0.02;
        self.take_off(Some(f));
        events.push(Event::Popped(f));
    }

    fn take_off(&mut self, f: Option<Flick>) {
        self.mode = Mode::Air { time: 0.0 };
        // Rolling off a ledge mid-manual just ends it (no points: it wasn't finished).
        self.manual = None;
        self.landing = None;
        self.grab = None;
        self.grabbed = None;
        self.spin = 0.0;
        self.spin_rate = 0.0;
        self.takeoff_fakie = self.fakie;
        self.trick = f.map(|f| ActiveTrick {
            flick: f,
            def: trick_def(f),
            t: 0.0,
        });
    }

    fn fly(
        &mut self,
        park: &Park,
        pad: &Pad,
        time: f32,
        dt: f32,
        events: &mut alloc::vec::Vec<Event>,
    ) {
        self.spin_rate += (-pad.stick[0] * AIR_SPIN - self.spin_rate) * smooth(10.0, dt);
        self.spin += self.spin_rate * dt;
        if let Some(t) = &mut self.trick {
            t.t += dt;
        }
        self.update_grab(pad, dt);
        self.vel.y -= GRAVITY * dt;
        self.pos.y += self.vel.y * dt;
        if !self.try_move(park, self.vel.horizontal() * dt) {
            // Hit a wall in the air: lose the horizontal speed.
            self.vel = Vec3 {
                x: 0.0,
                z: 0.0,
                ..self.vel
            };
        }
        // Ease the board back to level while airborne.
        self.normal = self
            .normal
            .lerp(Vec3::UP, smooth(4.0, dt))
            .normalize_or(Vec3::UP);

        let s = park.surface(self.pos.x, self.pos.z);
        if self.pos.y <= s.height && self.vel.y <= 0.0 {
            self.pos.y = s.height;
            self.land(s.normal, time, events);
        } else if self.pos.y < -5.0 {
            // Fell out of the world: start over.
            *self = Skater::spawn(park);
            events.push(Event::Respawned);
        } else {
            self.mode = Mode::Air { time };
        }
    }

    fn bail(&mut self, events: &mut alloc::vec::Vec<Event>) {
        self.mode = Mode::Bailed { time: 0.0 };
        self.trick = None;
        self.grab = None;
        self.grabbed = None;
        self.manual = None;
        self.landing = None;
        events.push(Event::Bailed);
    }

    /// Holds a grab while a trigger is down (once any flip has finished). Letting go keeps the longest
    /// grab of the jump for the score.
    fn update_grab(&mut self, pad: &Pad, dt: f32) {
        let held = GrabKind::from_triggers(pad.trig_l > GRAB_TRIGGER, pad.trig_r > GRAB_TRIGGER);
        let free = self.trick.is_none_or(|t| t.progress() >= 1.0);
        match (&mut self.grab, held) {
            (Some(g), Some(kind)) => {
                g.time += dt;
                // Adding the second trigger turns it into a mute grab.
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

    fn land(&mut self, normal: Vec3, air_time: f32, events: &mut alloc::vec::Vec<Event>) {
        let h = self.vel.horizontal();
        let travel = if h.length() > 0.5 {
            atan2f(h.x, h.z)
        } else {
            self.heading
        };
        let facing = wrap_angle(self.heading + self.spin);
        let off = wrap_angle(facing - travel).abs();
        let trick_done = self.trick.is_none_or(|t| t.progress() >= 1.0);
        let straight = off <= LAND_TOLERANCE;
        let switched = (PI - off) <= LAND_TOLERANCE;
        let impact = -self.vel.y;
        self.normal = normal;
        // Still holding a grab when the wheels touch down also takes you out.
        if !trick_done || !(straight || switched) || self.grab.is_some() {
            self.bail(events);
            return;
        }
        let crooked = if straight { off } else { PI - off };
        let kind = if crooked > SKETCHY_ANGLE {
            LandKind::Sketchy
        } else if impact > HEAVY_IMPACT {
            LandKind::Heavy
        } else {
            LandKind::Clean
        };
        self.landing = Some(Landing { kind, time: 0.0 });
        self.heading = travel;
        let keep = if kind == LandKind::Sketchy { 0.8 } else { 0.95 };
        self.speed = h.length() * keep;
        // Half-turns spun with the body. A shove-it alone turns the board, not the rider.
        let halves = libm::roundf(self.spin / PI) as i32;
        if halves.rem_euclid(2) == 1 {
            self.fakie = !self.fakie;
        }
        let grab = self.grabbed.take();
        if let Some(name) = landed_name(
            self.trick.map(|t| t.def),
            halves,
            self.takeoff_fakie,
            grab.map(|g| g.kind),
        ) {
            let base = self.trick.map_or(0, |t| t.def.points);
            let spin_bonus = halves.unsigned_abs() * 150;
            let grab_bonus = grab.map_or(0, |g| GRAB_POINTS + (g.time * GRAB_POINTS_PER_S) as u32);
            let air_bonus = 1.0 + air_time.min(2.0);
            let mut points = ((base + spin_bonus + grab_bonus) as f32 * air_bonus) as u32;
            let name = if kind == LandKind::Sketchy {
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
    }
}

fn approach(v: f32, target: f32, by: f32) -> f32 {
    if v > target {
        (v - by).max(target)
    } else {
        (v + by).min(target)
    }
}

/// "FAKIE 360 KICKFLIP INDY" style names. `None` for nothing worth naming (rolled off a lip, no spin, no
/// grab).
fn landed_name(
    def: Option<TrickDef>,
    halves: i32,
    fakie: bool,
    grab: Option<GrabKind>,
) -> Option<String> {
    use core::fmt::Write;
    let mut s = String::new();
    if fakie {
        s.push_str("FAKIE ");
    }
    if halves != 0 {
        let dir = if halves > 0 { "BS" } else { "FS" };
        let _ = write!(s, "{dir} {} ", halves.unsigned_abs() * 180);
    }
    match (def, grab) {
        (Some(d), Some(g)) => {
            let _ = write!(s, "{} {}", d.name, g.name());
        }
        (Some(d), None) => s.push_str(d.name),
        (None, Some(g)) => s.push_str(g.name()),
        (None, None) if halves != 0 => s.push_str("AIR"),
        (None, None) => return None,
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    const DT: f32 = 1.0 / 60.0;

    fn flat() -> Park {
        Park {
            shapes: Vec::new(),
            half_size: 50.0,
            wall_height: 2.0,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
        }
    }

    fn steps(s: &mut Skater, park: &Park, pad: &Pad, flick: Option<Flick>, n: usize) -> Vec<Event> {
        let mut ev = Vec::new();
        for i in 0..n {
            s.step(
                park,
                pad,
                if i == 0 { flick } else { None },
                false,
                DT,
                &mut ev,
            );
        }
        ev
    }

    #[test]
    fn pushing_caps_and_friction_slows() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        let push = Pad {
            held: btn::A,
            pressed: btn::A,
            ..Default::default()
        };
        for _ in 0..10 {
            steps(&mut s, &park, &push, None, 1);
            steps(&mut s, &park, &Pad::default(), None, 40);
        }
        assert!(s.speed > 5.0 && s.speed <= PUSH_MAX);
        assert!(s.pos.z > 5.0, "rolled forward along +z");
        let v = s.speed;
        steps(&mut s, &park, &Pad::default(), None, 60);
        assert!(s.speed < v);
    }

    #[test]
    fn ollie_lands_on_flat() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = 5.0;
        let ev = steps(&mut s, &park, &Pad::default(), Some(Flick::Ollie), 90);
        assert_eq!(ev[0], Event::Popped(Flick::Ollie));
        assert!(
            matches!(ev[1], Event::Landed { ref name, .. } if name == "OLLIE"),
            "{ev:?}"
        );
        assert_eq!(s.mode, Mode::Rolling);
        // Air time = 2 * POP / GRAVITY ~ 0.66 s at 5 m/s ~ 3.3 m.
        assert!(s.pos.z > 3.0);
    }

    #[test]
    fn kickflip_has_time_to_finish_on_flat() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = 4.0;
        let ev = steps(&mut s, &park, &Pad::default(), Some(Flick::Kickflip), 90);
        assert!(
            matches!(ev.last(), Some(Event::Landed { name, .. }) if name == "KICKFLIP"),
            "{ev:?}"
        );
    }

    #[test]
    fn landing_sideways_bails_and_respawns() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = 4.0;
        // Hold the spin for the whole jump: ~0.66 s * 7 rad/s lands well past 35 degrees but short of 180.
        let spin = Pad {
            stick: [0.35, 0.0],
            ..Default::default()
        };
        let ev = steps(&mut s, &park, &spin, Some(Flick::Ollie), 60);
        assert!(ev.contains(&Event::Bailed), "{ev:?}");
        let ev = steps(&mut s, &park, &Pad::default(), None, 120);
        assert!(ev.contains(&Event::Respawned));
    }

    #[test]
    fn half_spin_lands_fakie() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = 4.0;
        let mut ev = Vec::new();
        // Spin until about 180 degrees, then let go.
        s.step(
            &park,
            &Pad::default(),
            Some(Flick::Ollie),
            false,
            DT,
            &mut ev,
        );
        let spin = Pad {
            stick: [-1.0, 0.0],
            ..Default::default()
        };
        while s.is_airborne() && s.spin < PI - 0.35 {
            s.step(&park, &spin, None, false, DT, &mut ev);
        }
        for _ in 0..60 {
            s.step(&park, &Pad::default(), None, false, DT, &mut ev);
        }
        assert!(s.fakie, "{ev:?}");
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::Landed { name, .. } if name == "BS 180 OLLIE")),
            "{ev:?}"
        );
    }

    #[test]
    fn walls_stop_you() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = 6.0;
        steps(&mut s, &park, &Pad::default(), None, 600);
        assert!(s.pos.z <= park.half_size + 0.01);
    }

    /// Pops an ollie, steps once, then sets the air spin directly (no stick, so it stays put).
    fn ollie_with_spin(s: &mut Skater, park: &Park, spin: f32) -> Vec<Event> {
        let mut ev = steps(s, park, &Pad::default(), Some(Flick::Ollie), 1);
        s.spin = spin;
        ev.extend(steps(s, park, &Pad::default(), None, 80));
        ev
    }

    #[test]
    fn slightly_crooked_landing_is_sketchy() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = 5.0;
        let ev = ollie_with_spin(&mut s, &park, 25.0 * PI / 180.0);
        let Some(Event::Landed { name, points }) =
            ev.iter().find(|e| matches!(e, Event::Landed { .. }))
        else {
            panic!("{ev:?}");
        };
        assert_eq!(name, "SKETCHY OLLIE");
        // Half points, and slower than a clean landing keeps.
        let mut clean = Skater::spawn(&park);
        clean.speed = 5.0;
        let cev = ollie_with_spin(&mut clean, &park, 0.0);
        let Some(Event::Landed { points: full, .. }) =
            cev.iter().find(|e| matches!(e, Event::Landed { .. }))
        else {
            panic!("{cev:?}");
        };
        assert_eq!(*points, full / 2);
        assert!(s.speed < clean.speed);
        assert!(clean.landing.is_some());
        steps(&mut clean, &park, &Pad::default(), None, 100);
        assert!(clean.landing.is_none(), "forgotten after a while");
    }

    #[test]
    fn landing_kinds() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = 5.0;
        steps(&mut s, &park, &Pad::default(), Some(Flick::Ollie), 1);
        steps(&mut s, &park, &Pad::default(), None, 20);
        assert!(s.is_airborne());
        steps(&mut s, &park, &Pad::default(), None, 40);
        assert_eq!(s.landing.map(|l| l.kind), Some(LandKind::Clean));
        // Dropped from 3 m: ~9 m/s on impact.
        let mut s = Skater::spawn(&park);
        s.pos.y = 3.0;
        s.take_off(None);
        steps(&mut s, &park, &Pad::default(), None, 60);
        assert_eq!(s.landing.map(|l| l.kind), Some(LandKind::Heavy));
    }

    /// Holds a manual (main stick down) for `secs`, correcting the balance when `correct`.
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
            let pad = Pad {
                stick: [0.0, -push.clamp(MANUAL_HOLD + 0.01, 1.0)],
                ..Default::default()
            };
            s.step(park, &pad, None, false, DT, &mut ev);
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
        assert!(!m.nose && m.time > 1.9);
        // Let go: scored.
        let ev = steps(&mut s, &park, &Pad::default(), None, 1);
        assert!(
            matches!(&ev[..], [Event::Landed { name, points }] if name == "MANUAL" && *points >= 230),
            "{ev:?}"
        );
        assert!(s.manual.is_none());

        // Never correcting: the wobble grows until it tips over.
        let ev = hold_manual(&mut s, &park, 5.0, false);
        assert!(ev.contains(&Event::ManualLost), "{ev:?}");
        // Still holding the stick doesn't start another until it comes back.
        let pad = Pad {
            stick: [0.0, -0.8],
            ..Default::default()
        };
        if s.manual.is_none() {
            steps(&mut s, &park, &pad, None, 5);
            assert!(s.manual.is_none());
        }
        steps(&mut s, &park, &Pad::default(), None, 1);
        s.speed = 5.0;
        // Stick up: nose manual.
        let up = Pad {
            stick: [0.0, 0.8],
            ..Default::default()
        };
        steps(&mut s, &park, &up, None, 1);
        assert!(s.manual.is_some_and(|m| m.nose));
    }

    #[test]
    fn grabs_score_and_landing_while_grabbing_bails() {
        let park = flat();
        let grab = |trig_r: f32| Pad {
            trig_r,
            ..Default::default()
        };
        // Grab for ~0.3 s mid-air, let go, land.
        let mut s = Skater::spawn(&park);
        s.speed = 5.0;
        let mut ev = steps(&mut s, &park, &Pad::default(), Some(Flick::Ollie), 5);
        ev.extend(steps(&mut s, &park, &grab(1.0), None, 18));
        assert_eq!(s.grab.map(|g| g.kind), Some(GrabKind::Indy));
        ev.extend(steps(&mut s, &park, &grab(0.0), None, 60));
        let landed = ev.iter().find_map(|e| match e {
            Event::Landed { name, points } => Some((name.clone(), *points)),
            _ => None,
        });
        let (name, points) = landed.expect("landed");
        assert_eq!(name, "OLLIE INDY");
        assert!(points > 2 * 100, "grab bonus on top of the ollie: {points}");

        // Still holding it when the wheels touch down.
        let mut s = Skater::spawn(&park);
        s.speed = 5.0;
        steps(&mut s, &park, &Pad::default(), Some(Flick::Ollie), 5);
        let ev = steps(&mut s, &park, &grab(1.0), None, 60);
        assert!(ev.contains(&Event::Bailed), "{ev:?}");

        // No grabbing until the flip has finished.
        let mut s = Skater::spawn(&park);
        s.speed = 5.0;
        steps(&mut s, &park, &Pad::default(), Some(Flick::Kickflip), 1);
        steps(&mut s, &park, &grab(1.0), None, 3);
        assert!(s.grab.is_none());
        steps(&mut s, &park, &grab(1.0), None, 25);
        assert!(s.grab.is_some());
    }

    #[test]
    fn names() {
        let kf = trick_def(Flick::Kickflip);
        assert_eq!(
            landed_name(Some(kf), 0, false, None).as_deref(),
            Some("KICKFLIP")
        );
        assert_eq!(
            landed_name(Some(kf), -2, true, None).as_deref(),
            Some("FAKIE FS 360 KICKFLIP")
        );
        assert_eq!(
            landed_name(None, 1, false, None).as_deref(),
            Some("BS 180 AIR")
        );
        assert_eq!(landed_name(None, 0, false, None), None);
    }
}
