//! Scripted input that reacts to game state, for tests, the host preview and (later) hands-off runs in
//! Dolphin. Produces raw pad states exactly like a real controller would.

use crate::game::Game;
use crate::input::{PadRaw, btn, flag};
use crate::math::{PI, cosf, sinf, wrap_angle};
use crate::skater::Mode;

/// Push up to speed down the spawn lane, load the flick stick on the kicker and kickflip off the lip.
#[derive(Default)]
pub struct KickerRun {
    flicked: bool,
    /// Frames pushed so far (an imported park has no kicker at the spawn: flick on flat ground instead).
    pushed: u32,
}

/// Raw C-stick values: pulled back, and flicked forward and right (a goofy rider's kickflip).
const BACK: (i8, i8) = (0, -92);
const UP_RIGHT: (i8, i8) = (50, 62);
const UP: (i8, i8) = (0, 92);
/// The C-stick pushed most of the way to the side and held: a grab's tweak.
const TWEAK: (i8, i8) = (80, 20);
/// The C-stick held down during a grab: its variant.
const GRAB_DOWN: (i8, i8) = (0, -85);

/// The C-stick at `deg` degrees clockwise from straight forward, fully over.
fn rim(deg: f32) -> (i8, i8) {
    rim_at(deg, 80.0)
}

/// The C-stick at `deg` degrees, `r` raw units out (92: fully over, a Skate 3 wind-up).
fn rim_at(deg: f32, r: f32) -> (i8, i8) {
    let a = deg * PI / 180.0;
    ((sinf(a) * r) as i8, (cosf(a) * r) as i8)
}

impl KickerRun {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next(&mut self, game: &Game) -> PadRaw {
        let s = &game.skater;
        let mut pad = PadRaw {
            flags: flag::CONNECTED,
            ..Default::default()
        };
        // Hold A (keep pushing) until near top push speed.
        if s.mode == Mode::Rolling && s.speed < 6.0 && !self.flicked {
            pad.buttons |= btn::A;
        }
        if !self.flicked && s.mode == Mode::Rolling {
            self.pushed += 1;
            let (load, flick) = if game.imported_park().is_some() {
                (self.pushed > 150, self.pushed > 175)
            } else {
                (s.pos.z > -5.0, s.pos.z > -2.8)
            };
            if flick {
                (pad.cstick_x, pad.cstick_y) = flick_end(game, "Kickflip", s.mirrored(), UP_RIGHT);
                self.flicked = true;
            } else if load {
                (pad.cstick_x, pad.cstick_y) = BACK;
            }
        }
        pad
    }
}

/// The kicker run, then the rest of the move set: carve both ways (the rider leans), a U-turn, a
/// manual held for 3 s, a kick turn, an ollie with an indy grab, a revert, a 360 flip, a 180 to fakie,
/// riding fakie until the rider turns round to switch (with Skate 3's state graphs on their own after
/// 0.6 s, else by pushing), a held double kickflip (state graphs; else a kickflip), an ollie with a late
/// kickflip (state graphs only),
/// a powerslide, a hippie jump, and finally a grab held into the landing (a bail).
/// For the preview and an end-to-end test.
#[derive(Default)]
pub struct TrickTour {
    kicker: KickerRun,
    phase: Tour,
    /// Frames spent in the current phase.
    t: u32,
    /// The kicker jump has left the ground.
    jumped: bool,
    /// Ticks the current wind-up has been held, and when the flick started (0: not yet).
    wind: u32,
    flick_at: u32,
    /// Ticks the current move has been held (the manual, the kick turn ...).
    held: u32,
    /// The stance (switch) when the phase started.
    switch_at_start: bool,
    /// What the tour saw happen, for the log and tests.
    pub seen: Seen,
    /// The heading (yaw) it steers for: clear, flat ground ahead.
    steer_to: Option<f32>,
    /// The grab's scored name when it was first held.
    grab_name: Option<&'static str>,
}

