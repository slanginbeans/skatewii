//! Wii side of flatground: a `no_std` staticlib linked by devkitPPC into `flatground.dol`.
//!
//! The C `main` (`wii/platform/main.c`) owns the loop and calls [`fg_rust_init`] once, then
//! [`fg_rust_frame`] every vsync. Everything platform-specific Rust needs is in `ffi.rs`, mirroring
//! `wii/platform/fg_platform.h`.

#![no_std]

extern crate alloc;

mod ffi;
mod rt;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use core::ffi::CStr;
use core::ptr;

use game_core::autoplay::Autoplay;
use game_core::character;
use game_core::game::{FrameCost, Game};
use game_core::gfx::{Camera, Gfx, StaticVerts, TexVertex, Texture, Vertex};
use game_core::input::PadRaw;
use game_core::math::Mat34;
use game_core::menu::{self, Location, LocationTour, Request};
use game_core::reader::{Poll, Reader, Ticket};

/// Max vertices per `GX_Begin` (it takes a u16), rounded down to whole triangles.
const MAX_BATCH: usize = 65535 / 3 * 3;

struct WiiGfx;

impl Gfx for WiiGfx {
    fn begin_frame(&mut self, cam: &Camera, clear_rgba: u32) {
        let eye = [cam.eye.x, cam.eye.y, cam.eye.z];
        let target = [cam.target.x, cam.target.y, cam.target.z];
        let up = [cam.up.x, cam.up.y, cam.up.z];
        unsafe {
            ffi::fg_begin_frame(
                eye.as_ptr(),
                target.as_ptr(),
                up.as_ptr(),
                cam.fov_y_deg,
                cam.near,
                cam.far,
                clear_rgba,
            )
        }
    }
    fn draw_tris(&mut self, verts: &[Vertex], model: &Mat34) {
        for chunk in verts.chunks(MAX_BATCH) {
            unsafe { ffi::fg_draw_tris(chunk.as_ptr(), chunk.len() as u32, model) }
        }
    }
    fn draw_indexed(
        &mut self,
        verts: &[TexVertex],
        indices: &[u16],
        texture: Option<&Texture>,
        model: &Mat34,
    ) {
        draw(ffi::fg_draw_indexed, verts, indices, texture, model);
    }
    fn draw_indexed_static(
        &mut self,
        verts: &StaticVerts,
        indices: &[u16],
        texture: Option<&Texture>,
        model: &Mat34,
    ) {
        // Flushed once when the park loaded (`flush_park`).
        draw(ffi::fg_draw_indexed_static, verts, indices, texture, model);
    }
    fn draw_2d(&mut self, verts: &[Vertex]) {
        for chunk in verts.chunks(MAX_BATCH) {
            unsafe { ffi::fg_draw_2d(chunk.as_ptr(), chunk.len() as u32) }
        }
    }
    fn end_frame(&mut self) {
        unsafe { ffi::fg_end_frame() }
    }
}

type DrawFn = unsafe extern "C" fn(
    *const TexVertex,
    u32,
    *const u16,
    u32,
    *const ffi::FgTexture,
    *const Mat34,
);

/// An indexed mesh through `f` (copying or in place), in batches one `GX_Begin` takes.
fn draw(f: DrawFn, verts: &[TexVertex], indices: &[u16], texture: Option<&Texture>, model: &Mat34) {
    // One GX_Begin takes at most 65535 indices; 16-bit indices reach 65536 vertices.
    if verts.len() > 0x10000 {
        return;
    }
    let tex = texture.map(|t| ffi::FgTexture {
        data: t.data.as_ptr(),
        width: t.width,
        height: t.height,
        format: t.format,
        alpha_test: t.alpha_test as u8,
        _pad: 0,
    });
    let tex_ptr = tex.as_ref().map_or(ptr::null(), |t| t as *const _);
    for chunk in indices.chunks(MAX_BATCH) {
        unsafe {
            f(
                verts.as_ptr(),
                verts.len() as u32,
                chunk.as_ptr(),
                chunk.len() as u32,
                tex_ptr,
                model,
            )
        }
    }
}

