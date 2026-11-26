//! Scripted input that reacts to game state, for tests, the host preview and (later) hands-off runs in
//! Dolphin. Produces raw pad states exactly like a real controller would.

use crate::game::Game;
use crate::input::{PadRaw, btn, flag};
use crate::math::{PI, Vec3, atan2f, cosf, sinf, v3, wrap_angle};
use crate::rails::RailClass;

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
        // Hold A (keep pushing) until near top push speed, and not once winding up for the pop (the
        // rigid board loses some speed meeting the kicker; a flick while a push is under way would be
        // a no-comply).
        let winding = game.on_demo_park() && s.pos.z > -5.0;
        if s.rolling() && s.speed < 6.0 && !self.flicked && !winding {
            pad.buttons |= btn::A;
        }
        if !self.flicked && s.rolling() {
            self.pushed += 1;
            let (load, flick) = if !game.on_demo_park() {
                (self.pushed > 150, self.pushed > 175)
            } else {
                (s.pos.z > -5.0, s.pos.z > -3.3)
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

/// A camera check: push up to riding speed, carve a quarter turn at full lock, ride straight on for
/// `straight` ticks, wind up and ollie, ride on. The preview's `--carve-ollie` and the camera's
/// regression test run it; [`CarveOllie::phase`] says where it is.
pub struct CarveOllie {
    /// Ticks so far, and ticks of straight riding between the carve and the wind-up.
    pub t: u32,
    pub straight: u32,
    /// When the carve started and ended, the wind-up started, the flick went (0: not yet).
    pub carve_at: u32,
    pub carved_at: u32,
    pub wind_at: u32,
    pub flick_at: u32,
    start_yaw: Option<f32>,
}

impl CarveOllie {
    pub fn new(straight: u32) -> Self {
        CarveOllie {
            t: 0,
            straight,
            carve_at: 0,
            carved_at: 0,
            wind_at: 0,
            flick_at: 0,
            start_yaw: None,
        }
    }

    /// `push`, `carve`, `straight`, `wind`, `flick` or `ride`.
    pub fn phase(&self) -> &'static str {
        if self.carve_at == 0 {
            "push"
        } else if self.carved_at == 0 {
            "carve"
        } else if self.wind_at == 0 {
            "straight"
        } else if self.flick_at == 0 {
            "wind"
        } else if self.t < self.flick_at + 8 {
            "flick"
        } else {
            "ride"
        }
    }

    /// Where to put the rider first on the demo park (the caller does: `Game::place_rider`): the clear
    /// lane by the bank, heading +z, so the carve goes off to -x across open ground.
    pub const START: (Vec3, f32) = (v3(12.0, 0.0, -19.0), 0.0);

    pub fn next(&mut self, game: &Game) -> PadRaw {
        let s = &game.skater;
        let mut pad = PadRaw {
            flags: flag::CONNECTED,
            ..Default::default()
        };
        self.t += 1;
        let rolling = s.rolling();
        match self.phase() {
            "push" => {
                if rolling && s.speed < 4.5 && self.t % 30 < 10 {
                    pad.buttons |= btn::A;
                }
                if self.t > 100 && !s.graph.pushing {
                    self.carve_at = self.t;
                    self.start_yaw = Some(s.travel_yaw());
                }
            }
            "carve" => {
                pad.stick_x = 100;
                let turned = self
                    .start_yaw
                    .is_some_and(|y| wrap_angle(s.travel_yaw() - y).abs() > PI / 2.0);
                if turned || self.t > self.carve_at + 120 {
                    self.carved_at = self.t;
                }
            }
            "straight" => {
                if self.t >= self.carved_at + self.straight {
                    self.wind_at = self.t;
                }
            }
            "wind" => {
                (pad.cstick_x, pad.cstick_y) = BACK;
                let ready = match game.graph.as_ref().map(|g| g.runner.motion_state()) {
                    Some(st) => st.contains("AnticCyc") || self.t > self.wind_at + 60,
                    None => self.t >= self.wind_at + 10,
                };
                if ready {
                    self.flick_at = self.t;
                }
            }
            "flick" => (pad.cstick_x, pad.cstick_y) = UP,
            _ => {}
        }
        pad
    }
}

/// The kicker run, then the rest of the move set: carve both ways (the rider leans), a U-turn, a
/// manual held for 3 s, a kick turn, an ollie with an indy grab, a revert, a 360 flip, a 180 to fakie,
/// riding fakie until the rider turns round to switch (with Skate 3's state graphs on their own after
/// 0.6 s, else by pushing), a held double kickflip (state graphs; else a kickflip), an ollie with a late
/// kickflip (state graphs only),
/// a powerslide, a hippie jump, grinds (an ollie onto a ledge into a 50-50, tipped into a 5-0 with the
/// C-stick; rolling off a quarter pipe's deck into a coping grind; an ollie along a ledge's line with a
/// quarter turn into a slide), and finally a grab held into the landing (a bail). For the grinds the
/// tour asks to be put beside a ledge or on a deck ([`TrickTour::take_place`]).
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
    /// Where the tour wants the rider put (the caller does it: `Game::place_rider`).
    place: Option<Placement>,
    /// The rail the grind phase uses.
    rail: Option<RailPick>,
    /// The off-board stop's jump happened.
    walk_jumped: bool,
}

