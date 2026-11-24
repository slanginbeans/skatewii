//! Locations: the menus drive the game, and switching parks at runtime starts the rider over at the new
//! park's spawn without leaking memory (synthetic parks built here).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use game_core::Game;
use game_core::collision;
use game_core::input::{PadRaw, btn, flag};
use game_core::menu::{Location, Request, Screen};
use game_core::pack::{Kind, name_hash};
use game_core::world::{MESH_ALPHA_TEST, encode_mesh};

/// Counts the bytes this thread has live (tests run on several threads).
struct Counting;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
}

fn track(d: isize) {
    let _ = LIVE.try_with(|l| l.set(l.get() + d));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        track(l.size() as isize);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        track(-(l.size() as isize));
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        track(n as isize - l.size() as isize);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn live() -> isize {
    LIVE.with(|l| l.get())
}

/// One pack entry: name, kind, data, width, height, GX format.
type Entry<'a> = (&'a str, Kind, Vec<u8>, u16, u16, u8);

/// A pack in memory (the format of `game_core::pack`).
fn pack(items: &[Entry]) -> Vec<u8> {
    let mut strings = vec![];
    let mut name_at = vec![];
    for (n, ..) in items {
        name_at.push(strings.len() as u32);
        strings.extend_from_slice(n.as_bytes());
        strings.push(0);
    }
    let strings_at = 16 + 32 * items.len();
    let mut data_at = (strings_at + strings.len()).next_multiple_of(32);
    let mut out = b"FGPA".to_vec();
    for x in [1u32, items.len() as u32, strings_at as u32] {
        out.extend_from_slice(&x.to_be_bytes());
    }
    let mut offsets = vec![];
    for (i, (n, kind, d, w, h, f)) in items.iter().enumerate() {
        for x in [
            name_hash(n),
            name_at[i],
            *kind as u32,
            data_at as u32,
            d.len() as u32,
        ] {
            out.extend_from_slice(&x.to_be_bytes());
        }
        out.extend_from_slice(&w.to_be_bytes());
        out.extend_from_slice(&h.to_be_bytes());
        out.extend_from_slice(&[*f, 0, 0, 0, 0, 0, 0, 0]);
        offsets.push(data_at);
        data_at = (data_at + d.len()).next_multiple_of(32);
    }
    out.extend_from_slice(&strings);
    for (i, (_, _, d, ..)) in items.iter().enumerate() {
        out.resize(offsets[i], 0);
        out.extend_from_slice(d);
    }
    out
}

/// A flat 60 m square at height `y` with the spawn at `spawn` facing `yaw`, drawn by one textured quad.
fn park(y: f32, spawn: [f32; 3], yaw: f32) -> &'static [u8] {
    let (a, b) = (-30.0, 30.0);
    let floor = collision::encode(&[
        ([[a, y, a], [a, y, b], [b, y, b]], 1, 0),
        ([[a, y, a], [b, y, b], [b, y, a]], 1, 0),
    ]);
    let quad = [
        ([a, y, a], 0xffff_ffff, [0.0, 0.0]),
        ([b, y, a], 0xffff_ffff, [1.0, 0.0]),
        ([b, y, b], 0xffff_ffff, [1.0, 1.0]),
        ([a, y, b], 0xffff_ffff, [0.0, 1.0]),
    ];
    let mesh = encode_mesh(&quad, &[0, 1, 2, 0, 2, 3], Some(0), MESH_ALPHA_TEST);
    let spawn: Vec<u8> = [spawn[0], spawn[1], spawn[2], yaw]
        .iter()
        .flat_map(|f| f.to_be_bytes())
        .collect();
    let bytes = pack(&[
        ("park/collision", Kind::Collision, floor, 0, 0, 0),
        ("park/spawn", Kind::Data, spawn, 0, 0, 0),
        ("park/mesh/0", Kind::Data, mesh, 0, 0, 0),
        ("park/tex/0", Kind::Texture, vec![0x80; 32], 8, 4, 14),
    ]);
    // The platform keeps the pack until it unloads the park; leaked once here (both parks are reused).
    Vec::leak(bytes)
}