/// Writes a freshly loaded park's vertex arrays out of the CPU cache once: the GPU reads them in place.
fn flush_park(game: &Game) {
    if let Some(p) = game.imported_park() {
        for m in &p.models {
            let (data, len) = m.verts.bytes();
            unsafe { ffi::fg_flush_range(data, len as u32) };
        }
    }
}

/// Where the build puts the pack `fg_import` makes from the player's own disc (the app folder's `data/`),
/// then the older location (see docs/importing.md).
const SKATER_PACKS: &[&CStr] = &[
    c"sd:/apps/flatground/data/skater.fgpak",
    c"usb:/apps/flatground/data/skater.fgpak",
    c"sd:/flatground/skater.fgpak",
    c"usb:/flatground/skater.fgpak",
];

/// The app folder's data directories: the location index (`parks.txt`, written by `fg_import build`) and the
/// park packs it lists are read from the first that has an index, else from the first with an old single
/// `park.fgpak`.
const DATA_DIRS: &[&str] = &["sd:/apps/flatground/data/", "usb:/apps/flatground/data/"];
/// The location index in a data directory.
const INDEX_FILE: &str = "parks.txt";
/// What builds before the location index made: one park, `data/park.fgpak`.
const OLD_PARK: &str = "park.fgpak";
/// The last location chosen, in the app folder (next to `data/`).
const SETTINGS_FILE: &str = "settings.txt";

/// Skate 3's riding tuning and flick shapes, converted from the player's disc (`fg_import rules`).
const RULES_PACKS: &[&CStr] = &[
    c"sd:/apps/flatground/data/rules.fgpak",
    c"usb:/apps/flatground/data/rules.fgpak",
];

/// Skate 3's motion library: clip timing, attribute tracks and events, animation trees, poses
/// (`fg_import motion`).
const MOTION_PACKS: &[&CStr] = &[
    c"sd:/apps/flatground/data/motion.fgpak",
    c"usb:/apps/flatground/data/motion.fgpak",
];

/// A file in the app folder that turns on hands-off play (`game_core::autoplay::Autoplay`): scripted test
/// runs in Dolphin (`tools/dolphin_test.ps1`) and on a Wii. Its contents are ignored.
const AUTOPLAY_FILES: &[&CStr] = &[
    c"sd:/apps/flatground/autoplay.txt",
    c"usb:/apps/flatground/autoplay.txt",
];

/// A whole file, or `None`. Kept until given to [`unload`] (the packs the game keeps for the whole session
/// never are): the GPU reads textures straight out of packs.
fn load(path: &CStr) -> Option<&'static [u8]> {
    let mut size = 0u32;
    let data = unsafe { ffi::fg_load_file(path.as_ptr(), &mut size) };
    (!data.is_null()).then(|| unsafe { core::slice::from_raw_parts(data, size as usize) })
}

/// [`load`] by a path built at runtime.
fn load_path(path: &str) -> Option<&'static [u8]> {
    alloc::ffi::CString::new(path).ok().and_then(|p| load(&p))
}

/// Gives back a file from [`load`].
///
/// # Safety
/// Nothing may refer to `data` any more, and no draw still queued for the GPU may read it.
unsafe fn unload(data: &'static [u8]) {
    unsafe { ffi::fg_free_file(data.as_ptr() as *mut u8) }
}

/// Background whole-file reads (`wii/platform/fg_read.c`), for streaming a district's cells.
#[allow(dead_code)] // The district streamer is its user.
struct WiiReader;

impl Reader for WiiReader {
    fn start(&mut self, path: &str) -> Option<Ticket> {
        let p = alloc::ffi::CString::new(path).ok()?;
        let t = unsafe { ffi::fg_read_start(p.as_ptr()) };
        (t >= 0).then_some(Ticket(t))
    }
    fn poll(&mut self, ticket: Ticket) -> Poll {
        let mut data = ptr::null_mut();
        let mut size = 0u32;
        match unsafe { ffi::fg_read_poll(ticket.0, &mut data, &mut size) } {
            0 => Poll::Waiting,
            1 if !data.is_null() => {
                Poll::Done(unsafe { core::slice::from_raw_parts(data, size as usize) })
            }
            _ => Poll::Failed,
        }
    }
    fn cancel(&mut self, ticket: Ticket) {
        unsafe { ffi::fg_read_cancel(ticket.0) }
    }
    unsafe fn free(&mut self, data: &'static [u8]) {
        unsafe { unload(data) }
    }
}

