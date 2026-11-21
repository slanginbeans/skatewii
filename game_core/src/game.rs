//! One session in the park: input -> flick reader -> skater -> camera, then drawing.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::camera::ChaseCam;
use crate::character::{self, Character};
use crate::collision;
use crate::devlog::DevLog;
use crate::flick::{FlickInput, FlickOut};
use crate::gesture;
use crate::gfx::{Gfx, MeshBuilder, SCREEN_W, Vertex, rgb};
use crate::hud;
use crate::input::{Pad, PadRaw, flag};
use crate::math::{Mat3, PI, TAU, Xform, sinf, v3};
use crate::model::Models;
use crate::pack::Pack;
use crate::park::{Park, Terrain};
use crate::rider::{self, Rider};
use crate::skater::{ClipTiming, Event, Mode, STEP, Skater, Stance};
use crate::tuning::Tuning;
use crate::world::ImportedPark;

const SKY: u32 = rgb(150, 196, 232);
/// Imported character space (Skate 3: Y up, travelling along +Z) to board space (+Z nose, Y up).
const IMPORTED_TO_BOARD: Mat3 = Mat3::IDENTITY;
const POPUP_TIME: f32 = 1.8;
const HINT_TIME: f32 = 12.0;
/// The simulation's fixed step: Skate 3's tuning is per 1/60 s tick, so the skater, the flick reader and
/// the camera always step at 60 Hz whatever the TV runs at (PAL50 draws between two ticks).
pub const TICK: f32 = 1.0 / 60.0;
/// A drawn skater further than this (m) from where it was a tick ago was teleported: not blended.
const TELEPORT: f32 = 2.0;

/// What is drawn of one simulation tick: the board's frame and the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    base: Xform,
    cam: crate::gfx::Camera,
}

/// `a` to `b` by `k` (0..1): positions blended, the frame turned part way (re-squared).
fn blend_xform(a: &Xform, b: &Xform, k: f32) -> Xform {
    if k >= 1.0 || (b.pos - a.pos).length() > TELEPORT {
        return *b;
    }
    let up = a.rot.cols[1]
        .lerp(b.rot.cols[1], k)
        .normalize_or(b.rot.cols[1]);
    let nose = a.rot.cols[2].lerp(b.rot.cols[2], k);
    let nose = (nose - up * nose.dot(up)).normalize_or(b.rot.cols[2]);
    Xform::new(
        Mat3::from_cols(up.cross(nose), up, nose),
        a.pos.lerp(b.pos, k),
    )
}

/// Frame timing measured by the platform, over windows of a second: the CPU's own work (update + render, not
/// counting waits), the time spent waiting for the GPU to finish, the time idle until the next refresh, and
/// frames that missed their refresh.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameCost {
    /// Running average and the worst frame of the last full window (microseconds).
    pub avg_us: f32,
    pub peak_us: u32,
    pub gpu_avg_us: f32,
    pub idle_avg_us: f32,
    /// Frames that missed their refresh in the last full window.
    pub dropped: u32,
    window_peak: u32,
    window_dropped: u32,
    frames: u32,
    started: bool,
}

impl FrameCost {
    /// Frames per window (one second at 60 Hz).
    pub const WINDOW: u32 = 60;

    /// One frame: CPU work, GPU wait, idle wait (microseconds), and refreshes missed since the last frame.
    pub fn add(&mut self, cpu_us: u32, gpu_us: u32, idle_us: u32, missed: u32) {
        let first = !self.started;
        self.started = true;
        let ease = |avg: f32, v: u32| {
            if first {
                v as f32
            } else {
                avg + (v as f32 - avg) * 0.05
            }
        };
        self.avg_us = ease(self.avg_us, cpu_us);
        self.gpu_avg_us = ease(self.gpu_avg_us, gpu_us);
        self.idle_avg_us = ease(self.idle_avg_us, idle_us);
        self.window_peak = self.window_peak.max(cpu_us);
        self.window_dropped += missed;
        self.frames += 1;
        if self.frames == Self::WINDOW {
            self.peak_us = self.window_peak;
            self.dropped = self.window_dropped;
            self.window_peak = 0;
            self.window_dropped = 0;
            self.frames = 0;
        }
    }
}

