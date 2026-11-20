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
use alloc::string::String;
use core::ffi::CStr;
use core::ptr;

use game_core::character;
use game_core::game::{FrameCost, Game};
use game_core::gfx::{Camera, Gfx, TexVertex, Texture, Vertex};
use game_core::input::PadRaw;
use game_core::math::Mat34;

/// Max vertices per `GX_Begin` (it takes a u16), rounded down to whole triangles.
const MAX_BATCH: usize = 65535 / 3 * 3;

struct WiiGfx;

impl Gfx for WiiGfx {
    fn begin_frame(&mut self, cam: &Camera, clear_rgba: u32) {
        let eye = [cam.eye.x, cam.eye.y, cam.eye.z];
        let target = [cam.target.x, cam.target.y, cam.target.z];
        unsafe {
            ffi::fg_begin_frame(
                eye.as_ptr(),
                target.as_ptr(),
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
                ffi::fg_draw_indexed(
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
    fn draw_2d(&mut self, verts: &[Vertex]) {
        for chunk in verts.chunks(MAX_BATCH) {
            unsafe { ffi::fg_draw_2d(chunk.as_ptr(), chunk.len() as u32) }
        }
    }
    fn end_frame(&mut self) {
        unsafe { ffi::fg_end_frame() }
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

/// The park `fg_import park` converts from the player's disc (collision; drawn from it for now).
const PARK_PACKS: &[&CStr] = &[
    c"sd:/apps/flatground/data/park.fgpak",
    c"usb:/apps/flatground/data/park.fgpak",
];

/// The first of `paths` that exists, read whole. Loaded once and kept for the rest of the game: the GPU
/// reads textures straight out of it.
fn load_first(paths: &[&CStr]) -> Option<&'static [u8]> {
    paths.iter().find_map(|p| {
        let mut size = 0u32;
        let data = unsafe { ffi::fg_load_file(p.as_ptr(), &mut size) };
        (!data.is_null()).then(|| unsafe { core::slice::from_raw_parts(data, size as usize) })
    })
}

/// The one game instance. Only touched from the main thread, through the two entry points below.
static mut GAME: *mut Game = ptr::null_mut();

/// Builds the game (park mesh, models). Called once after the platform is up.
#[unsafe(no_mangle)]
pub extern "C" fn fg_rust_init() {
    let mut game = Box::new(Game::new());
    // Without a pack (or with one that doesn't parse) the placeholder stays; say which.
    let skater: String = match load_first(SKATER_PACKS) {
        Some(pack) => match game.load_skater(pack) {
            Ok(()) => "IMPORTED".into(),
            Err(e) => format!("UNREADABLE: {}", error_text(e)),
        },
        None => "PLACEHOLDER".into(),
    };
    let park: String = match load_first(PARK_PACKS) {
        Some(pack) => match game.load_park(pack) {
            Ok(()) => "IMPORTED".into(),
            Err(_) => "UNREADABLE".into(),
        },
        None => "DEMO".into(),
    };
    game.notice = Some(format!("SKATER: {skater}   PARK: {park}"));
    game.frame_cost = Some(FrameCost::default());
    unsafe { GAME = Box::into_raw(game) };
}

/// Short, HUD-font-safe (upper case) reason a pack was rejected.
fn error_text(e: character::Error) -> String {
    match e {
        character::Error::Pack => "NOT A PACK OR OLD VERSION".into(),
        character::Error::NoSkeleton => "NO SKELETON".into(),
        character::Error::NoParts => "NO MESHES".into(),
        character::Error::Bad(name) => format!("BAD ENTRY {}", name.to_ascii_uppercase()),
    }
}

/// One fixed step plus one rendered frame.
///
/// # Safety
/// `pad` must point to a valid `FgPadRaw`; `fg_rust_init` must have been called.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fg_rust_frame(pad: *const PadRaw, dt: f32) {
    let game = unsafe { &mut *GAME };
    let pad = unsafe { &*pad };
    let start = unsafe { ffi::fg_time_us() };
    game.update(pad, dt);
    game.render(&mut WiiGfx);
    // CPU time of this frame (the GPU works on in parallel), shown from the next frame on.
    let spent = unsafe { ffi::fg_time_us() }.wrapping_sub(start);
    if let Some(c) = &mut game.frame_cost {
        c.add(spent);
    }
}