/// Small text file, or `None`.
fn load_text(path: &str) -> Option<String> {
    let data = load_path(path)?;
    let text = String::from_utf8_lossy(data).into_owned();
    unsafe { unload(data) };
    Some(text)
}

fn save(path: &str, data: &[u8]) -> bool {
    alloc::ffi::CString::new(path).is_ok_and(|p| unsafe {
        ffi::fg_save_file(p.as_ptr(), data.as_ptr(), data.len() as u32) == 0
    })
}

fn stats() -> ffi::FgFrameStats {
    let mut st = ffi::FgFrameStats::default();
    unsafe { ffi::fg_frame_stats(&mut st) };
    st
}

/// Heap and arena memory, for the log.
fn memory_text(st: &ffi::FgFrameStats) -> String {
    format!(
        "heap {} KB, MEM1 {} KB free, MEM2 {} KB free",
        st.heap_used / 1024,
        st.mem1_free / 1024,
        st.mem2_free / 1024
    )
}

/// Where the locations come from, and the park pack in use.
struct Places {
    /// The data directory the index (or old park pack) was found in, e.g. `sd:/apps/flatground/data/`.
    data_dir: Option<&'static str>,
    /// The pack of the park being ridden (freed when another location is loaded).
    pack: Option<&'static [u8]>,
    /// Autoplay visiting every location in turn (`switch <seconds>` in `autoplay.txt`).
    tour: Option<LocationTour>,
}

/// The locations: the index in the first data directory that has one, else an old single `park.fgpak`,
/// else none (the demo park only). Logs which.
fn find_locations() -> (Option<&'static str>, menu::Index) {
    for dir in DATA_DIRS {
        if let Some(text) = load_text(&format!("{dir}{INDEX_FILE}")) {
            let index = menu::parse_index(&text);
            log(
                false,
                &format!(
                    "locations: {dir}{INDEX_FILE}, {} parks ({}), default {}",
                    index.locations.len(),
                    index
                        .locations
                        .iter()
                        .map(|l| l.id.as_str())
                        .collect::<alloc::vec::Vec<_>>()
                        .join(" "),
                    index
                        .default
                        .map_or("none", |d| index.locations[d].id.as_str())
                ),
            );
            return (Some(dir), index);
        }
    }
    for dir in DATA_DIRS {
        let path = format!("{dir}{OLD_PARK}");
        if let Some(data) = load_path(&path) {
            let bytes = data.len() as u32;
            unsafe { unload(data) };
            log(
                false,
                &format!("locations: no {INDEX_FILE}, the one park in {path}"),
            );
            let index = menu::Index {
                locations: alloc::vec![Location {
                    id: "park".into(),
                    name: "Imported park".into(),
                    file: OLD_PARK.into(),
                    bytes,
                }],
                default: Some(0),
            };
            return (Some(dir), index);
        }
    }
    log(false, "locations: no park packs, the demo park only");
    (None, menu::Index::default())
}

/// The app folder of a data directory (`sd:/apps/flatground/`).
fn app_dir(data_dir: &str) -> &str {
    data_dir.strip_suffix("data/").unwrap_or(data_dir)
}