/// Where to put the rider: position, heading (yaw), speed along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub pos: Vec3,
    pub heading: f32,
    pub speed: f32,
    /// Upward speed (m/s): above 0 the rider is put in the air at `pos`, moving along `heading` at
    /// `speed` and up at this ([`Game::place`]).
    pub lift: f32,
}

/// A straight, level run of a rail the tour grinds: its rail, where it starts along it, its direction
/// and high side, the point it starts at.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RailPick {
    rail: usize,
    s: f32,
    dir: Vec3,
    high: Vec3,
    start: Vec3,
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
    /// Grinds: onto a ledge, tipped into a 5-0 by the C-stick, on coping, a slide (board or tip).
    pub ledge: bool,
    pub five_o: bool,
    pub coping: bool,
    pub slide: bool,
    /// A d-pad gesture played (the state graphs' CharacterGesture).
    pub gesture: bool,
    /// A thin rail ground to its end (not leaned off).
    pub handrail: bool,
    /// A back flip turned (whole turns, rounded) and landed on the wheels.
    pub backflip: bool,
    /// Got off the board (Y), walked, jumped, landed on foot and got back on.
    pub walked: bool,
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
    LedgeFind,
    LedgeRide,
    LedgeAir,
    LedgeGrind,
    CopingFind,
    CopingRide,
    SlideFind,
    SlideRide,
    SlideAir,
    Gesture,
    Walk,
    RailFind,
    RailGrind,
    FlipFind,
    FlipRide,
    FlipAir,
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
            Tour::LedgeFind | Tour::LedgeRide | Tour::LedgeAir | Tour::LedgeGrind => {
                "ollie onto a ledge, 50-50 into a 5-0"
            }
            Tour::CopingFind | Tour::CopingRide => "coping grind off a quarter pipe's deck",
            Tour::SlideFind | Tour::SlideRide | Tour::SlideAir => "ollie into a slide",
            Tour::Gesture => "d-pad gesture",
            Tour::Walk => "off the board: walk, jump, back on",
            Tour::RailFind | Tour::RailGrind => "handrail grind to its end",
            Tour::FlipFind | Tour::FlipRide | Tour::FlipAir => "back flip off the kicker",
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

/// Whether the ground stays flat and free of walls for `dist` metres along `yaw` from the skater, on
/// its line and `half` metres either side of it.
fn lane_clear(game: &Game, yaw: f32, dist: f32, half: f32) -> bool {
    let s = &game.skater;
    let ground = game.terrain();
    let y0 = s.pos.y;
    let (dx, dz) = (sinf(yaw), cosf(yaw));
    [-half, 0.0, half].iter().all(|&side| {
        let start = s.pos + crate::math::v3(dz * side, 0.0, -dx * side);
        let mut prev = start;
        let mut d = 0.5;
        while d <= dist {
            let p = crate::math::v3(start.x + dx * d, y0, start.z + dz * d);
            let g = ground.ground(p.x, p.z, y0 + 1.5);
            if (g.height - y0).abs() > 0.08 || ground.blocked(prev, p) {
                return false;
            }
            prev = p;
            d += 0.5;
        }
        true
    })
}

