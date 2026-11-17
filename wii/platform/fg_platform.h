// flatground platform layer: the whole API between the Rust game and libogc.
// Keep in sync with wii/game/src/ffi.rs and game_core::{gfx::Vertex, input::PadRaw}.
#ifndef FG_PLATFORM_H
#define FG_PLATFORM_H

#include <stdint.h>

// One vertex: position + pre-lit colour 0xRRGGBBAA. 16 bytes (game_core::gfx::Vertex).
typedef struct {
    float pos[3];
    uint32_t rgba;
} FgVertex;

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

// ---- Called from C main ----
void fg_platform_init(void);
// Seconds per frame: 1/50 on PAL50, else 1/60.
float fg_frame_dt(void);
void fg_read_pad(FgPadRaw *out);

// ---- Called from Rust ----
void fg_begin_frame(const float eye[3], const float target[3], float fov_y_deg, float near_z, float far_z,
                    uint32_t clear_rgba);
// Triangle list (count is a multiple of 3, at most 65535) placed by a row-major 3x4 model matrix.
void fg_draw_tris(const FgVertex *verts, uint32_t count, const float model[3][4]);
// Screen-space triangle list in a 640x480 virtual screen, origin top-left, drawn over the scene.
void fg_draw_2d(const FgVertex *verts, uint32_t count);
void fg_end_frame(void);
// Shows the message on screen and waits for HOME / START to return to the loader.
void fg_panic(const char *msg, uint32_t len) __attribute__((noreturn));

// ---- Provided by Rust (libfg_game.a) ----
void fg_rust_init(void);
void fg_rust_frame(const FgPadRaw *pad, float dt);

#endif