/// Moves the tour saw the game do.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    /// Longest manual (ticks).
    pub manual: u32,
    pub kick_turn: bool,
    pub revert: bool,
    /// Turned round from riding fakie to switch without a push.
    pub auto_switch: bool,
    /// Most flips of a held kickflip.
    pub flips: u8,
    pub late_flip: bool,
    /// The C-stick picked a grab's variant (its scored name changed while the grab was held).
    pub grab_variant: bool,
}

/// How far short of half a turn the 180's let-go spin may carry it (rad): the landing assist does the
/// rest.
const RELEASE_HALF_CAB: f32 = 0.25;

/// How long the tour holds its manual (ticks): over 3 s.
pub const MANUAL_TICKS: u32 = 190;

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
enum Tour {
    #[default]
    Kicker,
    Carve,
    Manual,
    KickTurn,
    GrabOllie,
    GrabAir,
    Revert,
    TreFlip,
    TreAir,
    HalfCab,
    HalfCabAir,
    FakieRide,
    DoubleFlip,
    DoubleAir,
    LateOllie,
    LateAir,
    Powerslide,
    Hippie,
    HippieAir,
    BailOllie,
    BailAir,
    Done,
}

impl Tour {
    fn name(self) -> &'static str {
        match self {
            Tour::Kicker => "kicker",
            Tour::Carve => "carve",
            Tour::Manual => "manual (held 3 s)",
            Tour::KickTurn => "kick turn",
            Tour::GrabOllie | Tour::GrabAir => "ollie, indy grab",
            Tour::Revert => "revert",
            Tour::TreFlip | Tour::TreAir => "360 flip",
            Tour::HalfCab | Tour::HalfCabAir => "180 to fakie",
            Tour::FakieRide => "riding fakie into switch",
            Tour::DoubleFlip | Tour::DoubleAir => "held double kickflip",
            Tour::LateOllie | Tour::LateAir => "ollie, late kickflip",
            Tour::Powerslide => "powerslide",
            Tour::Hippie | Tour::HippieAir => "hippie jump",
            Tour::BailOllie | Tour::BailAir => "grab held into the landing (bail)",
            Tour::Done => "done",
        }
    }
}

/// The heading (yaw) nearest `yaw` along which the ground stays flat and free of walls and rails for
/// `dist` metres from the skater, if there is one.
fn clear_heading(game: &Game, yaw: f32, dist: f32) -> Option<f32> {
    let s = &game.skater;
    let ground = game.terrain();
    let y0 = s.pos.y;
    // Only the rails that come near: checked at each probe.
    let rails: alloc::vec::Vec<&crate::rails::Rail> = ground
        .rails()
        .iter()
        .filter(|r| r.nearest(s.pos).is_some_and(|n| n.dist_h < dist + 1.0))
        .collect();
    let clear = |a: f32| {
        let (dx, dz) = (sinf(a), cosf(a));
        let mut prev = s.pos;
        let mut d = 0.5;
        while d <= dist {
            let p = crate::math::v3(s.pos.x + dx * d, y0, s.pos.z + dz * d);
            let g = ground.ground(p.x, p.z, y0 + 0.3);
            if (g.height - y0).abs() > 0.08 || ground.blocked(prev, p) {
                return false;
            }
            if rails.iter().any(|r| {
                r.nearest(p)
                    .is_some_and(|n| n.dist_h < 0.7 && n.point.y > y0 - 0.2 && n.point.y < y0 + 1.5)
            }) {
                return false;
            }
            prev = p;
            d += 0.75;
        }
        true
    };
    for k in 0..24 {
        let step = (k + 1) / 2;
        let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
        let a = wrap_angle(yaw + sign * step as f32 * (PI / 12.0));
        if clear(a) {
            return Some(a);
        }
    }
    None
}

/// Where a flick of gesture `name` ends on the C-stick (raw), from the loaded gesture file `skater`
/// (mirrored left for right for a mirrored rider); `fallback` without it.
fn flick_end(game: &Game, name: &str, mirrored: bool, fallback: (i8, i8)) -> (i8, i8) {
    match game
        .gesture_shape("skater", name)
        .and_then(|s| s.points.last())
    {
        Some(p) => {
            let x = if mirrored { -p[0] } else { p[0] };
            raw_stick([x, -p[1]], crate::input::C_RANGE)
        }
        None => fallback,
    }
}