/// How far beside a ledge the tour rides to ollie onto it, and how far in from a deck's edge it rolls
/// off onto coping (m).
const LEDGE_SIDE: f32 = 0.35;
const DECK_IN: f32 = 0.4;

/// What the tour wants of a rail: a ledge to ride beside and ollie onto, coping to roll onto from its
/// deck, or a ledge to ollie onto from its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    Ledge,
    Coping,
    Slide,
}

/// The longest straight, level run (at least `len` metres) of a ledge-class rail whose sides are what
/// the tour needs, skipping rail `avoid`: for a ledge, flat ground 0.25 to 0.8 m down on the low side
/// along the run and `approach` metres before it (to ride beside it and pop); for coping, a steep
/// transition more than a metre down on the low side and a flat deck level with the rail on the high
/// side; for a slide, a ledge with flat ground on the rail's line before its start.
fn find_rail(game: &Game, want: Want, len: f32, avoid: Option<usize>) -> Option<RailPick> {
    let park = game.terrain();
    let (side, approach) = match want {
        Want::Ledge => (LEDGE_SIDE, 5.0),
        Want::Coping => (DECK_IN, 0.0),
        Want::Slide => (0.05, 2.5),
    };
    let flat_at = |p: Vec3, height: f32| {
        let g = park.ground(p.x, p.z, height + 0.1);
        g.normal.y > 0.95 && (g.height - height).abs() < 0.05
    };
    let mut best: Option<(f32, RailPick)> = None;
    for (ri, r) in park.rails().iter().enumerate() {
        if Some(ri) == avoid {
            continue;
        }
        let n = r.points.len();
        let mut s0 = 0.0;
        let mut dir0: Option<Vec3> = None;
        for i in 0..n.saturating_sub(1) {
            let (a, b) = (r.points[i], r.points[i + 1]);
            let d = (b - a).normalize_or(Vec3::ZERO);
            let class = r.classes.get(i).map_or(RailClass::Thin, |c| c.class);
            let ok = class == RailClass::Ledge
                && d.y.abs() < 0.05
                && dir0.is_none_or(|d0| d0.dot(d) > 0.996);
            if !ok {
                s0 = r.segment_start(i + 1);
                dir0 = None;
                continue;
            }
            let d0 = *dir0.get_or_insert(d);
            let end = r.segment_start(i + 1);
            // Only a run's longest stretch counts: judge it where it ends.
            let last = i + 2 >= n
                || r.classes
                    .get(i + 1)
                    .is_none_or(|c| c.class != RailClass::Ledge)
                || {
                    let e = (r.points[i + 2] - b).normalize_or(Vec3::ZERO);
                    e.y.abs() >= 0.05 || d0.dot(e) <= 0.996
                };
            let run = end - s0;
            if !last || run < len || best.is_some_and(|b| b.0 >= run) {
                continue;
            }
            let (start, _) = r.at(s0);
            let (_, high) = r.class_at(s0 + 0.5);
            let low = -high;
            let low_height = park.ground(
                start.x + low.x * side,
                start.z + low.z * side,
                start.y - 0.1,
            );
            let drop = start.y - low_height.height;
            let mut good = match want {
                Want::Coping => drop > 1.0,
                _ => (0.25..=0.8).contains(&drop),
            };
            let mut k = -approach;
            while k <= run && good {
                let p = start + d0 * k;
                good = match want {
                    Want::Coping => {
                        let deck = p + high * DECK_IN;
                        let wall = park.ground(p.x + low.x * 0.3, p.z + low.z * 0.3, p.y - 0.05);
                        flat_at(deck, p.y) && wall.normal.y < 0.85 && p.y - wall.height > 0.05
                    }
                    Want::Ledge => {
                        flat_at(p + low * side, low_height.height)
                            && flat_at(p + low * (side + 0.8), low_height.height)
                    }
                    Want::Slide if k < 0.0 => {
                        flat_at(p + low * side, low_height.height)
                            && flat_at(p + high * 0.3, low_height.height)
                            && flat_at(p + low * 0.5, low_height.height)
                    }
                    Want::Slide => flat_at(p + low * 0.5, low_height.height),
                };
                k += 1.0;
            }
            if good {
                best = Some((
                    run,
                    RailPick {
                        rail: ri,
                        s: s0,
                        dir: d0,
                        high,
                        start,
                    },
                ));
            }
        }
    }
    best.map(|b| b.1)
}