/// Rides location `i`: shows the loading screen (`show`), lets go of the park ridden so far and frees its
/// pack, then reads and parses the new one (the demo park needs nothing). Logs the times and the memory
/// before, between and after. On failure the demo park is ridden. Returns whether the location loaded.
fn load_location(game: &mut Game, places: &mut Places, i: usize, show: bool) -> bool {
    let Some(loc) = game.locations.get(i).cloned() else {
        return false;
    };
    let before = stats();
    if show {
        let size = if loc.bytes > 0 {
            format!("  {} KB", loc.bytes / 1024)
        } else {
            String::new()
        };
        game.loading = Some(format!("{}{size}", loc.name.to_ascii_uppercase()));
        // One frame with the message (it waits for the GPU, so nothing draws from the old pack after).
        game.render(&mut WiiGfx);
    }
    game.unload_park();
    if let Some(old) = places.pack.take() {
        // SAFETY: the game let go of the park, and the last frame (the loading screen, or the frame before
        // this request) finished drawing in fg_end_frame.
        unsafe { unload(old) };
    }
    let freed = stats();
    let demo = game.locations.len() - 1;
    let mut ok = true;
    let (mut read_ms, mut parse_ms) = (0.0, 0.0);
    if !loc.is_demo() {
        let t0 = clock_us();
        let data = places
            .data_dir
            .and_then(|d| load_path(&format!("{d}{}", loc.file)));
        let t1 = clock_us();
        read_ms = t1.wrapping_sub(t0) as f32 / 1000.0;
        match data {
            Some(pack) => match game.load_park(pack) {
                Ok(()) => {
                    flush_park(game);
                    places.pack = Some(pack)
                }
                Err(e) => {
                    log(true, &format!("location {}: unreadable ({e:?})", loc.id));
                    unsafe { unload(pack) };
                    ok = false;
                }
            },
            None => {
                log(true, &format!("location {}: no file {}", loc.id, loc.file));
                ok = false;
            }
        }
        parse_ms = clock_us().wrapping_sub(t1) as f32 / 1000.0;
    }
    game.location = if ok { i } else { demo };
    game.loading = None;
    let after = stats();
    let park = match game.imported_park() {
        Some(p) => format!(
            "{} collision triangles, {} rails, {} meshes, {} vertices",
            p.mesh.tris.len(),
            p.mesh.rails.len(),
            p.models.len() + p.chunks.len(),
            p.vertex_count()
        ),
        None => "the demo park".into(),
    };
    log(
        true,
        &format!(
            "location: {} ({}), {} KB read in {read_ms:.0} ms, parsed in {parse_ms:.0} ms: {park} | before: {} | old park freed: {} | loaded: {}",
            loc.name,
            if loc.is_demo() { "built in" } else { &loc.file },
            places.pack.map_or(0, |p| p.len() / 1024),
            memory_text(&before),
            memory_text(&freed),
            memory_text(&after)
        ),
    );
    ok
}

/// Remembers the location the player chose (`settings.txt` in the app folder).
fn save_location(places: &Places, id: &str) {
    let Some(dir) = places.data_dir.map(app_dir) else {
        return;
    };
    let path = format!("{dir}{SETTINGS_FILE}");
    if !save(&path, menu::write_settings(id).as_bytes()) {
        log(false, &format!("settings: could not write {path}"));
    }
}

/// The first of `paths` that exists, read whole. Logs which one (with its size and the build stamp next to
/// it, which says which converter version and disc image made it) or that none was found.
fn load_first(what: &str, paths: &[&CStr]) -> Option<&'static [u8]> {
    for p in paths {
        if let Some(data) = load(p) {
            let path = p.to_str().unwrap_or("?");
            let stamp = path
                .strip_suffix(".fgpak")
                .and_then(|base| alloc::ffi::CString::new(format!("{base}.stamp")).ok())
                .and_then(|s| load(&s))
                .map(|b| {
                    String::from_utf8_lossy(b)
                        .replace('\n', " ")
                        .trim()
                        .to_string()
                })
                .unwrap_or_else(|| "no stamp".into());
            log(
                false,
                &format!("{what}: {path}, {} KB ({stamp})", data.len() / 1024),
            );
            return Some(data);
        }
    }
    log(false, &format!("{what}: no pack found"));
    None
}

/// One line for the session log on the SD card. `important`: written out at once.
fn log(important: bool, line: &str) {
    unsafe { ffi::fg_log(line.as_ptr(), line.len() as u32, important as i32) }
}

fn clock_us() -> u32 {
    unsafe { ffi::fg_time_us() }
}