/// Raw stick values for a conditioned stick position (undoes `input`'s dead zone and range).
fn raw_stick(v: [f32; 2], range: f32) -> (i8, i8) {
    let mag = crate::math::sqrtf(v[0] * v[0] + v[1] * v[1]);
    if mag < 1e-4 {
        return (0, 0);
    }
    let raw = (mag.min(1.0) * 0.7 + 0.25) * range;
    let k = raw / mag;
    (
        (v[0] * k).clamp(-127.0, 127.0) as i8,
        (v[1] * k).clamp(-127.0, 127.0) as i8,
    )
}

impl TrickTour {
    pub fn new() -> Self {
        Self::default()
    }

    /// The motion graph's state, when Skate 3's state graphs run.
    fn graph_state(game: &Game) -> Option<alloc::string::String> {
        game.graph.as_ref().map(|g| g.runner.motion_state())
    }

    /// A wind-up then a flick: holds `hold` until the rider is wound up (with Skate 3's state graphs,
    /// until its motion graph is in the anticipation cycle; else for 10 ticks), then plays `flick` one
    /// position per tick and holds the last.
    fn wind_and_flick(
        &mut self,
        game: &Game,
        hold: (i8, i8),
        ticks: u32,
        flick: &[(i8, i8)],
    ) -> (i8, i8) {
        if self.flick_at == 0 {
            self.wind += 1;
            let ready = match Self::graph_state(game) {
                Some(st) => {
                    st.contains("AnticCyc") || (self.wind > 120 && st.contains("Anticipation"))
                }
                None => self.wind >= ticks,
            };
            if !ready {
                return hold;
            }
            self.flick_at = self.t;
        }
        let k = (self.t - self.flick_at) as usize;
        flick[k.min(flick.len() - 1)]
    }

    /// Plays a gesture file's shape on the main stick from `from`: its first point held for a moment
    /// (long enough for shorter shapes that also start there, such as the body flips, to be dropped),
    /// then one point a tick. The raw stick for this tick, `None` once played (or without the shape).
    fn replay(&self, game: &Game, set: &str, name: &str, from: u32) -> Option<(i8, i8)> {
        const HOLD_FIRST: usize = 12;
        let shape = game.gesture_shape(set, name)?;
        let k = (self.t.checked_sub(from)? as usize).saturating_sub(HOLD_FIRST);
        let p = shape.points.get(k)?;
        // Gesture space has y towards the player: the stick's y is the other way.
        Some(raw_stick([p[0], -p[1]], crate::input::MAIN_RANGE))
    }

    /// Heading where the tour steers (clear ground ahead), give or take a little: time to pop.
    fn lined_up(&self, s: &crate::skater::Skater) -> bool {
        self.wind > 0
            || self
                .steer_to
                .is_none_or(|h| wrap_angle(h - s.travel_yaw()).abs() < 0.12)
    }

    /// In the air: on to `air` if it was the flick's pop; launched off a lip before the flick, the
    /// phase starts over once rolling again.
    fn popped(&mut self, air: Tour, switch: bool) {
        if self.flick_at != 0 {
            self.phase = air;
            self.t = 0;
            self.wind = 0;
            self.flick_at = 0;
            self.held = 0;
            self.switch_at_start = switch;
        } else {
            self.wind = 0;
            self.t = 1;
        }
    }

