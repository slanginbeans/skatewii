//! The C platform layer as Rust sees it. Must match `wii/platform/fg_platform.h` exactly.

use game_core::gfx::Vertex;
use game_core::math::Mat34;

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
    pub fn fg_draw_2d(verts: *const Vertex, count: u32);
    pub fn fg_end_frame();
    pub fn fg_panic(msg: *const u8, len: u32) -> !;
}
