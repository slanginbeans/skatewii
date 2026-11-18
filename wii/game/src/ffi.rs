//! The C platform layer as Rust sees it. Must match `wii/platform/fg_platform.h` exactly.

use core::ffi::c_char;

use game_core::gfx::{TexVertex, Vertex};
use game_core::math::Mat34;

/// `FgTexture`: a GX texture in main memory.
#[repr(C)]
pub struct FgTexture {
    pub data: *const u8,
    pub width: u16,
    pub height: u16,
    pub format: u8,
    pub alpha_test: u8,
    pub _pad: u16,
}

// The header's layout on the Wii (32-bit pointers).
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(core::mem::size_of::<FgTexture>() == 12);
    assert!(core::mem::offset_of!(FgTexture, format) == 8);
};

unsafe extern "C" {
    pub fn fg_begin_frame(
        eye: *const f32,
        target: *const f32,
        fov_y_deg: f32,
        near: f32,
        far: f32,
        clear_rgba: u32,
    );
    pub fn fg_draw_tris(verts: *const Vertex, count: u32, model: *const Mat34);
    pub fn fg_draw_indexed(
        verts: *const TexVertex,
        vert_count: u32,
        indices: *const u16,
        index_count: u32,
        tex: *const FgTexture,
        model: *const Mat34,
    );
    pub fn fg_draw_2d(verts: *const Vertex, count: u32);
    pub fn fg_end_frame();
    pub fn fg_load_file(path: *const c_char, size: *mut u32) -> *mut u8;
    pub fn fg_panic(msg: *const u8, len: u32) -> !;
}