/// Hands-off play, when `autoplay.txt` is in the app folder.
static mut AUTOPLAY: *mut Autoplay = ptr::null_mut();

/// The one game instance. Only touched from the main thread, through the two entry points below.
static mut GAME: *mut Game = ptr::null_mut();

/// Builds the game (park mesh, models). Called once after the platform is up.
#[unsafe(no_mangle)]
pub extern "C" fn fg_rust_init() {
    log(
        true,
        &format!(
            "flatground {} (build {})",
            env!("CARGO_PKG_VERSION"),
            option_env!("FG_BUILD_ID").unwrap_or("dev")
        ),
    );
    let mut game = Box::new(Game::new());
    game.timer = Some(clock_us);
    game.set_aspect(unsafe { ffi::fg_aspect() });
    // Rules first: the skater and park are set up with its tuning.
    let rules: String = match load_first("rules", RULES_PACKS) {
        Some(pack) => match game.load_rules(pack) {
            Ok((values, shapes)) => {
                log(
                    false,
                    &format!("rules: {values} tuning values, {shapes} flick shapes from the disc"),
                );
                "DISC".into()
            }
            Err(e) => {
                log(
                    true,
                    &format!("rules: unreadable ({e:?}), built-in values used"),
                );
                "UNREADABLE".into()
            }
        },
        None => "BUILT-IN".into(),
    };
    let t = &game.tuning;
    log(
        false,
        &format!(
            "tuning: push max {:.1} m/s, jump {:.2}-{:.2} m, gravity {:.1}, flick misses {}, camera {} shots ({}), {}, aspect {:.2}",
            t.push_max_speed,
            t.jump_low,
            t.jump_high,
            t.gravity,
            t.flick_misses,
            game.camera().data.shots.defs.len(),
            if game.camera().data.shots.imported {
                "disc"
            } else {
                "built-in"
            },
            if game.camera().data.graph.is_some() {
                "Skate 3's camera graph"
            } else {
                "built-in shot choice"
            },
            game.camera().aspect
        ),
    );
    // Without a pack (or with one that doesn't parse) the placeholder stays; say which.
    let skater: String = match load_first("skater", SKATER_PACKS) {
        Some(pack) => match game.load_skater(pack) {
            Ok(()) => {
                if let Some(r) = game.rider() {
                    let c = &r.character;
                    let verts: usize = c.parts.iter().map(|p| p.out.capacity()).sum();
                    log(
                        false,
                        &format!(
                            "skater: {} parts, {verts} skinned vertices, {} bones",
                            c.parts.len(),
                            c.bones()
                        ),
                    );
                }
                "IMPORTED".into()
            }
            Err(e) => {
                log(true, &format!("skater: unreadable: {}", error_text(e)));
                format!("UNREADABLE: {}", error_text(e))
            }
        },
        None => "PLACEHOLDER".into(),
    };
    // The locations, and the one to start in: the last one chosen, else the build's default (autoplay: the
    // default always).
    let autoplay = AUTOPLAY_FILES.iter().find_map(|p| load(p));
    let tour = autoplay.and_then(|a| LocationTour::from_autoplay(&String::from_utf8_lossy(a)));
    let (data_dir, index) = find_locations();
    let last = match (autoplay, data_dir) {
        (None, Some(dir)) => load_text(&format!("{}{SETTINGS_FILE}", app_dir(dir)))
            .and_then(|t| menu::parse_settings(&t)),
        _ => None,
    };
    game.set_locations(index.locations);
    let start = menu::start_location(&game.locations, last.as_deref(), index.default);
    let mut places = Places {
        data_dir,
        pack: None,
        tour,
    };
    let park: String = if game.locations[start].is_demo() {
        "DEMO".into()
    } else if load_location(&mut game, &mut places, start, false) {
        "IMPORTED".into()
    } else {
        "UNREADABLE".into()
    };
    let motion_start = clock_us();
    let motion: String = match load_first("motion", MOTION_PACKS) {
        Some(pack) => match game.load_motion(pack) {
            Ok(lib) => {
                let db = &lib.db;
                let timed = db
                    .clips()
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        db.attributes(i as u16)
                            .iter()
                            .filter(|a| !a.untimed())
                            .count()
                    })
                    .sum::<usize>();
                log(
                    false,
                    &format!(
                        "motion: {} clips ({} with poses), {} trees, {} attribute tracks ({} timed events/windows), {} names",
                        db.clips().len(),
                        lib.pose_count(),
                        db.trees().len(),
                        db.attribute_count(),
                        timed,
                        db.names()
                    ),
                );
                "DISC".into()
            }
            Err(e) => {
                log(true, &format!("motion: unreadable ({e:?})"));
                "UNREADABLE".into()
            }
        },
        None => "NONE".into(),
    };
    let motion_us = clock_us().wrapping_sub(motion_start);
    if let Some(g) = &game.graph {
        let r = &g.runner;
        let (done, nothing, unsupported) = r.coverage();
        let mut st = ffi::FgFrameStats::default();
        unsafe { ffi::fg_frame_stats(&mut st) };
        log(
            false,
            &format!(
                "graph: action {} states, motion {} states / {} transitions / {} operations ({} run, {} no-op, unsupported: {}); {} KB parsed in {:.1} ms (motion pack read + parsed in {:.1} ms); MEM1 {} KB free, MEM2 {} KB free",
                r.action_graph().states.len(),
                r.motion_graph().states.len(),
                r.motion_graph().transitions.len(),
                r.motion_graph().ops.len() + r.action_graph().ops.len(),
                done,
                nothing,
                unsupported.join(" "),
                g.memory / 1024,
                g.load_us as f32 / 1000.0,
                motion_us as f32 / 1000.0,
                st.mem1_free / 1024,
                st.mem2_free / 1024
            ),
        );
    }
    let notice = format!("SKATER: {skater}  PARK: {park}  RULES: {rules}  MOTION: {motion}");
    log(true, &format!("ready: {notice}, stance {:?}", game.stance));
    game.notice = Some(notice);
    game.frame_cost = Some(FrameCost::default());
    if let Some(a) = autoplay {
        let every = places.tour.as_ref().map_or(String::new(), |_| {
            format!(
                ", every location in turn ({})",
                String::from_utf8_lossy(a).trim()
            )
        });
        log(true, &format!("autoplay: on (trick tour, repeated{every})"));
        unsafe { AUTOPLAY = Box::into_raw(Box::new(Autoplay::new())) };
        unsafe { unload(a) };
    } else {
        game.open_start_menu();
    }
    unsafe { PLACES = Box::into_raw(Box::new(places)) };
    unsafe { GAME = Box::into_raw(game) };
}