    /// The phase it is in, by name (for the log).
    pub fn phase_name(&self) -> &'static str {
        self.phase.name()
    }

    pub fn next(&mut self, game: &Game) -> PadRaw {
        let s = &game.skater;
        let rolling = s.mode == Mode::Rolling;
        let graph = Self::graph_state(game);
        let in_state = |part: &str| graph.as_ref().is_some_and(|g| g.contains(part));
        let mut pad = PadRaw {
            flags: flag::CONNECTED,
            ..Default::default()
        };
        let next = |p: &mut Self, phase, switch: bool| {
            p.phase = phase;
            p.t = 0;
            p.wind = 0;
            p.flick_at = 0;
            p.held = 0;
            p.switch_at_start = switch;
        };
        self.t += 1;
        // Somewhere the script doesn't fit (an imported park): give up on a phase after 20 s.
        if self.t > 20 * 60 && self.phase != Tour::Done {
            next(self, Tour::Done, s.switch);
            return pad;
        }
        // Keep the speed down so everything fits in the park (X: foot brake), and steer back towards the
        // middle when near a wall.
        let busy_stick = matches!(
            self.phase,
            Tour::Kicker
                | Tour::Carve
                | Tour::KickTurn
                | Tour::Revert
                | Tour::HalfCabAir
                | Tour::Powerslide
                | Tour::BailAir
        );
        // (Not in the manual, the kick turn or riding fakie: a foot brake would take the rider out.)
        if rolling
            && s.speed > 4.5
            && !matches!(
                self.phase,
                Tour::Kicker | Tour::Powerslide | Tour::KickTurn | Tour::FakieRide | Tour::Revert
            )
            && !(self.phase == Tour::Manual && self.t >= 40)
        {
            pad.buttons |= btn::X;
        }
        if rolling && !busy_stick && game.imported_park().is_none() {
            // The demo park: back towards the middle when near a wall.
            if s.pos.x.abs() > 6.0 || s.pos.z.abs() > 9.0 {
                let want = crate::math::atan2f(-s.pos.x, -s.pos.z);
                let err = wrap_angle(want - s.travel_yaw());
                pad.stick_x = (-err * 120.0).clamp(-100.0, 100.0) as i8;
            }
        } else if rolling && !busy_stick {
            // An imported park: towards clear, flat ground.
            if self.t % 20 == 1 {
                let ahead = (s.speed.abs() * 1.6 + 2.5).clamp(4.0, 10.0);
                self.steer_to = clear_heading(game, s.travel_yaw(), ahead);
            }
            if let Some(h) = self.steer_to {
                let err = wrap_angle(h - s.travel_yaw());
                // Slow, a stick past 0.3 is a kick turn: steer gently then.
                let most = if s.speed.abs() < 4.6 { 45.0 } else { 100.0 };
                if err.abs() > 0.05 {
                    pad.stick_x = (-err * 120.0).clamp(-most, most) as i8;
                }
            }
        }
        let sw = s.switch;
        match self.phase {
            Tour::Kicker => {
                pad = self.kicker.next(game);
                // Landed off the kicker and rolled a moment.
                if !rolling {
                    self.jumped = true;
                    self.t = 0;
                } else if self.jumped && self.t > 30 {
                    next(self, Tour::Carve, sw);
                }
            }
            Tour::Carve => {
                // A quick lean right, straight on past the funbox, then a U-turn to the left back down
                // the park between the kicker and the funbox.
                if s.speed > 3.5 {
                    pad.buttons |= btn::X;
                }
                let turned = wrap_angle(s.travel_yaw() - PI).abs() < 0.25;
                pad.stick_x = if self.t < 12 {
                    85
                } else if s.pos.z < 9.0 && self.t < 200 {
                    -10
                } else if !turned && self.t < 300 {
                    -100
                } else {
                    0
                };
                if turned && self.t > 40 {
                    next(self, Tour::Manual, sw);
                }
            }
            Tour::Manual => {
                // Push up first, then hold the C-stick lightly back (below full: a full stick is a
                // wind-up) and keep the balance marker centred.
                if self.t < 40 {
                    if s.speed < 3.0 && s.manual.is_none() {
                        pad.buttons |= btn::A;
                    }
                } else {
                    let b = s.manual.map_or(0.0, |m| m.balance);
                    let push = (0.35 - 1.2 * b).clamp(0.02, 0.98);
                    let gesture_y = 0.1 + push * 0.8;
                    // Undo the stick conditioning: 0.25 dead zone, full at 0.95 of the C-stick's range.
                    pad.cstick_y = -((gesture_y * 0.7 + 0.25) * crate::input::C_RANGE) as i8;
                }
                // Held for 3 s from when it engaged (a push still running holds it off at first).
                if s.manual.is_some() {
                    self.held += 1;
                    self.seen.manual = self.seen.manual.max(self.held);
                }
                if self.held > MANUAL_TICKS || (self.held == 0 && self.t > 600) {
                    next(self, Tour::KickTurn, sw);
                }
            }
            Tour::KickTurn => {
                // Brake nearly to a stop, then the main stick hard over: the board turns on its back
                // wheels.
                if self.held == 0 && s.speed.abs() > 0.6 {
                    pad.buttons |= btn::X;
                } else {
                    self.held += 1;
                    if self.held > 6 {
                        pad.stick_x = 100;
                    }
                }
                if s.kickturn.is_some() || in_state("/KickTurn/") {
                    self.seen.kick_turn = true;
                }
                if (self.seen.kick_turn && self.held > 50) || self.t > 300 {
                    next(self, Tour::GrabOllie, sw);
                }
            }
            Tour::GrabOllie => {
                if s.speed < 3.0 && self.t < 40 {
                    pad.buttons |= btn::A;
                }
                // Load, then flick straight up.
                if self.t > 50 && self.lined_up(s) {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, 10, &[UP]);
                }
                if !rolling {
                    self.popped(Tour::GrabAir, sw);
                }
            }
            Tour::GrabAir => {
                // Grab from just after the pop until falling, then let go to land; a moment into the
                // grab the C-stick goes down to pick the grab's variant (and back before letting go).
                let grabbing = self.t > 4 && s.vel.y > -2.0;
                if grabbing {
                    pad.trig_r = 255;
                }
                if let Some(g) = &s.grab {
                    match self.grab_name {
                        None => self.grab_name = Some(g.name),
                        Some(n) if n != g.name => self.seen.grab_variant = true,
                        _ => {}
                    }
                    // (Back to the middle by the top: still tweaking it on landing would wipe out.)
                    if grabbing && g.time > 0.08 && s.vel.y > 0.5 {
                        (pad.cstick_x, pad.cstick_y) = GRAB_DOWN;
                    }
                }
                if rolling {
                    next(self, Tour::Revert, sw);
                }
            }
            Tour::Revert => {
                // Skate 3's revert gesture on the main stick (`SlideFs180`, with the state graphs) once
                // riding again; without them a powerslide flick just after landing.
                if game.graph.is_some() {
                    // Fast enough that a stick to the side isn't a kick turn.
                    if self.held == 0 && s.speed < 4.3 {
                        pad.buttons |= btn::A;
                    }
                    let from = if self.held == 0
                        && in_state("Turning/Idle")
                        && s.speed > 4.1
                        && self.t > 10
                    {
                        self.held = self.t;
                        self.t
                    } else {
                        self.held
                    };
                    if self.held > 0
                        && let Some((x, y)) = self.replay(game, "skaterls", "SlideFs180", from)
                    {
                        (pad.stick_x, pad.stick_y) = (x, y);
                    }
                } else if self.t < 12 {
                    (pad.stick_x, pad.stick_y) = (50, -87);
                }
                if s.revert.is_some() || in_state("/Revert/") {
                    self.seen.revert = true;
                }
                if (self.seen.revert && s.revert.is_none() && !in_state("/Revert/")) || self.t > 240
                {
                    next(self, Tour::TreFlip, sw);
                }
            }
            Tour::TreFlip => {
                if s.speed < 3.5 && self.t < 40 {
                    pad.buttons |= btn::A;
                }
                // Roll the stick round the back from the left, then flick across to forward-right.
                if self.t > 50 && self.lined_up(s) {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(
                        game,
                        rim_at(260.0, 92.0),
                        4,
                        &[rim(215.0), rim(165.0), (39, -19), rim(65.0)],
                    );
                }
                if !rolling {
                    self.popped(Tour::TreAir, sw);
                }
            }
            Tour::TreAir => {
                if rolling {
                    next(self, Tour::HalfCab, sw);
                }
            }
            Tour::HalfCab => {
                if s.speed < 3.5 && self.t < 40 {
                    pad.buttons |= btn::A;
                }
                if self.t > 50 && self.lined_up(s) {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, 10, &[UP]);
                }
                if !rolling {
                    self.popped(Tour::HalfCabAir, sw);
                }
            }
            Tour::HalfCabAir => {
                // Spin left, letting the stick go once the let-go spin will carry it about half round
                // (it slows by the opposite-direction limit a tick); the landing assist finishes it.
                let slow = (s.tuning.spin_reverse_change * 60.0).max(1.0);
                let carry = s.spin_rate * s.spin_rate / (2.0 * slow);
                if s.spin.abs() + carry < PI - RELEASE_HALF_CAB {
                    pad.stick_x = -100;
                }
                if rolling {
                    next(self, Tour::FakieRide, sw);
                }
            }
            Tour::FakieRide => {
                // Rolling tail first: with the state graphs the rider turns round to switch on their own
                // after a moment; without them a push does it.
                let graphs = game.graph.is_some();
                if (!graphs && self.t == 20) || (self.t > 120 && s.speed < 3.5 && self.t < 160) {
                    pad.buttons |= btn::A;
                }
                if s.switch != self.switch_at_start && self.t <= 120 && graphs {
                    self.seen.auto_switch = true;
                }
                if self.t > 180 {
                    next(self, Tour::DoubleFlip, sw);
                }
            }
            Tour::DoubleFlip => {
                if s.speed < 3.5 && self.t < 40 {
                    pad.buttons |= btn::A;
                }
                // A full wind-up, the kickflip flick, and the stick held where the flick ended.
                if self.t > 50 && self.lined_up(s) {
                    let flick = flick_end(game, "Kickflip", s.mirrored(), UP_RIGHT);
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, 10, &[flick]);
                }
                if !rolling {
                    self.popped(Tour::DoubleAir, sw);
                }
            }
            Tour::DoubleAir => {
                // The stick stays on the flick's end until the second flip, then lets go.
                if let Some(t) = &s.trick {
                    self.seen.flips = self.seen.flips.max(t.flips);
                }
                // (flatground's own rules need more air than a flat-ground ollie for two flips.)
                if self.seen.flips < 2 && game.graph.is_some() {
                    (pad.cstick_x, pad.cstick_y) =
                        flick_end(game, "Kickflip", s.mirrored(), UP_RIGHT);
                }
                if rolling {
                    next(self, Tour::LateOllie, sw);
                }
            }
            Tour::LateOllie => {
                // Late flips are the state graphs' (InAir/Lateflip).
                if game.graph.is_none() {
                    next(self, Tour::Powerslide, sw);
                    return pad;
                }
                if s.speed < 3.5 && self.t < 40 {
                    pad.buttons |= btn::A;
                }
                if self.t > 50 && self.lined_up(s) {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, 10, &[UP]);
                }
                if !rolling {
                    self.popped(Tour::LateAir, sw);
                }
            }
            Tour::LateAir => {
                // The stick pulled back while the ollie plays, flicked forward the moment the graph is
                // back in its plain air state (a late flip starts only from there): a late kickflip.
                if self.held == 0 && in_state("InAirStatic") {
                    self.held = self.t;
                }
                let flick = flick_end(game, "Kickflip", s.mirrored(), UP_RIGHT);
                if self.held > 0 {
                    if self.t - self.held < 3 {
                        (pad.cstick_x, pad.cstick_y) = flick;
                    }
                } else if self.t > 10 && in_state("/LeftGround/") {
                    (pad.cstick_x, pad.cstick_y) = BACK;
                }
                if in_state("/Lateflip/") {
                    self.seen.late_flip = true;
                }
                if rolling {
                    next(self, Tour::Powerslide, sw);
                }
            }
            Tour::Powerslide => {
                if self.t < 60 && s.speed < 5.5 {
                    pad.buttons |= btn::A;
                } else if self.t >= 60 && self.t < 100 {
                    // Main stick fully back and to the right.
                    (pad.stick_x, pad.stick_y) = (50, -87);
                }
                if self.t > 120 {
                    next(self, Tour::Hippie, sw);
                }
            }
            Tour::Hippie => {
                if self.t < 40 && s.speed < 3.0 {
                    pad.buttons |= btn::A;
                }
                if (50..70).contains(&self.t) {
                    pad.buttons |= btn::A | btn::B;
                }
                if !rolling {
                    next(self, Tour::HippieAir, sw);
                }
            }
            Tour::HippieAir => {
                if rolling {
                    next(self, Tour::BailOllie, sw);
                }
            }
            Tour::BailOllie => {
                if s.speed < 3.0 && self.t < 30 {
                    pad.buttons |= btn::A;
                }
                if self.t > 50 && self.lined_up(s) {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, 10, &[UP]);
                }
                if !rolling {
                    self.popped(Tour::BailAir, sw);
                }
            }
            Tour::BailAir => {
                // Grab, tweak it with the C-stick and never let go: still tweaking the grab when the
                // wheels touch down wipes out.
                if self.t > 4 {
                    pad.trig_l = 255;
                }
                if self.t > 8 {
                    (pad.cstick_x, pad.cstick_y) = TWEAK;
                }
                if matches!(s.mode, Mode::Bailed { .. }) {
                    next(self, Tour::Done, sw);
                }
            }
            Tour::Done => {}
        }
        pad
    }

    /// True once the whole tour has been played (after the bail).
    pub fn done(&self) -> bool {
        self.phase == Tour::Done
    }
}