/// The longest run (at least `len` metres, gently sloped) of a thin rail, started from its higher end:
/// (rail, start point, direction along it, length).
fn find_thin_rail(game: &Game, len: f32) -> Option<(usize, Vec3, Vec3, f32)> {
    let mut best: Option<(usize, Vec3, Vec3, f32)> = None;
    for (ri, r) in game.terrain().rails().iter().enumerate() {
        let n = r.points.len();
        let mut i = 0;
        while i + 1 < n {
            let thin = |k: usize| r.classes.get(k).is_none_or(|c| c.class == RailClass::Thin);
            if !thin(i) {
                i += 1;
                continue;
            }
            let d0 = (r.points[i + 1] - r.points[i]).normalize_or(Vec3::ZERO);
            let mut j = i + 1;
            while j + 1 < n && thin(j) && {
                let e = (r.points[j + 1] - r.points[j]).normalize_or(Vec3::ZERO);
                e.dot(d0) > 0.99
            } {
                j += 1;
            }
            let (a, b) = (r.points[i], r.points[j]);
            let length = (b - a).length();
            let dir = (b - a).normalize_or(Vec3::ZERO);
            if length >= len && dir.y.abs() < 0.35 && best.is_none_or(|x| x.3 < length) {
                best = Some(if a.y >= b.y {
                    (ri, a, dir, length)
                } else {
                    (ri, b, -dir, length)
                });
            }
            i = j;
        }
    }
    best
}

