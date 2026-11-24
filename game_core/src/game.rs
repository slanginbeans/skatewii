//! One session in the park: input -> flick reader -> skater -> camera, then drawing.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::camera::{CamData, CameraMan};
use crate::character::{self, Character};
use crate::collision;
use crate::devlog::DevLog;
use crate::district::District;
use crate::flick::{FlickInput, FlickOut};
use crate::gesture;
use crate::gfx::{Gfx, MeshBuilder, SCREEN_H, SCREEN_W, Vertex, rgb};
use crate::hud;
use crate::input::{Pad, PadRaw, btn, flag};
use crate::math::{Mat3, PI, TAU, Xform, sinf, v3};
use crate::menu::{Choice, Location, Menu, Request};
use crate::model::Models;
use crate::pack::Pack;
use crate::park::{Park, Terrain};
use crate::rider::Rider;
use crate::skater::{ClipTiming, Event, Mode, Skater, Stance};
use crate::tuning::Tuning;
use crate::world::ImportedPark;
use alloc::boxed::Box;

/// How long the drawn character takes to move its origin between the deck and the feet (s).
const FOOT_ORIGIN_BLEND: f32 = 0.2;
const SKY: u32 = rgb(150, 196, 232);
/// Imported character space (Skate 3: Y up, travelling along +Z) to board space (+Z nose, Y up).
const IMPORTED_TO_BOARD: Mat3 = Mat3::IDENTITY;
const POPUP_TIME: f32 = 1.8;
const HINT_TIME: f32 = 12.0;
/// The trick name's text size (pixels per font cell): small enough to leave the skater in view.
const POPUP_PX: f32 = 3.5;
/// The simulation's fixed step: Skate 3's tuning is per 1/60 s tick, so the skater, the flick reader and
/// the camera always step at 60 Hz whatever the TV runs at (PAL50 draws between two ticks).
pub const TICK: f32 = 1.0 / 60.0;
/// The drawn board rises at most this much (m) to keep its wheels out of the ground under them.
const WHEEL_LIFT_MAX: f32 = 0.04;
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
    /// The part of `render_us` spent on the park or district (choosing what's in sight and sending it).
    pub world_us: u32,
    /// Park meshes and vertices drawn last frame.
    pub park_draws: u32,
    pub park_verts: u32,
    /// Skate 3's state graphs (input intents, both graphs, animation advance), part of `physics_us`.
    pub graph_us: u32,
    /// The skater's step on the last tick spent in the air (the landing prediction, spin, flight), part
    /// of `physics_us`, and the most one took since the platform last cleared `air_peak_us`.
    pub air_us: u32,
    pub air_peak_us: u32,
    /// The camera's tick (shot choice, rig, collision, predictors), part of `physics_us`, and the most
    /// one took since the platform last cleared `cam_peak_us`.
    pub cam_us: u32,
    pub cam_peak_us: u32,
}

struct Popup {
    title: String,
    detail: String,
    rgba: u32,
    time: f32,
}

/// After a landing: the drawn board's yaw against the rider's travel, unwrapped (radians), for the log.
struct BoardWatch {
    trick: String,
    frames: u32,
    last: Option<f32>,
    turned: f32,
    lo: f32,
    hi: f32,
}

impl BoardWatch {
    const FRAMES: u32 = 45;
    /// More than this (degrees) is the board turning round, not the landing lining up with the travel.
    const SWING: f32 = 90.0;
}

pub struct Game {
    pub park: Park,
    park_mesh: Vec<Vertex>,
    /// What is ridden and drawn: the demo park, or a park or district imported from the player's disc.
    world: World,
    /// A district was loaded and the cells around its start are still being read: the loading screen is
    /// shown and the rider placed once they are in.
    district_wait: bool,
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
    /// The clips the motion graph plays this frame (reused buffer).
    pose: crate::graph::playback::PoseProgram,
    pub skater: Skater,
    flick: FlickInput,
    /// This frame's flick-stick reading (for the preview and tests).
    pub flick_out: FlickOut,
    /// Riding numbers: built in, or Skate 3's from a rules pack.
    pub tuning: Tuning,
    pub stance: Stance,
    cam: CameraMan,
    pub score: u32,
    popup: Option<Popup>,
    prev_held: u32,
    /// How far the imported character's origin has moved from the deck's centre (the riding clips') to
    /// the ground under the feet (the off-board clips': walking, getting off and on), 0..1.
    foot_origin: f32,
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
    world_us: core::cell::Cell<u32>,
    /// How far a district is drawn (m): the pause menu's setting.
    draw_distance: f32,
    /// After a landing: the drawn board's yaw against the way the rider travels, to log the board turning
    /// under his feet (a player saw it after some tricks; not reproduced off the Wii yet).
    board_watch: Option<BoardWatch>,
    /// The player changed a setting since the platform last saved them.
    settings_changed: bool,
    /// Simulated time minus shown time (s): ticks run until the simulation is at or just past the frame.
    ahead: f32,
    /// Ticks run since the start.
    pub ticks: u64,
    /// The view one tick before the current one, and where between the two the frame is drawn (0..1).
    prev_view: Option<View>,
    alpha: f32,
    /// Where the player can ride (the platform fills it from the location index; the demo park is always
    /// last) and which of them is loaded.
    pub locations: Vec<Location>,
    pub location: usize,
    /// The start or pause menu, when one is open: the simulation waits, and the pad only drives the menu.
    pub menu: Option<Menu>,
    /// What the game asks of the platform (load a location, quit); see [`Game::take_request`].
    request: Option<Request>,
    /// Shown over a black screen while the platform reads a park from the card.
    pub loading: Option<String>,
    /// Buttons held last frame (for START), and buttons still held from a menu: ignored by the riding
    /// until they are let go, so the A that chose a location is not a push.
    menu_prev: u32,
    blocked: u32,
}