fn held(buttons: u32) -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        buttons,
        ..Default::default()
    }
}

fn locations() -> Vec<Location> {
    ["A", "B"]
        .iter()
        .map(|n| Location {
            id: (*n).into(),
            name: format!("Park {n}"),
            file: format!("parks/{n}.fgpak"),
            bytes: 0,
            district: None,
        })
        .collect()
}

/// What the platform does with a request: load the location's pack (or the demo park).
fn serve(g: &mut Game, parks: &[&'static [u8]]) -> Option<Request> {
    let r = g.take_request();
    if let Some(Request::Load(i)) = r {
        g.unload_park();
        if let Some(p) = parks.get(i) {
            g.load_park(p).unwrap();
        }
        g.location = i;
    }
    r
}

fn ride(g: &mut Game, frames: usize, buttons: u32) {
    for _ in 0..frames {
        g.update(&held(buttons), 1.0 / 60.0);
        g.log.clear();
    }
}

#[test]
fn switching_parks_starts_over_at_the_new_spawn() {
    let parks = [
        park(0.0, [1.0, 0.0, 2.0], 0.0),
        park(3.0, [5.0, 3.0, -4.0], 1.0),
    ];
    let mut g = Game::new();
    g.set_locations(locations());
    assert_eq!(g.locations.len(), 3, "the demo park is added");
    assert_eq!(g.location, 2, "the demo park until something loads");

    g.load_park(parks[0]).unwrap();
    g.location = 0;
    assert!((g.skater.pos - game_core::math::v3(1.0, 0.0, 2.0)).length() < 0.05);
    // Ride off somewhere.
    ride(&mut g, 90, btn::A);
    ride(&mut g, 30, 0);
    assert!(g.skater.speed.abs() > 0.5);
    assert!((g.skater.pos - game_core::math::v3(1.0, 0.0, 2.0)).length() > 1.0);

    g.unload_park();
    g.load_park(parks[1]).unwrap();
    g.location = 1;
    let s = &g.skater;
    assert!(
        (s.pos - game_core::math::v3(5.0, 3.0, -4.0)).length() < 0.05,
        "{:?}",
        s.pos
    );
    assert_eq!(s.speed, 0.0);
    assert!(s.checkpoints.is_empty());
    // Nothing is blended from the old park: the drawn rider is where the rider is, and the camera cuts to
    // it on the next frame.
    assert_eq!(g.drawn_pos(), g.skater.pos);
    ride(&mut g, 1, 0);
    let eye = g.drawn_camera().eye;
    assert!(
        (eye - g.skater.pos).length() < 10.0,
        "camera at {eye:?}, rider at {:?}",
        g.skater.pos
    );
    // A pack that doesn't parse leaves the park as it was.
    assert!(g.load_park(Vec::leak(vec![0u8; 64])).is_err());
    assert!(g.imported_park().is_some());
    // Back to the demo park.
    g.unload_park();
    assert!(g.imported_park().is_none());
    assert_eq!(g.skater.pos, g.terrain().spawn().0);
}

#[test]
fn switching_back_and_forth_leaks_nothing() {
    let parks = [
        park(0.0, [1.0, 0.0, 2.0], 0.0),
        park(3.0, [5.0, 3.0, -4.0], 1.0),
    ];
    let mut g = Game::new();
    g.set_locations(locations());
    let cycle = |g: &mut Game| {
        for p in [parks[1], parks[0]] {
            g.unload_park();
            g.load_park(p).unwrap();
            ride(g, 40, btn::A);
            ride(g, 20, 0);
        }
        g.unload_park();
        ride(g, 20, 0);
        live()
    };
    // The first rounds may grow buffers that are then reused; after that every round ends where the
    // last one did.
    cycle(&mut g);
    let a = cycle(&mut g);
    let b = cycle(&mut g);
    let c = cycle(&mut g);
    assert_eq!(a, b, "{a} then {b} bytes live");
    assert_eq!(b, c, "{b} then {c} bytes live");
}

#[test]
fn the_menus_take_the_pad_and_ask_for_locations() {
    let parks = [
        park(0.0, [1.0, 0.0, 2.0], 0.0),
        park(3.0, [5.0, 3.0, -4.0], 1.0),
    ];
    let mut g = Game::new();
    g.set_locations(locations());
    g.load_park(parks[0]).unwrap();
    g.location = 0;
    g.open_start_menu();
    // The simulation waits while a menu is open.
    let ticks = g.ticks;
    ride(&mut g, 10, 0);
    ride(&mut g, 1, btn::DPAD_DOWN);
    ride(&mut g, 1, 0);
    assert_eq!(g.ticks, ticks);
    assert_eq!(g.menu.as_ref().unwrap().cursor, 1);
    ride(&mut g, 1, btn::A);
    assert!(g.menu.is_none());
    assert_eq!(serve(&mut g, &parks), Some(Request::Load(1)));
    assert!((g.skater.pos - game_core::math::v3(5.0, 3.0, -4.0)).length() < 0.05);

    // The A that chose the location is still held: not a push until it is let go and pressed again.
    ride(&mut g, 60, btn::A);
    assert!(g.skater.push.is_none() && g.skater.speed == 0.0);
    assert!(g.ticks > ticks);
    ride(&mut g, 1, 0);
    ride(&mut g, 60, btn::A);
    assert!(g.skater.speed.abs() > 0.1, "{}", g.skater.speed);

    // START pauses; the d-pad moves the menu, not the rider (no gesture, no ticks).
    ride(&mut g, 30, 0);
    ride(&mut g, 1, btn::START);
    let (ticks, pos) = (g.ticks, g.skater.pos);
    assert_eq!(g.menu.as_ref().unwrap().screen, Screen::Pause);
    ride(&mut g, 1, 0);
    ride(&mut g, 1, btn::DPAD_DOWN);
    ride(&mut g, 1, 0);
    ride(&mut g, 1, btn::A);
    ride(&mut g, 1, 0);
    assert_eq!(g.menu.as_ref().unwrap().screen, Screen::Locations);
    assert_eq!(
        g.menu.as_ref().unwrap().cursor,
        1,
        "opens on the park ridden"
    );
    // Down to the demo park and ride it.
    ride(&mut g, 1, btn::DPAD_DOWN);
    ride(&mut g, 1, 0);
    ride(&mut g, 1, btn::A);
    assert_eq!((g.ticks, g.skater.pos), (ticks, pos));
    assert_eq!(serve(&mut g, &parks), Some(Request::Load(2)));
    assert!(g.imported_park().is_none());
    assert_eq!(g.location, 2);

    // Choosing the park being ridden asks for nothing; Respawn puts the rider back; Quit asks to quit.
    ride(&mut g, 1, 0);
    ride(&mut g, 1, btn::START);
    ride(&mut g, 1, 0);
    ride(&mut g, 1, btn::B);
    assert!(g.menu.is_none(), "B resumes");
    assert_eq!(g.take_request(), None);
    ride(&mut g, 1, 0);
    ride(&mut g, 60, btn::B);
    ride(&mut g, 30, 0);
    let spawn = g.terrain().spawn().0;
    assert!((g.skater.pos - spawn).length() > 0.5);
    ride(&mut g, 1, btn::START);
    ride(&mut g, 1, 0);
    // Down past Change location, Draw distance and Camera.
    for _ in 0..4 {
        ride(&mut g, 1, btn::DPAD_DOWN);
        ride(&mut g, 1, 0);
    }
    ride(&mut g, 1, btn::A);
    assert_eq!(g.skater.pos, spawn, "respawned");
    ride(&mut g, 1, 0);
    ride(&mut g, 1, btn::START);
    ride(&mut g, 1, 0);
    ride(&mut g, 1, btn::DPAD_UP);
    ride(&mut g, 1, 0);
    ride(&mut g, 1, btn::A);
    assert_eq!(g.take_request(), Some(Request::Quit));
}