/// The yaw of a direction.
fn yaw_of(d: Vec3) -> f32 {
    atan2f(d.x, d.z)
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
        self.replay_held(game, set, name, from, 12)
    }

    /// [`TrickTour::replay`] with the first point held `hold` ticks.
    fn replay_held(
        &self,
        game: &Game,
        set: &str,
        name: &str,
        from: u32,
        hold: usize,
    ) -> Option<(i8, i8)> {
        let shape = game.gesture_shape(set, name)?;
        let k = (self.t.checked_sub(from)? as usize).saturating_sub(hold);
        let p = shape.points.get(k)?;
        // Gesture space has y towards the player: the stick's y is the other way.
        Some(raw_stick([p[0], -p[1]], crate::input::MAIN_RANGE))
    }

    /// Heading where the tour steers (clear ground ahead), give or take a little: time to pop.
    /// (Not mid-turn either: a wind-up started at full lock flies with the camera wherever the turn left
    /// it, as Skate 3's small-air shot keeps the shot it took over.)
    fn lined_up(&self, s: &crate::skater::Skater) -> bool {
        self.wind > 0
            || (s.steer.abs() < 0.3
                && self
                    .steer_to
                    .is_none_or(|h| wrap_angle(h - s.travel_yaw()).abs() < 0.12))
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

    /// Where the tour wants the rider put now, once (the caller puts the rider there:
    /// `Game::place_rider`).
    pub fn take_place(&mut self) -> Option<Placement> {
        self.place.take()
    }

    /// The main stick that keeps the rider riding along `pick`'s rail at `side` metres off it (+ on the
    /// low side), heading along it.
    fn hold_line(s: &crate::skater::Skater, pick: &RailPick, side: f32) -> i8 {
        let low = -pick.high;
        let off = (s.pos - pick.start).dot(low) - side;
        let want =
            yaw_of(pick.dir) + (off * 0.6).clamp(-0.15, 0.15) * low.cross(pick.dir).y.signum();
        let err = wrap_angle(want - s.travel_yaw());
        (-err * 150.0).clamp(-45.0, 45.0) as i8
    }

    pub fn next(&mut self, game: &Game) -> PadRaw {
        let s = &game.skater;
        let rolling = s.rolling();
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
                | Tour::TreFlip
                | Tour::TreAir
                | Tour::HalfCabAir
                | Tour::Powerslide
                | Tour::BailAir
                | Tour::LedgeRide
                | Tour::LedgeAir
                | Tour::LedgeGrind
                | Tour::CopingRide
                | Tour::SlideRide
                | Tour::SlideAir
                | Tour::RailGrind
                | Tour::FlipRide
                | Tour::FlipAir
        );
        // (Not in the manual, the kick turn or riding fakie: a foot brake would take the rider out.)
        if rolling
            && s.speed > 4.5
            && !matches!(
                self.phase,
                Tour::Kicker
                    | Tour::Powerslide
                    | Tour::KickTurn
                    | Tour::FakieRide
                    | Tour::Revert
                    | Tour::FlipRide
            )
            && !(self.phase == Tour::Manual && self.t >= 40)
        {
            pad.buttons |= btn::X;
        }
        if rolling && !busy_stick && game.on_demo_park() {
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
                // (The rigid board's U-turn at this speed is about 5 m across: lean right a while first.)
                pad.stick_x = if self.t < 30 {
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
                    self.steer_to = None;
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
                // The manual barely steers: it starts only with clear, flat ground for the whole of
                // it ahead, a board's width either side (the rigid board is pushed back off a wall it
                // runs into, which would leave it rolling fakie); until then, steer for some.
                if rolling && self.t >= 40 && s.manual.is_none() && self.held == 0 {
                    let ahead = (s.speed.abs() * 3.5).clamp(4.0, 16.0);
                    if !lane_clear(game, s.travel_yaw(), ahead, 0.4) {
                        pad.cstick_y = 0;
                        if self.steer_to.is_none() {
                            self.steer_to = (1..12).find_map(|k| {
                                let a = s.travel_yaw()
                                    + if k % 2 == 0 { 1.0 } else { -1.0 }
                                        * ((k + 1) / 2) as f32
                                        * 0.2;
                                lane_clear(game, a, ahead, 0.4).then_some(a)
                            });
                        }
                        if let Some(h) = self.steer_to {
                            let err = wrap_angle(h - s.travel_yaw());
                            pad.stick_x = if err > 0.0 { -100 } else { 100 };
                        }
                    }
                }
                // Held for 3 s from when it engaged (a push still running holds it off at first).
                if s.manual.is_some() {
                    self.held += 1;
                    self.seen.manual = self.seen.manual.max(self.held);
                }
                if self.held > MANUAL_TICKS || (self.held == 0 && self.t > 600) {
                    next(self, Tour::KickTurn, sw);
                    self.steer_to = None;
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
                // Skate 3's revert gesture on the main stick (`SlideFs180`) once riding again, fast
                // enough that a stick to the side isn't a kick turn.
                if self.held == 0 && s.speed < 4.3 {
                    pad.buttons |= btn::A;
                }
                let from =
                    if self.held == 0 && in_state("Turning/Idle") && s.speed > 4.1 && self.t > 10 {
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
                // Rolling tail first: the state graphs turn the rider round to switch on their own after
                // a moment.
                if self.t > 120 && s.speed < 3.5 && self.t < 160 {
                    pad.buttons |= btn::A;
                }
                if s.switch != self.switch_at_start && self.t <= 120 {
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
                if self.seen.flips < 2 {
                    (pad.cstick_x, pad.cstick_y) =
                        flick_end(game, "Kickflip", s.mirrored(), UP_RIGHT);
                }
                if rolling {
                    next(self, Tour::LateOllie, sw);
                }
            }
            Tour::LateOllie => {
                // Late flips are the state graphs' (InAir/Lateflip).
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
                // (Pushed only up to 4.5 m/s and sliding from half a second in, once over the
                // slide's 1.2 m/s, for 40 ticks: the rigid board leaves the late flip heading for a
                // wall, and from a fakie roll the push turns round first.)
                if self.t < 30 && s.speed < 4.5 {
                    pad.buttons |= btn::A;
                } else if self.t >= 30 && self.held < 40 && (self.held > 0 || s.speed > 2.0) {
                    // Main stick fully back and to the right.
                    (pad.stick_x, pad.stick_y) = (50, -87);
                    self.held += 1;
                }
                if self.t > 120 && self.held >= 40 {
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
                    next(self, Tour::LedgeFind, sw);
                }
            }
            Tour::LedgeFind => {
                self.rail = find_rail(game, Want::Ledge, 8.0, None);
                match self.rail {
                    Some(r) => {
                        // Beside the ledge's line, a little before it starts.
                        let low = -r.high;
                        let at = r.start - r.dir * 3.0 + low * LEDGE_SIDE;
                        let p = r.start;
                        let g = game.terrain().ground(at.x, at.z, p.y - 0.1);
                        self.place = Some(Placement {
                            pos: v3(at.x, g.height, at.z),
                            heading: yaw_of(r.dir),
                            speed: 4.0,
                            lift: 0.0,
                        });
                        next(self, Tour::LedgeRide, sw);
                    }
                    None => next(self, Tour::CopingFind, sw),
                }
            }
            Tour::LedgeRide => {
                let Some(r) = self.rail else {
                    next(self, Tour::CopingFind, sw);
                    return pad;
                };
                pad.stick_x = Self::hold_line(s, &r, LEDGE_SIDE);
                // Load and pop once riding along it (the state graphs' wind-up takes a while longer):
                // the landing prediction steers onto the ledge.
                let along = (s.pos - r.start).dot(r.dir);
                if along > -2.0 || self.wind > 0 {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, 10, &[UP]);
                }
                if !rolling {
                    self.popped(Tour::LedgeAir, sw);
                }
                if self.t > 240 {
                    next(self, Tour::CopingFind, sw);
                }
            }
            Tour::LedgeAir => {
                if s.grinding() {
                    self.seen.ledge = true;
                    next(self, Tour::LedgeGrind, sw);
                } else if rolling || self.t > 120 {
                    next(self, Tour::CopingFind, sw);
                }
            }
            Tour::LedgeGrind => {
                // A moment of 50-50, then the C-stick back tips it onto the back truck: a 5-0.
                if (15..60).contains(&self.t) {
                    pad.cstick_y = -80;
                }
                if let Some(g) = &s.grind
                    && g.family == crate::skater::Family::FiveO
                    && !g.front
                {
                    self.seen.five_o = true;
                }
                if !s.grinding() && self.held == 0 {
                    self.held = self.t;
                }
                if (self.held > 0 && rolling && self.t > self.held + 30) || self.t > 600 {
                    next(self, Tour::CopingFind, sw);
                }
            }
            Tour::CopingFind => {
                let pick = find_rail(game, Want::Coping, 6.0, None);
                match pick {
                    Some(r) => {
                        self.rail = Some(r);
                        let (p, _) = game.terrain().rails()[r.rail].at(r.s + 0.5);
                        let at = p + r.high * DECK_IN;
                        let g = game.terrain().ground(at.x, at.z, p.y + 0.1);
                        // Along the coping, a little towards the edge.
                        let toward =
                            wrap_angle(yaw_of(r.dir) + 0.1 * r.dir.cross(-r.high).y.signum());
                        self.place = Some(Placement {
                            pos: v3(at.x, g.height, at.z),
                            heading: toward,
                            speed: 3.5,
                            lift: 0.0,
                        });
                        next(self, Tour::CopingRide, sw);
                    }
                    None => next(self, Tour::SlideFind, sw),
                }
            }
            Tour::CopingRide => {
                if s.grinding() {
                    self.seen.coping = true;
                }
                if !s.grinding() && self.seen.coping && self.held == 0 {
                    self.held = self.t;
                }
                if (self.held > 0 && rolling && self.t > self.held + 30) || self.t > 360 {
                    next(self, Tour::SlideFind, sw);
                }
            }
            Tour::SlideFind => {
                let avoid = self.rail.map(|r| r.rail);
                let pick = find_rail(game, Want::Slide, 4.0, avoid)
                    .or_else(|| find_rail(game, Want::Slide, 4.0, None));
                match pick {
                    Some(r) => {
                        self.rail = Some(r);
                        // On the ledge's line, a little back from its start, on the low ground.
                        let low = -r.high;
                        let at = r.start - r.dir * 2.2 + low * 0.05;
                        let g = game.terrain().ground(at.x, at.z, r.start.y - 0.1);
                        self.place = Some(Placement {
                            pos: v3(at.x, g.height, at.z),
                            heading: yaw_of(r.dir),
                            speed: 4.0,
                            lift: 0.0,
                        });
                        next(self, Tour::SlideRide, sw);
                    }
                    None => next(self, Tour::Gesture, sw),
                }
            }
            Tour::SlideRide => {
                // Pop just before the ledge's end.
                if self.t > 6 {
                    (pad.cstick_x, pad.cstick_y) = self.wind_and_flick(game, BACK, 4, &[UP]);
                }
                if !rolling {
                    self.popped(Tour::SlideAir, sw);
                }
                if self.t > 240 {
                    next(self, Tour::Gesture, sw);
                }
            }
            Tour::SlideAir => {
                // A quarter turn in the air (the main stick for a moment): across the ledge's edge.
                if self.t < 9 && !s.grinding() {
                    pad.stick_x = 100;
                }
                if let Some(g) = &s.grind
                    && g.family != crate::skater::Family::FiftyFifty
                    && g.family != crate::skater::Family::FiveO
                {
                    self.seen.slide = true;
                }
                if s.rolling() && self.t > 2 && self.held == 0 {
                    self.held = self.t;
                }
                if (self.held > 0 && self.t > self.held + 30) || self.t > 600 {
                    next(self, Tour::Gesture, sw);
                }
            }
            Tour::Gesture => {
                // The d-pad up for a moment while rolling: the state graphs play the gesture.
                if s.speed.abs() < 2.0 && self.t < 40 {
                    pad.buttons |= btn::A;
                }
                if (30..90).contains(&self.t) {
                    pad.buttons |= btn::DPAD_UP;
                }
                if game
                    .graph
                    .as_ref()
                    .is_some_and(|g| g.runner.s.gesture.is_some())
                {
                    self.seen.gesture = true;
                }
                if self.t > 180 {
                    // Off the board facing clear, flat ground (the walk ahead and its jump need room).
                    let yaw = clear_heading(game, s.travel_yaw(), 8.0).unwrap_or(s.travel_yaw());
                    self.place = Some(Placement {
                        pos: s.pos + Vec3::UP * 0.1,
                        heading: yaw,
                        speed: 0.0,
                        lift: 0.0,
                    });
                    next(self, Tour::Walk, sw);
                }
            }
            Tour::Walk => {
                // Y off the board, a short walk ahead and a jump, then Y back on.
                if (10..13).contains(&self.t) || (260..263).contains(&self.t) {
                    pad.buttons |= btn::Y;
                }
                if (70..170).contains(&self.t) {
                    pad.stick_y = 60;
                }
                if (150..153).contains(&self.t) {
                    pad.buttons |= btn::B;
                }
                if s.walking() {
                    self.held += 1;
                    if s.biped.jumped {
                        self.walk_jumped = true;
                    }
                }
                if self.t > 263 && s.rolling() {
                    self.seen.walked = self.held > 60 && self.walk_jumped;
                    next(self, Tour::RailFind, sw);
                } else if self.t > 420 {
                    next(self, Tour::RailFind, sw);
                }
            }
            Tour::RailFind => match find_thin_rail(game, 3.0) {
                Some((ri, start, dir, _)) => {
                    // Dropped onto the rail's high end, moving along it.
                    self.rail = Some(RailPick {
                        rail: ri,
                        s: 0.0,
                        dir,
                        high: Vec3::UP,
                        start,
                    });
                    let at = start - dir * 0.6 + Vec3::UP * 0.45;
                    self.place = Some(Placement {
                        pos: at,
                        heading: yaw_of(dir),
                        speed: 4.0,
                        lift: 0.5,
                    });
                    next(self, Tour::RailGrind, sw);
                }
                None => next(self, Tour::FlipFind, sw),
            },
            Tour::RailGrind => {
                if s.grinding() {
                    self.held = self.t;
                }
                // Off the rail: it went to the end (the log says why: `grind: off (off the end)`).
                if self.held > 0 && !s.grinding() {
                    if let Some(r) = self.rail {
                        let rail = &game.terrain().rails()[r.rail];
                        let end = rail.points.iter().any(|p| (*p - s.pos).length() < 1.0);
                        self.seen.handrail |= end;
                    }
                    if rolling || self.t > self.held + 60 {
                        next(self, Tour::FlipFind, sw);
                    }
                }
                if self.t > 360 {
                    next(self, Tour::FlipFind, sw);
                }
            }
            Tour::FlipFind => {
                // The demo park's kicker run-in; an imported park has no kicker the tour knows of, so
                // the rider is launched from its spawn instead.
                self.place = Some(if game.on_demo_park() {
                    Placement {
                        pos: v3(0.0, 0.0, -16.0),
                        heading: 0.0,
                        speed: 2.0,
                        lift: 0.0,
                    }
                } else {
                    // Where the rider is, towards clear, flat ground.
                    let p = s.pos;
                    let yaw = clear_heading(game, s.travel_yaw(), 8.0).unwrap_or(s.travel_yaw());
                    Placement {
                        pos: p + Vec3::UP * 0.3,
                        heading: yaw,
                        speed: 3.0,
                        lift: 6.5,
                    }
                });
                next(self, Tour::FlipRide, sw);
            }
            Tour::FlipRide => {
                // Push up to speed, load the C-stick and ollie off the kicker's lip.
                if rolling && s.speed < 6.5 {
                    pad.buttons |= btn::A;
                }
                // Wound up well before the lip (the state graphs' wind-up takes a moment), flicked at
                // it.
                let wound = Self::graph_state(game).is_none_or(|st| st.contains("Antic"));
                if s.pos.z > -8.0 && rolling {
                    (pad.cstick_x, pad.cstick_y) = BACK;
                }
                if s.pos.z > -4.3 && rolling && wound {
                    (pad.cstick_x, pad.cstick_y) = UP;
                }
                if !rolling {
                    next(self, Tour::FlipAir, sw);
                }
                if self.t > 600 {
                    next(self, Tour::BailOllie, sw);
                }
            }
            Tour::FlipAir => {
                // Grab, the left stick's back flip shape, and hold the grab until nearly round (the
                // flip slows to a stop once let go).
                let turned = s.body_flip.map_or(0.0, |f| f.angle.abs());
                let release = s
                    .body_flip
                    .is_some_and(|f| f.angle.abs() > crate::math::TAU - 1.1);
                if !release && self.held == 0 {
                    pad.trig_r = 255;
                }
                if release {
                    self.held = self.t;
                }
                // Straight after take-off: the flip gesture counts only early in the air.
                if let Some((x, y)) = self.replay_held(game, "skaterls", "BackFlip", 1, 2) {
                    (pad.stick_x, pad.stick_y) = (x, y);
                }
                if turned > 0.0 {
                    self.wind = self.wind.max((turned * 10.0) as u32);
                }
                if !s.is_airborne() {
                    if rolling && self.wind as f32 / 10.0 > crate::math::PI {
                        self.seen.backflip = true;
                    }
                    if !s.is_airborne() && self.t > 2 {
                        next(self, Tour::BailOllie, sw);
                    }
                }
                if self.t > 300 {
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
                if s.bailed() {
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

    /// The rider was moved to another location: the tour starts over there (from its spawn) after a short
    /// rest.
    pub fn new_location(&mut self) {
        self.tour = TrickTour::new();
        self.shown = "";
        self.restart = false;
        self.rest = 60;
    }

    /// Where the tour wants the rider put (the caller does it: `Game::place_rider`).
    pub fn take_place(&mut self) -> Option<Placement> {
        self.tour.take_place()
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
                if game.skater.rolling() {
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
                     {} flips held, late flip {}, grab variant {}, ledge grind {}, 5-0 {}, coping {}, \
                     slide {}, gesture {}, handrail {}, back flip {}, walked {})",
                    self.rounds,
                    seen.manual as f32 / 60.0,
                    seen.kick_turn,
                    seen.revert,
                    seen.auto_switch,
                    seen.flips,
                    seen.late_flip,
                    seen.grab_variant,
                    seen.ledge,
                    seen.five_o,
                    seen.coping,
                    seen.slide,
                    seen.gesture,
                    seen.handrail,
                    seen.backflip,
                    seen.walked
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
            if let Some(p) = auto.take_place() {
                game.place(&p);
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