/// Lines kept while nobody takes them.
const LOG_CAP: usize = 512;
/// Seconds between status lines.
const STATUS_EVERY: f32 = 1.0;

/// What is ridden.
enum World {
    /// The built-in demo park (`Game::park`).
    Demo,
    Park(Box<ImportedPark>),
    District(Box<District>),
}

/// What the skater rides on (a free function so callers can borrow other fields of the game at the same
/// time).
fn terrain<'a>(world: &'a World, park: &'a Park) -> &'a dyn Terrain {
    match world {
        World::Demo => park,
        World::Park(p) => &p.mesh,
        World::District(d) => &**d,
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
            cam: CameraMan::new(&skater, &park, CamData::default(), Tuning::BUILT_IN.cam),
            park,
            skater,
            flick: FlickInput::new(),
            flick_out: FlickOut::default(),
            tuning: Tuning::BUILT_IN,
            stance: Stance::Goofy,
            score: 0,
            popup: None,
            prev_held: 0,
            foot_origin: 0.0,
            clock: 0.0,
            connected: true,
            events: Vec::new(),
            frame_cost: None,
            notice: None,
            rider: None,
            motion: None,
            graph: None,
            gesture_sets: Vec::new(),
            pose: Default::default(),
            world: World::Demo,
            district_wait: false,
            log: Vec::new(),
            devlog: DevLog::new(),
            status_timer: 0.0,
            timer: None,
            times: SectionTimes::default(),
            render_times: core::cell::Cell::new((0, 0, 0)),
            world_us: core::cell::Cell::new(0),
            draw_distance: crate::world::DRAW_DISTANCE,
            board_watch: None,
            settings_changed: false,
            ahead: 0.0,
            ticks: 0,
            prev_view: None,
            alpha: 1.0,
            locations: alloc::vec![Location::demo()],
            location: 0,
            menu: None,
            request: None,
            loading: None,
            menu_prev: 0,
            blocked: 0,
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
            let ops = p.find(crate::graph::opcode::ENTRY).map(|o| o.data);
            match crate::graph::drive::Driver::new(e.data, ops, &lib.db, self.gesture_sets()) {
                Ok(mut d) => {
                    d.load_us = self.timer.map_or(0, |f| f()).wrapping_sub(t0);
                    d.set_tuning(&self.tuning);
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

    /// Rides an imported park (`fg_import park`) instead of the demo park (or the park loaded before),
    /// starting at its spawn. The bytes stay borrowed until [`Game::unload_park`]: the Wii reads the park's
    /// textures in place. On an error the park ridden before stays.
    pub fn load_park(&mut self, pack: &'static [u8]) -> Result<(), collision::Error> {
        let p = ImportedPark::parse(pack)?;
        self.world = World::Park(Box::new(p));
        self.district_wait = false;
        self.start_over();
        Ok(())
    }

    /// Rides a district that streams in as the rider goes ([`District`]): the loading screen shows until
    /// the cells around its start are read, then the rider starts there. Replaces the park or district
    /// ridden before (which is freed: call between frames).
    pub fn load_district(&mut self, mut district: District) {
        district.set_draw_distance(self.draw_distance);
        self.world = World::District(Box::new(district));
        self.district_wait = true;
    }

    /// The district being ridden, if one is.
    pub fn district(&self) -> Option<&District> {
        match &self.world {
            World::District(d) => Some(d),
            _ => None,
        }
    }

    /// Is the built-in demo park being ridden (nothing imported)?
    pub fn on_demo_park(&self) -> bool {
        matches!(self.world, World::Demo)
    }

    /// Is a district still reading the cells around its start?
    pub fn district_loading(&self) -> bool {
        self.district_wait
    }

    /// Drops the imported park (back to the demo park, at its spawn). Afterwards nothing refers to the
    /// pack given to [`Game::load_park`] any more, so the platform may free it.
    pub fn unload_park(&mut self) {
        self.world = World::Demo;
        self.district_wait = false;
        self.start_over();
    }

    /// A fresh start on whatever park is loaded now (a new location): the rider at the spawn, the
    /// camera cut to it, the flick reader and both state graphs started over, nothing blended from the
    /// last frame, the old park's checkpoints gone.
    fn start_over(&mut self) {
        let ground: &dyn Terrain = terrain(&self.world, &self.park);
        self.skater.reset_to_spawn(ground);
        self.flick.reset();
        self.cam.request_reset();
        self.prev_view = None;
        self.popup = None;
        if let (Some(g), Some(lib)) = (&mut self.graph, &self.motion) {
            g.runner.reset(Some(&lib.db));
            g.input.reset();
        }
    }

    /// The places to ride (from the platform's location index). The demo park is added at the end; the
    /// demo park is what is loaded until the platform loads another and sets [`Game::location`].
    pub fn set_locations(&mut self, mut list: Vec<Location>) {
        list.retain(|l| !l.is_demo());
        list.push(Location::demo());
        self.location = list.len() - 1;
        self.locations = list;
    }

    /// Shows the start menu (at boot) with the location being ridden highlighted.
    pub fn open_start_menu(&mut self) {
        self.menu = Some(Menu::start(self.location, self.menu_prev));
    }

    /// What the game wants the platform to do, once.
    pub fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    /// A menu's decision. `held`: the buttons down as it closes (ignored by the riding until let go).
    fn choose(&mut self, c: Choice, held: u32) {
        self.menu = None;
        self.blocked = held;
        match c {
            Choice::Resume => {}
            Choice::Respawn => self.restart_at_spawn(),
            Choice::Quit => self.request = Some(Request::Quit),
            Choice::Ride(i) if i == self.location => {}
            Choice::Ride(i) => self.request = Some(Request::Load(i.min(self.locations.len() - 1))),
            Choice::DrawDistance(d) => self.set_draw_distance(d as f32),
        }
    }

    /// Follows the drawn board for [`BoardWatch::FRAMES`] frames after a landing: its yaw in the world
    /// against the way the rider travels. Logs once if that swings more than [`BoardWatch::SWING`] degrees
    /// (the board turning round under his feet), with what the graphs and the stance were doing.
    fn watch_board(&mut self) {
        let Some(mut w) = self.board_watch.take() else {
            return;
        };
        w.frames += 1;
        if w.frames > BoardWatch::FRAMES {
            return;
        }
        let travel = self.skater.vel.horizontal();
        let axes = self.board_axes_in_character();
        let Some((_, nose)) =
            axes.filter(|_| travel.length() >= 1.0 && self.skater.mode == Mode::Rolling)
        else {
            self.board_watch = Some(w);
            return;
        };
        let nose = self.imported_xform().rot.apply(nose);
        let rel = crate::math::wrap_angle(
            crate::math::atan2f(nose.x, nose.z) - crate::math::atan2f(travel.x, travel.z),
        );
        if let Some(last) = w.last {
            w.turned += crate::math::wrap_angle(rel - last);
            w.lo = w.lo.min(w.turned);
            w.hi = w.hi.max(w.turned);
        }
        w.last = Some(rel);
        let swing = (w.hi - w.lo).to_degrees();
        if swing > BoardWatch::SWING {
            let state = self
                .graph
                .as_ref()
                .map(|g| g.runner.motion_state())
                .unwrap_or_default();
            let line = format!(
                "board: turned {swing:.0} deg under the rider {} frames after {} (graph {state}, fakie {}, frame turned {})",
                w.frames, w.trick, self.skater.fakie, self.skater.frame_turned
            );
            if self.log.len() < LOG_CAP {
                self.log.push(line);
            }
            return;
        }
        self.board_watch = Some(w);
    }

    /// How far a district is drawn (m, the pause menu's setting): its fog and the cells read follow it.
    pub fn set_draw_distance(&mut self, d: f32) {
        self.draw_distance = d.clamp(
            crate::world::MIN_DRAW_DISTANCE,
            crate::world::MAX_DRAW_DISTANCE,
        );
        if let World::District(dist) = &mut self.world {
            dist.set_draw_distance(self.draw_distance);
        }
    }

    pub fn draw_distance(&self) -> f32 {
        self.draw_distance
    }

    /// True once after the player changed a setting (the platform saves them).
    pub fn take_settings_changed(&mut self) -> bool {
        core::mem::take(&mut self.settings_changed)
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
            g.set_tuning(&self.tuning);
        }
        self.skater.tuning = self.tuning;
        // The disc's camera shots and graph (`fg_import rules`), else the built-in ones.
        let mut data = CamData::default();
        if let Some(e) = pack.find(crate::camera::shot::ENTRY) {
            match crate::camera::shot::ShotSet::parse(e.data) {
                Ok(set) => data.shots = set,
                Err(e) => self.log.push(format!("camera: shots unreadable ({e:?})")),
            }
        }
        if data.shots.imported
            && let Some(e) = pack.find(crate::camera::select::ENTRY)
        {
            match crate::camera::select::CamGraph::load(e.data) {
                Ok(g) => {
                    for n in &g.unsupported {
                        self.log.push(format!("camera graph: unsupported {n}"));
                    }
                    data.graph = Some(g);
                }
                Err(e) => self.log.push(format!("camera: graph unreadable ({e:?})")),
            }
        }
        self.cam.set_data(data, self.tuning.cam);
        Ok((values, shapes))
    }

    /// A shape of a gesture file from the rules pack (`skaterls`, `SlideFs180` ...), if loaded.
    pub fn gesture_shape(&self, set: &str, name: &str) -> Option<&gesture::Shape> {
        self.gesture_sets
            .iter()
            .find(|s| s.0 == set)
            .and_then(|s| s.1.iter().find(|x| x.name == name))
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
        match &self.world {
            World::Park(p) => Some(p),
            _ => None,
        }
    }

    pub fn imported_park_mut(&mut self) -> Option<&mut ImportedPark> {
        match &mut self.world {
            World::Park(p) => Some(p),
            _ => None,
        }
    }

    /// What the skater rides on: the imported park if there is one, else the demo park.
    /// Puts the rider back at the park's spawn, a fresh start (scripted runs between tours).
    pub fn restart_at_spawn(&mut self) {
        let ground: &dyn Terrain = terrain(&self.world, &self.park);
        self.skater.reset_to_spawn(ground);
        self.flick.reset();
        self.cam.request_reset();
    }

    /// Puts the rider on the board at `pos` (on the ground there), heading `heading` (yaw), rolling at
    /// `speed`: for scripted runs (`autoplay`).
    /// Puts the rider where a scripted run asks: on the ground ([`Game::place_rider`]), or in the air
    /// moving along its heading and up at its lift.
    pub fn place(&mut self, p: &crate::autoplay::Placement) {
        if p.lift <= 0.0 {
            self.place_rider(p.pos, p.heading, p.speed);
            return;
        }
        self.place_rider(p.pos, p.heading, 0.0);
        let s = &mut self.skater;
        s.pos = p.pos;
        s.vel = crate::math::yaw_dir(p.heading) * p.speed + crate::math::Vec3::UP * p.lift;
        s.mode = Mode::Air { time: 0.0 };
    }

    pub fn place_rider(&mut self, pos: crate::math::Vec3, heading: f32, speed: f32) {
        let ground: &dyn Terrain = terrain(&self.world, &self.park);
        self.skater.put_at(ground, pos, heading, speed);
        self.flick.reset();
        self.cam.request_reset();
    }

    /// The screen's width over its height (the platform's TV setting): the camera's field of view
    /// follows it.
    pub fn set_aspect(&mut self, aspect: f32) {
        if aspect.is_finite() && aspect > 0.5 {
            self.cam.aspect = aspect;
        }
    }

    /// The camera (shots, rig, what chose the shot).
    pub fn camera(&self) -> &CameraMan {
        &self.cam
    }

    pub fn terrain(&self) -> &dyn Terrain {
        terrain(&self.world, &self.park)
    }

    /// One displayed frame of `dt` seconds (1/60 on NTSC and PAL60, 1/50 on PAL50): runs as many fixed
    /// [`TICK`]s as it takes to reach the frame's time (one at 60 Hz; one or two at 50 Hz), then poses the
    /// rider once. A PAL50 frame is drawn between the last two ticks.
    pub fn update(&mut self, raw: &PadRaw, dt: f32) {
        self.connected = raw.flags & flag::CONNECTED != 0;
        self.events.clear();
        let pressed = raw.buttons & !self.menu_prev;
        self.menu_prev = raw.buttons;
        // A district reads and drops cells around the rider (or its start, before the rider is placed),
        // menus open or not. Where the cells under the rider aren't in yet, the riding waits.
        let mut held = false;
        if let World::District(d) = &mut self.world {
            let at = if self.district_wait {
                d.spawn().0
            } else {
                self.skater.pos
            };
            d.stream(at);
            if self.log.len() < LOG_CAP {
                self.log.append(&mut d.log);
            } else {
                d.log.clear();
            }
            held = !d.ready(at);
            if self.district_wait && !held {
                self.district_wait = false;
                self.start_over();
            }
        }
        if self.district_wait {
            return;
        }
        // A menu freezes the riding and takes the pad (the d-pad moves its cursor, not the rider's
        // gestures).
        if let Some(menu) = &mut self.menu {
            match menu.update(raw, dt, self.locations.len(), self.location) {
                Some(Choice::DrawDistance(d)) => {
                    self.set_draw_distance(d as f32);
                    self.settings_changed = true;
                }
                Some(c) => self.choose(c, raw.buttons),
                None => {}
            }
            return;
        }
        if self.loading.is_some() {
            return;
        }
        if pressed & btn::START != 0 {
            self.menu = Some(Menu::pause(raw.buttons, self.draw_distance as u16));
            return;
        }
        self.blocked &= raw.buttons;
        let unblocked = PadRaw {
            buttons: raw.buttons & !self.blocked,
            ..*raw
        };
        let raw = &unblocked;
        if held {
            return;
        }
        self.ahead -= dt;
        // A little slack so float error never runs (or skips) a tick at 60 Hz.
        while self.ahead < -1e-4 {
            self.prev_view = Some(self.view());
            let turned = self.skater.frame_turned;
            self.tick(raw);
            self.ahead += TICK;
            if self.skater.frame_turned != turned
                && let Some(p) = &mut self.prev_view
            {
                // The board's frame turned end for end (fakie to switch): the frame drawn between the
                // two ticks turns with it, or PAL50's blend would swing the rider through half a turn.
                p.base = p.base.then(&Xform::new(Mat3::rot_y(PI), v3(0.0, 0.0, 0.0)));
            }
        }
        self.alpha = (1.0 - self.ahead / TICK).clamp(0.0, 1.0);
        let now = |c: Option<fn() -> u32>| c.map_or(0, |f| f());
        let t1 = now(self.timer);
        if self.rider.is_some() {
            let rot = self.imported_xform().rot;
            let s = &self.skater;
            let ground: &dyn Terrain = terrain(&self.world, &self.park);
            let land_in = s.time_to_land(ground);
            let mut posed = false;
            if let (Some(r), Some(g), Some(lib)) = (&mut self.rider, &mut self.graph, &self.motion)
            {
                g.runner.pose(&mut self.pose);
                posed = r.update_graph(lib, &self.pose, rot, self.skater.frame_turned);
            }
            if !posed && let Some(r) = &mut self.rider {
                r.update(&self.skater, self.clock, land_in, dt, rot);
            }
        }
        let t2 = now(self.timer);
        self.times.skin_us = t2.wrapping_sub(t1);
        let (render_us, park_draws, park_verts) = self.render_times.get();
        self.times.render_us = render_us;
        self.times.world_us = self.world_us.get();
        self.times.park_draws = park_draws;
        self.times.park_verts = park_verts;

        self.watch_board();
        for e in &self.events {
            match e {
                Event::Landed { name, points } => {
                    self.board_watch = Some(BoardWatch {
                        trick: name.clone(),
                        frames: 0,
                        last: None,
                        turned: 0.0,
                        lo: 0.0,
                        hi: 0.0,
                    });
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
        let ground: &dyn Terrain = terrain(&self.world, &self.park);
        // On foot the stick is read against the camera (where it looks, seen from above).
        {
            let c = self.cam.camera();
            let look = (c.target - c.eye).horizontal();
            if look.length() > 1e-3 {
                self.skater.cam_yaw = crate::math::atan2f(look.x, look.z);
            }
        }
        if let (Some(g), Some(lib)) = (&mut self.graph, &self.motion) {
            let s = &self.skater;
            let walking = s.mode == Mode::Walk;
            let ob_world = crate::skater::biped::world_stick(pad.stick, s.cam_yaw);
            let (toggle_board, toggle_board_new) = s.board_toggle_intent(&pad);
            let (ob_mag, ob_steer) =
                crate::skater::biped::read_stick(&s.tuning.biped, s.biped.heading, ob_world);
            // The known landing's time (the trajectory selector), else straight down.
            let land_in = matches!(s.mode, Mode::Air { .. }).then(|| s.time_to_land(ground));
            let mut world = g.world(s, land_in, g.input.idle());
            if let Some(f) = self.rider.as_ref().and_then(|r| r.feet()) {
                world.feet = f;
            }
            if matches!(s.mode, Mode::Bailed { .. }) {
                world.wipe_stick = Some(pad.stick);
            }
            let gates = crate::graph::input::Gates {
                late_flips_blocked: s.trick.is_some_and(|t| t.progress() < 1.0),
                finger_flips_blocked: s.grab.is_none(),
                reverts_blocked: false,
                grinding: s.mode == Mode::Grind,
                grabbing: pad.trig_l > 0.0 || pad.trig_r > 0.0,
                pushing: pad.held & (crate::input::btn::A | crate::input::btn::B) != 0,
                off_board: walking,
                ob_mag,
                ob_steer,
                ob_world,
                toggle_board,
                toggle_board_new,
            };
            g.tick(&lib.db, &pad, world, gates, dt, self.timer);
            let mut control = g.control(&lib.db, &self.tuning);
            if control.push_target.is_some() {
                // Skate 3 chases the posed push's trajectory speed while the foot is on the ground
                // (SK8 contact_events.rs `push_speed`, ground_runtime/input.rs `push_target`).
                g.runner.pose(&mut self.pose);
                if let Some(v) = self
                    .pose
                    .trajectory_speed(lib, dt, self.skater.frame_turned)
                {
                    control.push_target = Some(v);
                    control.push_posed = true;
                }
            }
            if let Some(r) = &self.rider {
                control.pose_com = r.com_above_board();
                control.pose_lift = (r.board_rest_height() - crate::skater::DECK_HEIGHT).max(0.0);
            }
            self.skater.graph = Some(control);
            self.times.graph_us = g.cost_us;
            if self.log.len() < LOG_CAP {
                if let Some(state) = g.state_change() {
                    self.log.push(format!("graph: {state}"));
                }
                for n in g.new_unsupported() {
                    self.log.push(format!("graph: unsupported {n}"));
                }
                for (tree, param) in g.new_unset(&lib.db) {
                    self.log
                        .push(format!("graph: {tree} plays with unset parameter {param}"));
                }
            }
        }
        let mut events = Vec::new();
        let air = self.skater.is_airborne();
        let t_step = now(self.timer);
        self.skater.step(ground, &pad, &flick, dt, &mut events);
        let off_clips = self.graph.is_some()
            && (self.skater.mode == Mode::Walk
                || self
                    .skater
                    .graph
                    .is_some_and(|c| c.mounting || c.dismounting));
        let step = dt / FOOT_ORIGIN_BLEND;
        self.foot_origin =
            (self.foot_origin + if off_clips { step } else { -step }).clamp(0.0, 1.0);
        if air {
            let us = now(self.timer).wrapping_sub(t_step);
            self.times.air_us = us;
            self.times.air_peak_us = self.times.air_peak_us.max(us);
        }
        if events.contains(&Event::Respawned) {
            self.flick.reset();
            self.cam.request_reset();
        }
        let t_cam = now(self.timer);
        self.cam.update(&self.skater, ground, dt);
        let us = now(self.timer).wrapping_sub(t_cam);
        self.times.cam_us = us;
        self.times.cam_peak_us = self.times.cam_peak_us.max(us);
        self.times.physics_us = now(self.timer).wrapping_sub(t0);
        // Development log: what happened this tick, and a status line now and then.
        let cam_lines = self.cam.take_log();
        if self.log.len() < LOG_CAP {
            self.log.extend(cam_lines);
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

    /// The posed board's up and nose in the rider's space (the imported rider's last pose).
    pub fn board_axes_in_character(&self) -> Option<(crate::math::Vec3, crate::math::Vec3)> {
        self.rider.as_ref().and_then(|r| r.board_axes())
    }

    /// The posed board's centre in the rider's space (the imported rider's last pose).
    pub fn board_in_character(&self) -> Option<crate::math::Vec3> {
        self.rider.as_ref().and_then(|r| r.board_in_character())
    }

    /// Where the drawn board's centre is (the posed `SKATEBOARD_ROOT` placed in the world), with the
    /// imported rider; else the physics board's deck.
    pub fn drawn_board(&self) -> crate::math::Vec3 {
        let x = self.imported_xform();
        self.rider
            .as_ref()
            .and_then(|r| r.board_in_character())
            .map_or(x.pos, |b| x.point(b))
    }

    /// The camera to draw with (blended like the skater).
    pub fn drawn_camera(&self) -> crate::gfx::Camera {
        let cur = self.cam.camera();
        match &self.prev_view {
            Some(p) if self.alpha < 1.0 && (cur.eye - p.cam.eye).length() < TELEPORT => {
                crate::gfx::Camera {
                    eye: p.cam.eye.lerp(cur.eye, self.alpha),
                    target: p.cam.target.lerp(cur.target, self.alpha),
                    up: p.cam.up.lerp(cur.up, self.alpha).normalize_or(cur.up),
                    fov_y_deg: p.cam.fov_y_deg + (cur.fov_y_deg - p.cam.fov_y_deg) * self.alpha,
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
        let streaming = self.district().filter(|_| self.district_wait).map(|d| {
            let (done, of) = d.progress(d.spawn().0);
            format!("{done} OF {of}")
        });
        if let Some(text) = self.loading.as_ref().or(streaming.as_ref()) {
            gfx.begin_frame(&cam, rgb(0, 0, 0));
            let mut m = MeshBuilder::new();
            m.rect_2d(0.0, 0.0, SCREEN_W, SCREEN_H, rgb(0, 0, 0));
            hud::text_centered(
                &mut m,
                SCREEN_W / 2.0,
                200.0,
                5.0,
                "LOADING",
                rgb(255, 230, 90),
            );
            hud::text_centered(&mut m, SCREEN_W / 2.0, 250.0, 3.0, text, rgb(255, 255, 255));
            gfx.draw_2d(&m.verts);
            gfx.end_frame();
            return;
        }
        gfx.begin_frame(&cam, SKY);
        // A district is drawn only so far around the rider: it fades into the sky before that.
        let mut reach = crate::world::DRAW_DISTANCE;
        if let World::District(_) = self.world {
            reach = self.draw_distance;
            let (start, end) = crate::world::fog_for(reach);
            gfx.fog(Some(crate::gfx::Fog {
                start,
                end,
                rgba: SKY,
            }));
        }
        // Only what the camera can see is drawn.
        let sight = crate::world::Sight::of(&cam, self.cam.aspect, reach);
        let tw = now();
        let (draws, verts) = match &self.world {
            World::Park(p) => p.draw(gfx, &sight),
            World::District(d) => d.draw(gfx, &sight),
            World::Demo => {
                gfx.draw_tris(&self.park_mesh, &Xform::IDENTITY.to_mat34());
                (1, self.park_mesh.len() as u32)
            }
        };
        self.world_us.set(now().wrapping_sub(tw));
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
        // A body flip turns board and rider about the centre of mass.
        if let Some(f) = &s.body_flip {
            let pivot = s.pos + up * crate::skater::flip::PIVOT;
            let (up, nose) = (f.apply(up), f.apply(nose));
            let pos = pivot + f.apply(s.pos - pivot);
            return Xform::new(Mat3::from_cols(up.cross(nose), up, nose), pos);
        }
        Xform::new(Mat3::from_cols(up.cross(nose), up, nose), s.pos)
    }

    /// Where the imported character is drawn: board space, tipped over on a bail. The clips' board
    /// (`SKATEBOARD_ROOT`, the character's origin) is the deck's centre, which Skate 3 puts on the
    /// physical deck: riding, high enough over the board's contact point for its lowest wheel to rest
    /// on the ground ([`Rider::board_rest_height`], about [`crate::skater::DECK_HEIGHT`]), elsewhere
    /// the deck's height.
    pub fn imported_xform(&self) -> Xform {
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
        let base = self.drawn_base();
        // Riding, the board's lowest wheel on the ground (a manual's pitch lifts the deck).
        let height = match (&self.rider, self.skater.mode) {
            (Some(r), Mode::Rolling) => r.board_rest_height() + self.wheel_lift(&base),
            (_, Mode::Air { .. }) => crate::skater::DECK_HEIGHT + self.skater.air_lift,
            // On foot the off-board clips' origin is on the ground under the feet.
            _ => crate::skater::DECK_HEIGHT,
        };
        // On foot (and getting off or on) the off-board clips' origin is on the ground under the feet.
        let height = height * (1.0 - self.foot_origin);
        base.then(&Xform::new(tip, v3(0.0, 0.0, 0.0)))
            .then(&Xform::new(IMPORTED_TO_BOARD, v3(0.0, height, 0.0)))
    }

    /// How far (m, along the board's up) the drawn board must rise so none of its wheels is in the
    /// ground under it. The physics puts the board on its two trucks' contacts along their middle, and
    /// the drawn board's roll follows the smoothed ground normal: on a transition ridden across, a
    /// side's wheels would dip into it. The drawing takes that out, at most `WHEEL_LIFT_MAX`.
    fn wheel_lift(&self, base: &Xform) -> f32 {
        let ground: &dyn Terrain = terrain(&self.world, &self.park);
        let up = base.rot.cols[1];
        let mut lift: f32 = 0.0;
        // Under each wheel: `wheel_x_distance` (SK8 construction/authored_transforms.rs) each side of
        // each truck.
        for (x, z) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            let p = base.point(v3(x * 0.095, 0.0, z * crate::skater::AXLE));
            let g = ground.ground(p.x, p.z, p.y + WHEEL_LIFT_MAX);
            let along = g.normal.dot(up);
            if g.height > f32::MIN / 2.0 && along > 0.5 {
                lift = lift.max((g.height - p.y) * g.normal.y / along);
            }
        }
        lift.clamp(0.0, WHEEL_LIFT_MAX)
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
        // Caught dark (or riding dark): grip down.
        if s.dark || s.trick.is_some_and(|t| t.dark && t.progress() >= 1.0) {
            board_rot = board_rot.mul(&Mat3::rot_z(PI));
        }
        let pivot = 0.09;
        let board = base
            .then(&Xform::new(board_rot, v3(0.0, pivot + lift, 0.0)))
            .then(&Xform::new(Mat3::IDENTITY, v3(0.0, -pivot, 0.0)));
        // (On foot the placeholder rider has no board to carry.)
        if s.mode != Mode::Walk {
            gfx.draw_tris(&self.models.board, &board.to_mat34());
        }

        // Rider: tucks with the board, falls over on a bail.
        let (rider_pos, rider_rot) = match s.mode {
            Mode::Bailed { time } | Mode::Runout { time } => (
                v3(0.0, 0.0, 0.0),
                Mat3::rot_z(-(time * 5.0).min(1.0) * PI * 0.48),
            ),
            _ => (v3(0.0, lift * 0.7, 0.0), Mat3::IDENTITY),
        };
        // The placeholder stands goofy (facing +X); mirrored (regular, or goofy riding switch) it faces
        // the other side of the board, as the imported rider's mirrored clips do.
        let stance = if s.mirrored() {
            Mat3::rot_y(PI)
        } else {
            Mat3::IDENTITY
        };
        let rider = base.then(&Xform::new(stance.mul(&rider_rot), rider_pos));
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
        // The fade to black before an automatic reset after a bail.
        let dark = self.skater.recovery.darkness(&self.tuning);
        if dark > 0.0 {
            m.rect_2d(0.0, 0.0, SCREEN_W, SCREEN_H, (dark * 255.0) as u32 & 0xff);
        }
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
            // Pop in with a slight scale-up, no wider than most of the screen.
            let grow = 1.0 + 0.15 * (1.0 - (p.time * 8.0).min(1.0));
            let fit = (SCREEN_W * 0.8) / (4.0 * p.title.chars().count().max(1) as f32);
            let px = POPUP_PX.min(fit) * grow;
            hud::text_centered(&mut m, SCREEN_W / 2.0, 96.0, px, &p.title, p.rgba);
            if !p.detail.is_empty() {
                hud::text_centered(
                    &mut m,
                    SCREEN_W / 2.0,
                    96.0 + px * 7.0,
                    2.5,
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
        if let Some(menu) = &self.menu {
            let names: Vec<&str> = self.locations.iter().map(|l| l.name.as_str()).collect();
            menu.draw(&mut m, &names, self.location);
            gfx.draw_2d(&m.verts);
            return;
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
    fn a_frame_that_ran_long_keeps_the_game_at_full_speed() {
        // The Wii passes the refreshes the last frame was on screen for: a frame shown for two refreshes
        // runs two ticks, so the game keeps its speed at 30 fps (it doesn't go into slow motion).
        let mut g = Game::new();
        let mut slow = Game::new();
        for i in 0..60 {
            g.update(&held(crate::input::btn::A), 1.0 / 60.0);
            if i % 2 == 1 {
                let before = slow.ticks;
                slow.update(&held(crate::input::btn::A), 2.0 / 60.0);
                assert_eq!(slow.ticks - before, 2);
            }
        }
        assert_eq!((g.ticks, slow.ticks), (60, 60));
        assert_eq!(g.skater.pos, slow.skater.pos);
        assert_eq!(g.skater.speed, slow.skater.speed);
        assert_eq!(slow.blend(), 1.0);
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

    /// Keeps the 2D vertices drawn.
    #[derive(Default)]
    struct Overlay(Vec<Vertex>);
    impl Gfx for Overlay {
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
        fn draw_2d(&mut self, v: &[Vertex]) {
            self.0.extend_from_slice(v);
        }
        fn end_frame(&mut self) {}
    }

    #[test]
    fn the_hud_leaves_the_lower_screen_clear_and_a_long_trick_name_fits() {
        let mut g = Game::new();
        g.update(&held(0), 1.0 / 60.0);
        let mut o = Overlay::default();
        g.render(&mut o);
        // No controls list: nothing drawn in the bottom third, where the skater stands.
        assert!(o.0.iter().all(|v| v.pos[1] < SCREEN_H * 0.66), "text in the lower screen");
        g.popup = Some(Popup {
            title: "FAKIE BS 360 NOLLIE 360 INWARD HEELFLIP".into(),
            detail: "+1234".into(),
            rgba: 0x010203ff,
            time: 1.0,
        });
        let mut o = Overlay::default();
        g.render(&mut o);
        o.0.retain(|v| v.rgba == 0x010203ff);
        assert!(!o.0.is_empty());
        let (lo, hi) = o.0.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| {
            (lo.min(v.pos[0]), hi.max(v.pos[0]))
        });
        assert!(lo >= SCREEN_W * 0.09 && hi <= SCREEN_W * 0.91, "{lo}..{hi}");
        let tall = o.0.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        assert!(tall < SCREEN_H * 0.4, "{tall}");
    }

    #[test]
    fn a_reset_fades_the_screen_and_the_camera_draws_between_ticks() {
        let mut g = Game::new();
        g.update(&held(0), 1.0 / 60.0);
        let black = |g: &Game| {
            let mut o = Overlay::default();
            g.render(&mut o);
            o.0.iter()
                .filter(|v| v.rgba >> 8 == 0 && v.pos[0] == 0.0 && v.pos[1] == 0.0)
                .map(|v| v.rgba & 0xff)
                .max()
        };
        assert_eq!(black(&g), None);
        g.skater.recovery.fade = Some(g.tuning.reset_fade * 0.5);
        let a = black(&g).expect("a fade quad");
        assert!((100..=155).contains(&a), "{a}");
        // PAL50: the camera is drawn between the last two ticks.
        let mut g = Game::new();
        for _ in 0..30 {
            g.update(&held(crate::input::btn::A), 1.0 / 50.0);
        }
        let (prev, cur) = (g.prev_view.unwrap().cam.eye, g.cam.camera().eye);
        let k = g.blend();
        assert!((g.drawn_camera().eye - prev.lerp(cur, k)).length() < 1e-4);
    }

    /// Keeps the fog asked for.
    #[derive(Default)]
    struct FogSeen(Vec<Option<crate::gfx::Fog>>);
    impl Gfx for FogSeen {
        fn begin_frame(&mut self, _cam: &crate::gfx::Camera, _clear: u32) {}
        fn fog(&mut self, f: Option<crate::gfx::Fog>) {
            self.0.push(f);
        }
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

    #[test]
    fn a_district_fades_into_the_sky_before_the_draw_distance() {
        use crate::city::{CellInfo, Index};
        use crate::reader::MemReader;
        use alloc::vec;
        let mut g = Game::new();
        let mut seen = FogSeen::default();
        g.render(&mut seen);
        assert!(seen.0.is_empty(), "no fog on the demo park");
        let floor = crate::collision::encode(&[
            (
                [[0.0, 0.0, 0.0], [0.0, 0.0, 100.0], [100.0, 0.0, 100.0]],
                1,
                0,
            ),
            (
                [[0.0, 0.0, 0.0], [100.0, 0.0, 100.0], [100.0, 0.0, 0.0]],
                1,
                0,
            ),
        ]);
        let spawn: Vec<u8> = [50.0f32, 0.0, 50.0, 0.0]
            .iter()
            .flat_map(|f| f.to_be_bytes())
            .collect();
        let p = crate::pack::encode(&[
            (
                "park/collision",
                crate::pack::Kind::Collision,
                &floor,
                0,
                0,
                0,
            ),
            ("park/spawn", crate::pack::Kind::Data, &spawn, 0, 0, 0),
        ]);
        let mut r = MemReader::new();
        r.add("d/c_50_50.fgpak", p.clone());
        let index = Index {
            cell: 100.0,
            floor: 0.0,
            starts: vec![(v3(50.0, 0.0, 50.0), 0.0)],
            cells: vec![CellInfo {
                x: 50,
                z: 50,
                bytes: p.len() as u32,
                tris: 2,
                vertices: 0,
                texture_bytes: 0,
            }],
        };
        let r = alloc::rc::Rc::new(core::cell::RefCell::new(r));
        let d = crate::district::District::new(&index.encode(), "d", Box::new(r.clone()), 1 << 20)
            .unwrap();
        g.load_district(d);
        while g.district_loading() {
            r.borrow_mut().service();
            g.update(&held(0), 1.0 / 60.0);
        }
        g.render(&mut seen);
        let f = seen.0.last().copied().flatten().expect("fog in a district");
        assert!(f.start < f.end && f.end < crate::world::DRAW_DISTANCE);
        assert_eq!(f.rgba, SKY);

        // The pause menu's draw distance: the fog comes in with it, the district reads that far, and the
        // change is flagged for saving once.
        g.update(&held(btn::START), 1.0 / 60.0);
        g.update(&held(0), 1.0 / 60.0);
        for b in [
            btn::DPAD_DOWN,
            btn::DPAD_DOWN,
            btn::DPAD_LEFT,
            btn::DPAD_LEFT,
        ] {
            g.update(&held(b), 1.0 / 60.0);
            g.update(&held(0), 1.0 / 60.0);
        }
        assert_eq!(g.draw_distance(), 70.0);
        assert_eq!(g.district().unwrap().draw_distance(), 70.0);
        assert!(g.take_settings_changed() && !g.take_settings_changed());
        g.render(&mut seen);
        let near = seen.0.last().copied().flatten().unwrap();
        assert!(near.end < f.end && near.end < 70.0, "{near:?}");
        // Out of range: held to the range.
        g.set_draw_distance(500.0);
        assert_eq!(g.draw_distance(), crate::world::MAX_DRAW_DISTANCE);
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