/// Where the CPU went in the last frame (microseconds), when the platform gives the game a clock.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SectionTimes {
    /// Input, physics, camera.
    pub physics_us: u32,
    /// Posing and skinning the imported skater.
    pub skin_us: u32,
    /// Building and submitting the frame's draws (not waiting for the GPU).
    pub render_us: u32,
    /// Park meshes and vertices drawn last frame.
    pub park_draws: u32,
    pub park_verts: u32,
    /// Skate 3's state graphs (input intents, both graphs, animation advance), part of `physics_us`.
    pub graph_us: u32,
}

struct Popup {
    title: String,
    detail: String,
    rgba: u32,
    time: f32,
}

pub struct Game {
    pub park: Park,
    park_mesh: Vec<Vertex>,
    /// A park imported from the player's disc, ridden and drawn instead of the demo park when loaded.
    imported_park: Option<ImportedPark>,
    models: Models,
    /// The player's imported skater, drawn instead of the box placeholder when loaded.
    rider: Option<Rider>,
    /// Skate 3's motion library (`fg_import motion`): clip timing, attribute tracks, animation trees and
    /// poses. Loaded but not played yet.
    pub motion: Option<crate::motion::Library<'static>>,
    /// Skate 3's action and motion graphs from the motion pack (`motion/graph`), run every tick when
    /// present (see `graph`).
    pub graph: Option<crate::graph::drive::Driver>,
    /// Gesture files from the rules pack, by name (for the graph's input).
    gesture_sets: Vec<(&'static str, Vec<gesture::Shape>)>,
    pub skater: Skater,
    flick: FlickInput,
    /// This frame's flick-stick reading (for the preview and tests).
    pub flick_out: FlickOut,
    /// Riding numbers: built in, or Skate 3's from a rules pack.
    pub tuning: Tuning,
    pub stance: Stance,
    cam: ChaseCam,
    pub score: u32,
    popup: Option<Popup>,
    prev_held: u32,
    clock: f32,
    connected: bool,
    /// Events from the last `update` (cleared every frame). Handy for tests and the preview tool.
    pub events: Vec<Event>,
    /// Set by the platform each frame when it measures CPU time; shown in a corner of the HUD.
    pub frame_cost: Option<FrameCost>,
    /// A status line shown under the controls hint (for example whether the imported skater loaded).
    pub notice: Option<String>,
    /// Development log lines from the last updates; the platform takes them (`log.drain(..)`) and writes
    /// them out. Capped while nobody takes them.
    pub log: Vec<String>,
    devlog: DevLog,
    status_timer: f32,
    /// A microsecond clock from the platform, for [`Game::times`] (none: no timing).
    pub timer: Option<fn() -> u32>,
    pub times: SectionTimes,
    render_times: core::cell::Cell<(u32, u32, u32)>,
    /// Simulated time minus shown time (s): ticks run until the simulation is at or just past the frame.
    ahead: f32,
    /// Ticks run since the start.
    pub ticks: u64,
    /// The view one tick before the current one, and where between the two the frame is drawn (0..1).
    prev_view: Option<View>,
    alpha: f32,
}

/// Lines kept while nobody takes them.
const LOG_CAP: usize = 512;
/// Seconds between status lines.
const STATUS_EVERY: f32 = 1.0;

/// The imported park if there is one, else the demo park (a free function so callers can borrow other
/// fields of the game at the same time).
fn terrain<'a>(imported: &'a Option<ImportedPark>, park: &'a Park) -> &'a dyn Terrain {
    match imported {
        Some(p) => &p.mesh,
        None => park,
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Game {
        let park = Park::demo();
        let skater = Skater::spawn(&park);
        Game {
            park_mesh: park.build_mesh(),
            models: Models::build(),
            cam: ChaseCam::new(&skater),
            park,
            skater,
            flick: FlickInput::new(),
            flick_out: FlickOut::default(),
            tuning: Tuning::BUILT_IN,
            stance: Stance::Goofy,
            score: 0,
            popup: None,
            prev_held: 0,
            clock: 0.0,
            connected: true,
            events: Vec::new(),
            frame_cost: None,
            notice: None,
            rider: None,
            motion: None,
            graph: None,
            gesture_sets: Vec::new(),
            imported_park: None,
            log: Vec::new(),
            devlog: DevLog::new(),
            status_timer: 0.0,
            timer: None,
            times: SectionTimes::default(),
            render_times: core::cell::Cell::new((0, 0, 0)),
            ahead: 0.0,
            ticks: 0,
            prev_view: None,
            alpha: 1.0,
        }
    }

    /// Uses an imported skater pack (`fg_import skater`) instead of the placeholder model. The bytes stay
    /// borrowed for the rest of the game (the Wii reads its textures in place).
    pub fn load_skater(&mut self, pack: &'static [u8]) -> Result<(), character::Error> {
        self.rider = Some(Rider::new(Character::parse(pack)?));
        Ok(())
    }

    /// Loads a motion library pack (`fg_import motion`). The bytes stay borrowed for the rest of the game.
    /// Its push and brake clips also time the rider's feet in the physics ([`ClipTiming`]).
    pub fn load_motion(
        &mut self,
        pack: &'static [u8],
    ) -> Result<&crate::motion::Library<'static>, crate::motion::LibraryError> {
        let lib = crate::motion::Library::parse(pack)?;
        self.skater.clips = ClipTiming::from_motion(&lib.db);
        self.graph = None;
        if let Ok(p) = Pack::parse(pack)
            && let Some(e) = p.find("motion/graph")
        {
            let t0 = self.timer.map_or(0, |f| f());
            match crate::graph::drive::Driver::new(e.data, &lib.db, self.gesture_sets()) {
                Ok(mut d) => {
                    d.load_us = self.timer.map_or(0, |f| f()).wrapping_sub(t0);
                    self.graph = Some(d)
                }
                Err(e) => self.notice = Some(format!("state graphs unreadable ({e:?})")),
            }
        }
        Ok(self.motion.insert(lib))
    }

    pub fn rider(&self) -> Option<&Rider> {
        self.rider.as_ref()
    }

    /// Rides an imported park (`fg_import park`) instead of the demo park, starting at its spawn.
    pub fn load_park(&mut self, pack: &'static [u8]) -> Result<(), collision::Error> {
        let p = ImportedPark::parse(pack)?;
        let clips = self.skater.clips;
        self.skater = Skater::spawn_with(&p.mesh, self.tuning, self.stance);
        self.skater.clips = clips;
        self.cam = ChaseCam::new(&self.skater);
        self.imported_park = Some(p);
        Ok(())
    }

    /// Uses a rules pack (`fg_import rules`): Skate 3's tuning values and flick shapes from the player's
    /// disc. Returns how many tuning values and flick shapes it supplied.
    pub fn load_rules(&mut self, pack: &[u8]) -> Result<(usize, usize), crate::pack::Error> {
        let pack = Pack::parse(pack)?;
        let mut values = 0;
        if let Some(e) = pack.find("rules/tuning") {
            values = self.tuning.apply(e.data);
        }
        let mut shapes = 0;
        if let Some(e) = pack.find("rules/gestures/skater")
            && let Some(s) = gesture::parse(e.data)
            && !s.is_empty()
        {
            shapes = s.len();
            self.flick = FlickInput::with_shapes(s);
        }
        self.gesture_sets.clear();
        for name in crate::graph::input::RIGHT_SETS
            .iter()
            .chain(&crate::graph::input::LEFT_SETS)
        {
            if let Some(e) = pack.find(&format!("rules/gestures/{name}"))
                && let Some(s) = gesture::parse(e.data)
            {
                self.gesture_sets.push((name, s));
            }
        }
        let sets = self.gesture_sets();
        if let Some(g) = &mut self.graph {
            g.set_gestures(sets);
        }
        self.skater.tuning = self.tuning;
        self.cam = ChaseCam::new(&self.skater);
        Ok((values, shapes))
    }

    /// The gesture files for the graph's input: the disc's, else flatground's own flick shapes.
    fn gesture_sets(&self) -> Vec<(&'static str, Vec<gesture::Shape>)> {
        if self.gesture_sets.is_empty() {
            alloc::vec![("skater", gesture::built_in())]
        } else {
            self.gesture_sets.clone()
        }
    }

    /// Sets the rider's natural stance (Skate 3's default is goofy).
    pub fn set_stance(&mut self, stance: Stance) {
        self.stance = stance;
        self.skater.stance = stance;
    }

    pub fn imported_park(&self) -> Option<&ImportedPark> {
        self.imported_park.as_ref()
    }

    /// What the skater rides on: the imported park if there is one, else the demo park.
    pub fn terrain(&self) -> &dyn Terrain {
        terrain(&self.imported_park, &self.park)
    }

    /// One displayed frame of `dt` seconds (1/60 on NTSC and PAL60, 1/50 on PAL50): runs as many fixed
    /// [`TICK`]s as it takes to reach the frame's time (one at 60 Hz; one or two at 50 Hz), then poses the
    /// rider once. A PAL50 frame is drawn between the last two ticks.
    pub fn update(&mut self, raw: &PadRaw, dt: f32) {
        self.connected = raw.flags & flag::CONNECTED != 0;
        self.events.clear();
        self.ahead -= dt;
        // A little slack so float error never runs (or skips) a tick at 60 Hz.
        while self.ahead < -1e-4 {
            self.prev_view = Some(self.view());
            self.tick(raw);
            self.ahead += TICK;
        }
        self.alpha = (1.0 - self.ahead / TICK).clamp(0.0, 1.0);
        let now = |c: Option<fn() -> u32>| c.map_or(0, |f| f());
        let t1 = now(self.timer);
        if self.rider.is_some() {
            let rot = self.imported_xform().rot;
            let s = &self.skater;
            let ground: &dyn Terrain = terrain(&self.imported_park, &self.park);
            let ground = ground.ground(s.pos.x, s.pos.z, s.pos.y + STEP).height;
            let land_in = rider::time_to_land(s.pos.y - ground, s.vel.y, s.tuning.gravity);
            if let Some(r) = &mut self.rider {
                r.update(&self.skater, self.clock, land_in, dt, rot);
            }
        }
        let t2 = now(self.timer);
        self.times.skin_us = t2.wrapping_sub(t1);
        let (render_us, park_draws, park_verts) = self.render_times.get();
        self.times.render_us = render_us;
        self.times.park_draws = park_draws;
        self.times.park_verts = park_verts;

        for e in &self.events {
            match e {
                Event::Landed { name, points } => {
                    self.score += points;
                    self.popup = Some(Popup {
                        title: name.clone(),
                        detail: format!("+{points}"),
                        rgba: rgb(255, 230, 90),
                        time: 0.0,
                    });
                }
                Event::Bailed => {
                    self.popup = Some(Popup {
                        title: "BAIL!".into(),
                        detail: String::new(),
                        rgba: rgb(255, 90, 70),
                        time: 0.0,
                    });
                }
                _ => {}
            }
        }
        if let Some(p) = &mut self.popup {
            p.time += dt;
            if p.time > POPUP_TIME {
                self.popup = None;
            }
        }
    }

    /// One fixed 1/60 s step of the simulation: input, flick reader (so flicks are sampled at 60 Hz, as
    /// Skate 3 reads them), skater, camera, log.
    fn tick(&mut self, raw: &PadRaw) {
        let dt = TICK;
        let pad = Pad::from_raw(raw, self.prev_held);
        self.prev_held = raw.buttons;
        self.clock += dt;
        self.ticks += 1;
        let mut flick = self.flick.update(pad.flick, self.tuning.flick_misses);
        let rolling = self.skater.mode == Mode::Rolling;
        let grinding = self.skater.mode == Mode::Grind;
        if !(rolling || grinding) {
            flick.flick = None;
        }
        self.flick_out = flick;
        let now = |c: Option<fn() -> u32>| c.map_or(0, |f| f());
        let t0 = now(self.timer);
        let ground: &dyn Terrain = terrain(&self.imported_park, &self.park);
        if let (Some(g), Some(lib)) = (&mut self.graph, &self.motion) {
            let s = &self.skater;
            let land_in = matches!(s.mode, Mode::Air { .. }).then(|| {
                let h = ground.ground(s.pos.x, s.pos.z, s.pos.y + STEP).height;
                rider::time_to_land(s.pos.y - h, s.vel.y, s.tuning.gravity)
            });
            let world = crate::graph::drive::Driver::world(s, land_in, g.input.idle());
            let gates = crate::graph::input::Gates {
                late_flips_blocked: s.trick.is_some_and(|t| t.progress() < 1.0),
                finger_flips_blocked: s.grab.is_none(),
                reverts_blocked: false,
                grinding: s.mode == Mode::Grind,
            };
            g.tick(&lib.db, &pad, world, gates, dt, self.timer);
            self.skater.graph = Some(g.control(&lib.db, &self.tuning));
            self.times.graph_us = g.cost_us;
            if self.log.len() < LOG_CAP {
                if let Some(state) = g.state_change() {
                    self.log.push(format!("graph: {state}"));
                }
                for n in g.new_unsupported() {
                    self.log.push(format!("graph: unsupported {n}"));
                }
            }
        }
        let mut events = Vec::new();
        self.skater.step(ground, &pad, &flick, dt, &mut events);
        if events.contains(&Event::Respawned) {
            self.flick.reset();
        }
        self.cam.update(&self.skater, ground, dt);
        self.times.physics_us = now(self.timer).wrapping_sub(t0);
        // Development log: what happened this tick, and a status line now and then.
        if self.log.len() < LOG_CAP {
            let cam = self.cam.distance();
            self.devlog.observe(
                &self.skater,
                raw,
                &self.flick_out,
                &events,
                cam,
                &mut self.log,
            );
            self.status_timer += dt;
            if self.status_timer >= STATUS_EVERY {
                self.status_timer = 0.0;
                self.log.push(self.devlog.status(&self.skater, cam));
            }
        }
        self.events.extend(events);
    }

    /// The board's frame and the camera as they are now.
    fn view(&self) -> View {
        View {
            base: self.skater_base(),
            cam: self.cam.camera(),
        }
    }

    /// The board's frame to draw: between the last two ticks on PAL50.
    fn drawn_base(&self) -> Xform {
        let cur = self.skater_base();
        match &self.prev_view {
            Some(p) => blend_xform(&p.base, &cur, self.alpha),
            None => cur,
        }
    }

    /// The camera to draw with (blended like the skater).
    pub fn drawn_camera(&self) -> crate::gfx::Camera {
        let cur = self.cam.camera();
        match &self.prev_view {
            Some(p) if self.alpha < 1.0 && (cur.eye - p.cam.eye).length() < TELEPORT => {
                crate::gfx::Camera {
                    eye: p.cam.eye.lerp(cur.eye, self.alpha),
                    target: p.cam.target.lerp(cur.target, self.alpha),
                    ..cur
                }
            }
            _ => cur,
        }
    }

    /// Where between the last two ticks the frame is drawn (1: on the latest).
    pub fn blend(&self) -> f32 {
        self.alpha
    }

    /// The drawn skater's position (blended between ticks).
    pub fn drawn_pos(&self) -> crate::math::Vec3 {
        self.drawn_base().pos
    }

    pub fn render(&self, gfx: &mut dyn Gfx) {
        let now = || self.timer.map_or(0, |f| f());
        let t0 = now();
        let cam = self.drawn_camera();
        gfx.begin_frame(&cam, SKY);
        let (draws, verts) = match &self.imported_park {
            Some(p) => p.draw(gfx, cam.eye),
            None => {
                gfx.draw_tris(&self.park_mesh, &Xform::IDENTITY.to_mat34());
                (1, self.park_mesh.len() as u32)
            }
        };
        self.draw_skater(gfx);
        self.draw_hud(gfx);
        // Before end_frame: that waits for the GPU and the TV, which the platform measures itself.
        self.render_times
            .set((now().wrapping_sub(t0), draws, verts));
        gfx.end_frame();
    }

    /// The skater's board-space frame in the world (heading, spin, slope; powerslides and reverts turn
    /// the board unless the rider's clip does it).
    fn skater_base(&self) -> Xform {
        let s = &self.skater;
        let clip_turns = self.rider.as_ref().is_some_and(|r| r.turns_board());
        let extra = if clip_turns {
            0.0
        } else {
            s.board_yaw_offset()
        };
        // The board's own frame (its nose and the surface, or its up in the air), turned about its up by
        // a powerslide or revert.
        let frame = s.board_frame();
        let up = frame.cols[1];
        let nose = crate::skater::rotate(frame.cols[2], up, extra);
        Xform::new(Mat3::from_cols(up.cross(nose), up, nose), s.pos)
    }

    /// Where the imported character is drawn: board space, tipped over on a bail.
    fn imported_xform(&self) -> Xform {
        // Without bail clips in the pack, show a bail by tipping the rider over.
        let animated = self
            .rider
            .as_ref()
            .is_some_and(|r| r.character.has_clip("bail/fall"));
        let tip = match self.skater.mode {
            Mode::Bailed { time } | Mode::Runout { time } if !animated => {
                Mat3::rot_z(-(time * 5.0).min(1.0) * PI * 0.48)
            }
            _ => Mat3::IDENTITY,
        };
        self.drawn_base()
            .then(&Xform::new(tip, v3(0.0, 0.0, 0.0)))
            .then(&Xform::new(IMPORTED_TO_BOARD, v3(0.0, 0.0, 0.0)))
    }

    fn draw_skater(&self, gfx: &mut dyn Gfx) {
        if let Some(r) = &self.rider {
            let m = self.imported_xform().to_mat34();
            for p in &r.character.parts {
                gfx.draw_indexed(&p.out, &p.indices, p.texture.as_ref(), &m);
            }
            return;
        }
        let s = &self.skater;
        let base = self.drawn_base();

        // Board: lifts and turns under the feet while a trick plays.
        let (mut lift, mut board_rot) = (0.0, Mat3::IDENTITY);
        if let Some(t) = &s.trick {
            let p = t.progress();
            let (flip, shove) = t.flick.trick.motion();
            lift = 0.28 * sinf(PI * p);
            board_rot =
                Mat3::rot_y(shove * PI * p).mul(&Mat3::rot_z(flip * t.flips as f32 * TAU * p));
        }
        let pivot = 0.09;
        let board = base
            .then(&Xform::new(board_rot, v3(0.0, pivot + lift, 0.0)))
            .then(&Xform::new(Mat3::IDENTITY, v3(0.0, -pivot, 0.0)));
        gfx.draw_tris(&self.models.board, &board.to_mat34());

        // Rider: tucks with the board, falls over on a bail.
        let (rider_pos, rider_rot) = match s.mode {
            Mode::Bailed { time } | Mode::Runout { time } => (
                v3(0.0, 0.0, 0.0),
                Mat3::rot_z(-(time * 5.0).min(1.0) * PI * 0.48),
            ),
            _ => (v3(0.0, lift * 0.7, 0.0), Mat3::IDENTITY),
        };
        let rider = base.then(&Xform::new(rider_rot, rider_pos));
        let tucked = s.crouch > 0.5 || (s.is_airborne() && lift > 0.05);
        let mesh = if tucked {
            &self.models.rider_crouched
        } else {
            &self.models.rider
        };
        gfx.draw_tris(mesh, &rider.to_mat34());
    }

    fn draw_hud(&self, gfx: &mut dyn Gfx) {
        let mut m = MeshBuilder::new();
        let white = rgb(255, 255, 255);
        hud::text(
            &mut m,
            24.0,
            22.0,
            4.0,
            &format!("SCORE {}", self.score),
            white,
        );
        let kmh = (self.skater.speed.abs() * 3.6) as u32;
        let speed = format!("{kmh} KM/H");
        hud::text(
            &mut m,
            SCREEN_W - 24.0 - hud::text_width(&speed, 4.0),
            22.0,
            4.0,
            &speed,
            white,
        );

        if let Some(p) = &self.popup {
            // Pop in with a slight scale-up.
            let px = 6.0 + 2.0 * (1.0 - (p.time * 8.0).min(1.0));
            hud::text_centered(&mut m, SCREEN_W / 2.0, 120.0, px, &p.title, p.rgba);
            if !p.detail.is_empty() {
                hud::text_centered(
                    &mut m,
                    SCREEN_W / 2.0,
                    120.0 + px * 7.0,
                    4.0,
                    &p.detail,
                    white,
                );
            }
        }
        if let Some(c) = &self.frame_cost {
            // The CPU's own work against the 16.7 ms frame at 60 Hz; GPU: waiting for it to finish drawing.
            let ms = |us: f32| us / 1000.0;
            let line = format!(
                "CPU {:.1} PEAK {:.1}  GPU {:.1}  DROP {}",
                ms(c.avg_us),
                ms(c.peak_us as f32),
                ms(c.gpu_avg_us),
                c.dropped
            );
            let over = c.avg_us + c.gpu_avg_us > 14_000.0 || c.dropped > 0;
            let colour = if over {
                rgb(255, 120, 100)
            } else {
                rgb(200, 255, 200)
            };
            hud::text(&mut m, 24.0, 52.0, 2.5, &line, colour);
        }
        if let Some(n) = &self.notice
            && self.clock < HINT_TIME
        {
            // As big as fits across the screen.
            let px = ((SCREEN_W - 32.0) / (4.0 * n.chars().count().max(1) as f32)).min(3.0);
            hud::text_centered(&mut m, SCREEN_W / 2.0, 366.0, px, n, rgb(255, 230, 90));
        }
        if !self.connected {
            hud::text_centered(
                &mut m,
                SCREEN_W / 2.0,
                230.0,
                4.0,
                "CONNECT A GAMECUBE PAD TO PORT 1",
                rgb(255, 120, 100),
            );
        } else if self.clock < HINT_TIME {
            for (i, line) in [
                "A PUSH  B MONGO  A+B HIPPIE JUMP  X BRAKE",
                "STICK STEER - FULL BACK + SIDE: POWERSLIDE",
                "C-STICK: PULL BACK, FLICK UP TO OLLIE",
                "FLICK OR ROLL FOR FLIPS - HOLD LIGHT: MANUAL",
            ]
            .iter()
            .enumerate()
            {
                hud::text_centered(
                    &mut m,
                    SCREEN_W / 2.0,
                    392.0 + 18.0 * i as f32,
                    3.0,
                    line,
                    white,
                );
            }
        }
        if let Some(man) = &self.skater.manual {
            // Balance meter: keep the marker in the middle.
            let (w, x, y) = (160.0, SCREEN_W / 2.0 - 80.0, 300.0);
            m.rect_2d(x, y, w, 8.0, rgb(30, 30, 30));
            m.rect_2d(x + w / 2.0 - 12.0, y, 24.0, 8.0, rgb(70, 140, 70));
            let at = x + w / 2.0 + man.balance.clamp(-1.0, 1.0) * (w / 2.0) - 3.0;
            m.rect_2d(at, y - 4.0, 6.0, 16.0, rgb(255, 230, 90));
        }
        gfx.draw_2d(&m.verts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_cost_averages_and_keeps_last_seconds_peak() {
        let mut c = FrameCost::default();
        c.add(4000, 2000, 10000, 0);
        assert_eq!(c.avg_us, 4000.0, "first sample starts the average");
        assert_eq!(c.gpu_avg_us, 2000.0);
        for _ in 1..59 {
            c.add(4000, 2000, 10000, 0);
        }
        c.add(9000, 2000, 5000, 2); // frame 60 closes the window
        assert_eq!(c.peak_us, 9000);
        assert_eq!(c.dropped, 2);
        assert!(c.avg_us > 4000.0 && c.avg_us < 4500.0, "{}", c.avg_us);
        for _ in 0..60 {
            c.add(4000, 2000, 10000, 0);
        }
        assert_eq!(c.peak_us, 4000, "the spike has left the window");
        assert_eq!(c.dropped, 0);
    }

    #[test]
    fn the_game_logs_what_happens_and_times_itself() {
        fn fake_clock() -> u32 {
            use core::sync::atomic::{AtomicU32, Ordering};
            static T: AtomicU32 = AtomicU32::new(0);
            T.fetch_add(100, Ordering::Relaxed)
        }
        let mut g = Game::new();
        g.timer = Some(fake_clock);
        let mut run = crate::autoplay::KickerRun::new();
        let mut gfx = Null;
        for _ in 0..300 {
            let pad = run.next(&g);
            g.update(&pad, 1.0 / 60.0);
            g.render(&mut gfx);
        }
        assert!(g.log.iter().any(|l| l.starts_with("pop: ")), "{:?}", g.log);
        assert!(
            g.log.iter().any(|l| l.starts_with("landed: ")),
            "{:?}",
            g.log
        );
        assert!(g.log.iter().filter(|l| l.starts_with("status: ")).count() >= 4);
        assert!(g.times.physics_us > 0 && g.times.render_us > 0);
        assert_eq!(g.times.park_draws, 1);
        // Nobody takes the lines: they stop at the cap instead of growing forever.
        for _ in 0..(60 * 600) {
            g.update(&PadRaw::default(), 1.0 / 60.0);
        }
        assert!(g.log.len() <= LOG_CAP + 8);
    }

    fn held(buttons: u32) -> PadRaw {
        PadRaw {
            flags: flag::CONNECTED,
            buttons,
            ..Default::default()
        }
    }

    #[test]
    fn the_simulation_ticks_at_60_hz_on_any_tv() {
        // NTSC: one tick a frame, drawn on the latest tick.
        let mut g = Game::new();
        for _ in 0..60 {
            g.update(&held(crate::input::btn::A), 1.0 / 60.0);
            assert_eq!(g.blend(), 1.0);
        }
        assert_eq!(g.ticks, 60);
        // PAL50: 50 frames are 60 ticks (one or two a frame), drawn between the last two.
        let mut p = Game::new();
        let mut doubles = 0;
        for _ in 0..50 {
            let before = p.ticks;
            p.update(&held(crate::input::btn::A), 1.0 / 50.0);
            let n = p.ticks - before;
            assert!((1..=2).contains(&n), "{n}");
            doubles += (n == 2) as u32;
        }
        assert_eq!(p.ticks, 60);
        assert_eq!(doubles, 10);
        // The same input for the same time gives the same skater, whatever the frame rate.
        assert_eq!(g.skater.pos, p.skater.pos);
        assert_eq!(g.skater.speed, p.skater.speed);
    }

    #[test]
    fn pal50_draws_between_the_last_two_ticks() {
        let mut g = Game::new();
        let mut seen_between = false;
        for _ in 0..50 {
            let before = g.skater.pos;
            g.update(&held(crate::input::btn::A), 1.0 / 50.0);
            let (prev, cur) = (g.prev_view.unwrap().base.pos, g.skater.pos);
            let drawn = g.drawn_pos();
            let k = g.blend();
            assert!((0.0..=1.0).contains(&k));
            // On the line between the last two ticks, at `k`.
            assert!((drawn - prev.lerp(cur, k)).length() < 1e-4);
            if k < 0.99 && (cur - before).length() > 1e-3 {
                seen_between = true;
                assert!((drawn - cur).length() > 1e-5);
            }
        }
        assert!(seen_between);
        // A press seen on a two-tick frame is one press, not two.
        let mut g = Game::new();
        g.update(&held(0), 1.0 / 50.0);
        let mut pushes = 0;
        for i in 0..5 {
            let b = if i == 0 { crate::input::btn::A } else { 0 };
            g.update(&held(b), 1.0 / 50.0);
            pushes += g.skater.push.is_some() as u32;
        }
        assert!(pushes >= 1);
    }

    /// Draws nothing.
    struct Null;
    impl Gfx for Null {
        fn begin_frame(&mut self, _cam: &crate::gfx::Camera, _clear: u32) {}
        fn draw_tris(&mut self, _v: &[Vertex], _m: &crate::math::Mat34) {}
        fn draw_indexed(
            &mut self,
            _v: &[crate::gfx::TexVertex],
            _i: &[u16],
            _t: Option<&crate::gfx::Texture>,
            _m: &crate::math::Mat34,
        ) {
        }
        fn draw_2d(&mut self, _v: &[Vertex]) {}
        fn end_frame(&mut self) {}
    }
}