/// Hands-off play for scripted runs on the Wii and in Dolphin (`autoplay.txt` in the app folder, see
/// docs/logging.md): the [`TrickTour`] over and over, after the rider is back on the board each time.
#[derive(Default)]
pub struct Autoplay {
    tour: TrickTour,
    /// Tours finished so far.
    pub rounds: u32,
    /// Frames to wait (rolling, no input) before the next tour starts.
    rest: u32,
    /// The phase last logged.
    shown: &'static str,
    /// A tour just ended and the rider is back on the board: start the next one from the park's spawn
    /// (the tour is laid out from there), see [`Autoplay::take_restart`].
    restart: bool,
}

impl Autoplay {
    pub fn new() -> Self {
        Self::default()
    }

    /// True once when the next tour should start from the park's spawn (the caller puts the rider there:
    /// `Game::restart_at_spawn`).
    pub fn take_restart(&mut self) -> bool {
        core::mem::take(&mut self.restart)
    }

    /// The tour phase it is in (for the log).
    pub fn phase(&self) -> &'static str {
        self.tour.phase_name()
    }

    /// The next pad state. Returns the line to log when a tour phase starts or the tour finishes.
    pub fn next(&mut self, game: &Game) -> (PadRaw, Option<alloc::string::String>) {
        let idle = PadRaw {
            flags: flag::CONNECTED,
            ..Default::default()
        };
        if self.rest > 0 {
            self.rest -= 1;
            // Wait until the rider is up again before starting over.
            if self.rest == 0 {
                if matches!(game.skater.mode, Mode::Rolling) {
                    self.restart = true;
                } else {
                    self.rest = 30;
                }
            }
            return (idle, None);
        }
        let pad = self.tour.next(game);
        if self.tour.done() {
            self.rounds += 1;
            let seen = self.tour.seen;
            self.tour = TrickTour::new();
            self.shown = "";
            self.rest = 180;
            return (
                idle,
                Some(alloc::format!(
                    "autoplay: tour {} done (manual {:.1} s, kick turn {}, revert {}, auto switch {}, \
                     {} flips held, late flip {}, grab variant {})",
                    self.rounds,
                    seen.manual as f32 / 60.0,
                    seen.kick_turn,
                    seen.revert,
                    seen.auto_switch,
                    seen.flips,
                    seen.late_flip,
                    seen.grab_variant
                )),
            );
        }
        let name = self.tour.phase_name();
        if name != self.shown {
            self.shown = name;
            return (pad, Some(alloc::format!("autoplay: {name}")));
        }
        (pad, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoplay_repeats_the_tour() {
        let mut game = Game::new();
        let mut auto = Autoplay::new();
        let mut lines = 0;
        for _ in 0..(60 * 120) {
            let (pad, line) = auto.next(&game);
            lines += line.is_some_and(|l| l.contains(" done")) as u32;
            if auto.take_restart() {
                game.restart_at_spawn();
            }
            game.update(&pad, 1.0 / 60.0);
            if auto.rounds >= 2 {
                break;
            }
        }
        assert!(auto.rounds >= 2, "rounds {}", auto.rounds);
        assert_eq!(lines, auto.rounds);
    }
}
