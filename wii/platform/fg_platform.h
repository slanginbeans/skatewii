// flatground platform layer: the whole API between the Rust game and libogc.
// Keep in sync with wii/game/src/ffi.rs and game_core::{gfx::Vertex, gfx::TexVertex, input::PadRaw}.
#ifndef FG_PLATFORM_H
#define FG_PLATFORM_H

#include <stdint.h>

// One vertex: position + pre-lit colour 0xRRGGBBAA. 16 bytes (game_core::gfx::Vertex).
typedef struct {
    float pos[3];
    uint32_t rgba;
} FgVertex;

// Vertex of an indexed mesh: position, pre-lit colour (multiplied with the texture), texture coordinate.
// 24 bytes (game_core::gfx::TexVertex).
typedef struct {
    float pos[3];
    uint32_t rgba;
    float uv[2];
} FgTexVertex;

// A texture already in a GX format, in main memory: 32-byte aligned and flushed from the CPU cache
// (data from fg_load_file is both). 12 bytes (wii/game/src/ffi.rs FgTexture).
typedef struct {
    const void *data;
    uint16_t width, height;
    uint8_t format;     // GX_TF_*
    uint8_t alpha_test; // 1: discard texels with alpha < 128
    uint16_t _pad;
} FgTexture;

// Pad state for one frame (game_core::input::PadRaw). 16 bytes.
typedef struct {
    int8_t stick_x, stick_y;   // main stick, ~ -100..100, +y up
    int8_t cstick_x, cstick_y; // C-stick, ~ -80..80
    uint8_t trig_l, trig_r;    // 0..255
    uint16_t _pad;
    uint32_t buttons;          // FG_BTN_*
    uint32_t flags;            // FG_FLAG_*
} FgPadRaw;

// Button bits (game_core::input::btn).
#define FG_BTN_A          (1u << 0)
#define FG_BTN_B          (1u << 1)
#define FG_BTN_X          (1u << 2)
#define FG_BTN_Y          (1u << 3)
#define FG_BTN_Z          (1u << 4)
#define FG_BTN_START      (1u << 5)
#define FG_BTN_L          (1u << 6)
#define FG_BTN_R          (1u << 7)
#define FG_BTN_DPAD_UP    (1u << 8)
#define FG_BTN_DPAD_DOWN  (1u << 9)
#define FG_BTN_DPAD_LEFT  (1u << 10)
#define FG_BTN_DPAD_RIGHT (1u << 11)

// Flag bits (game_core::input::flag).
#define FG_FLAG_CONNECTED (1u << 0)
#define FG_FLAG_EXIT      (1u << 1)

// What the platform measured over the last frame, and memory. 32 bytes (wii/game/src/ffi.rs FgFrameStats).
typedef struct {
    uint32_t gpu_wait_us;   // in fg_end_frame, waiting for the GPU to finish drawing (GX_DrawDone)
    uint32_t vsync_wait_us; // then waiting for the TV's next refresh (idle time)
    uint32_t missed_vsyncs; // refreshes that went by without a new frame since the last call (dropped frames)
    uint32_t arena_used;    // bytes of the per-frame vertex arena used last frame
    uint32_t mem1_free;     // bytes free in the MEM1 / MEM2 arenas
    uint32_t mem2_free;
    uint32_t heap_used;     // bytes the C heap (Rust's allocator) has handed out
    uint32_t _pad;
} FgFrameStats;

// ---- Called from C main ----
void fg_platform_init(void);
// Opens the session log in <app folder>/logs (argv0: where the loader started us, may be NULL) and writes
// the platform's own lines (video mode, storage). Call after fg_platform_init.
void fg_log_start(const char *argv0);
// Once a frame: writes buffered log lines out every couple of seconds.
void fg_log_tick(void);
void fg_log_stop(void);
// Seconds per frame: 1/50 on PAL50, else 1/60.
float fg_frame_dt(void);
void fg_read_pad(FgPadRaw *out);

// ---- Called from Rust ----
void fg_begin_frame(const float eye[3], const float target[3], const float up[3], float fov_y_deg, float near_z,
                    float far_z, uint32_t clear_rgba);
// The TV's aspect (width over height: 4/3, or 16/9 when the Wii is set to widescreen).
float fg_aspect(void);
// Triangle list (count is a multiple of 3, at most 65535) placed by a row-major 3x4 model matrix.
void fg_draw_tris(const FgVertex *verts, uint32_t count, const float model[3][4]);
// Screen-space triangle list in a 640x480 virtual screen, origin top-left, drawn over the scene.
void fg_draw_2d(const FgVertex *verts, uint32_t count);
// Indexed triangle list (index_count a multiple of 3, at most 65535; indices < vert_count <= 65536) placed
// by a row-major 3x4 model matrix, coloured by the vertex colour times `tex` (NULL: vertex colour only).
// The vertices are copied, so the caller's buffer only has to live for the call.
void fg_draw_indexed(const FgTexVertex *verts, uint32_t vert_count, const uint16_t *indices,
                     uint32_t index_count, const FgTexture *tex, const float model[3][4]);
// Microseconds from a free-running CPU timer (wraps every ~71 minutes; subtract with wrap-around).
uint32_t fg_time_us(void);
// Reads a whole file (e.g. "sd:/apps/flatground/data/skater.fgpak") into 32-byte aligned memory, flushed for
// the GPU. NULL if there is no such file or no SD/USB storage. Kept until fg_free_file (packs the game keeps
// for the whole session are never freed).
void *fg_load_file(const char *path, uint32_t *size);
// Gives back what fg_load_file returned (NULL: nothing). Nothing may use it afterwards, the GPU included: call
// it after the frame that last drew from it has finished (fg_end_frame waits for that).
void fg_free_file(void *data);
// Writes a whole (small) file, replacing it: settings in the app folder. 0 on success, -1 if it couldn't.
int32_t fg_save_file(const char *path, const void *data, uint32_t size);
void fg_end_frame(void);
// One line for the session log (no trailing newline needed). `important`: written to the card at once.
void fg_log(const char *line, uint32_t len, int32_t important);
// The last frame's timings and memory (see FgFrameStats).
void fg_frame_stats(FgFrameStats *out);
// Logs it, shows it on screen with the log's last lines, and waits for HOME / START to return to the loader.
void fg_panic(const char *msg, uint32_t len) __attribute__((noreturn));

// ---- Provided by Rust (libfg_game.a) ----
void fg_rust_init(void);
// Nonzero: the player chose to quit to the loader.
int32_t fg_rust_frame(const FgPadRaw *pad, float dt);

#endif