/// Where the locations come from and the park pack in use.
static mut PLACES: *mut Places = ptr::null_mut();

/// Short, HUD-font-safe (upper case) reason a pack was rejected.
fn error_text(e: character::Error) -> String {
    match e {
        character::Error::Pack => "NOT A PACK OR OLD VERSION".into(),
        character::Error::NoSkeleton => "NO SKELETON".into(),
        character::Error::NoParts => "NO MESHES".into(),
        character::Error::Bad(name) => format!("BAD ENTRY {}", name.to_ascii_uppercase()),
    }
}

/// One displayed frame: the game runs its fixed 60 Hz ticks for `dt` (1/60, or 1/50 on PAL50), then
/// renders, then does what the menus asked (another location, quitting). Returns nonzero to quit.
///
/// # Safety
/// `pad` must point to a valid `FgPadRaw`; `fg_rust_init` must have been called.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fg_rust_frame(pad: *const PadRaw, dt: f32) -> i32 {
    let game = unsafe { &mut *GAME };
    let places = unsafe { &mut *PLACES };
    frame(game, pad, dt);
    // What the menus asked for, after the frame (a load shows its own loading screen).
    match game.take_request() {
        Some(Request::Quit) => {
            log(true, "menu: quit to the loader");
            return 1;
        }
        Some(Request::Load(i)) => {
            if load_location(game, places, i, true) {
                save_location(places, &game.locations[i].id);
            } else {
                game.open_start_menu();
            }
        }
        None => {}
    }
    // Autoplay visiting every location in turn.
    if let (Some(auto), Some(tour)) = (unsafe { AUTOPLAY.as_mut() }, &mut places.tour)
        && let Some(i) = tour.next(game.location, game.locations.len())
    {
        load_location(game, places, i, true);
        auto.new_location();
    }
    0
}

/// Runs and draws one frame, and logs what happened.
fn frame(game: &mut Game, pad: *const PadRaw, dt: f32) {
    let mut pad = unsafe { &*pad };
    // Hands-off play replaces the controller.
    let scripted;
    if let Some(auto) = unsafe { AUTOPLAY.as_mut() } {
        let (p, line) = auto.next(game);
        if let Some(line) = line {
            log(false, &line);
        }
        if auto.take_restart() {
            game.restart_at_spawn();
        }
        if let Some(p) = auto.take_place() {
            game.place(&p);
        }
        scripted = p;
        pad = &scripted;
    }
    let start = clock_us();
    game.update(pad, dt);
    game.render(&mut WiiGfx);
    // The frame's time includes waiting for the GPU and for the TV's refresh (in fg_end_frame): take
    // those out to get the CPU's own work.
    let spent = clock_us().wrapping_sub(start);
    let mut st = ffi::FgFrameStats::default();
    unsafe { ffi::fg_frame_stats(&mut st) };
    let cpu = spent.saturating_sub(st.gpu_wait_us + st.vsync_wait_us);
    if let Some(c) = &mut game.frame_cost {
        c.add(cpu, st.gpu_wait_us, st.vsync_wait_us, st.missed_vsyncs);
    }
    if st.missed_vsyncs > 0 {
        let t = game.times;
        log(
            false,
            &format!(
                "slow frame: {} refresh(es) missed; this frame cpu {:.1} ms (physics {:.1}, skin {:.1}, draw {:.1}), gpu {:.1} ms",
                st.missed_vsyncs,
                cpu as f32 / 1000.0,
                t.physics_us as f32 / 1000.0,
                t.skin_us as f32 / 1000.0,
                t.render_us as f32 / 1000.0,
                st.gpu_wait_us as f32 / 1000.0
            ),
        );
    }
    // Once a second: where the frame time went, and memory.
    unsafe {
        FRAMES += 1;
        if FRAMES.is_multiple_of(FrameCost::WINDOW)
            && let Some(c) = &game.frame_cost
        {
            let t = game.times;
            log(
                false,
                &format!(
                    "frame: cpu {:.1} ms avg, {:.1} peak (physics {:.1}, graph {:.2}, air {:.2} / {:.2} peak, camera {:.2} / {:.2} peak, skin {:.1}, draw {:.1}) | gpu wait {:.1} | idle {:.1} | dropped {} | park {} meshes {} verts | arena {} KB | heap {} KB, MEM1 {} KB free, MEM2 {} KB free",
                    c.avg_us / 1000.0,
                    c.peak_us as f32 / 1000.0,
                    t.physics_us as f32 / 1000.0,
                    t.graph_us as f32 / 1000.0,
                    t.air_us as f32 / 1000.0,
                    t.air_peak_us as f32 / 1000.0,
                    t.cam_us as f32 / 1000.0,
                    t.cam_peak_us as f32 / 1000.0,
                    t.skin_us as f32 / 1000.0,
                    t.render_us as f32 / 1000.0,
                    c.gpu_avg_us / 1000.0,
                    c.idle_avg_us / 1000.0,
                    c.dropped,
                    t.park_draws,
                    t.park_verts,
                    st.arena_used / 1024,
                    st.heap_used / 1024,
                    st.mem1_free / 1024,
                    st.mem2_free / 1024
                ),
            );
            game.times.air_peak_us = 0;
            game.times.cam_peak_us = 0;
        }
    }
    // What the game noticed this frame (flicks, landings, bails, status lines).
    for line in game.log.drain(..) {
        let important = line.starts_with("BAIL") || line.starts_with("error");
        log(important, &line);
    }
}

/// Frames since start, for the once-a-second line.
static mut FRAMES: u32 = 0;
