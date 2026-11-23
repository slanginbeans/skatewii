// flatground entry point: bring up the platform, then one Rust frame per vsync until the player quits (HOME, START + Z, or
// Quit in the pause menu).

#include <stdlib.h>

#include "fg_platform.h"

int main(int argc, char **argv) {
    fg_platform_init();
    fg_log_start(argc > 0 ? argv[0] : NULL);
    fg_rust_init();
    const float refresh = fg_frame_dt();
    for (;;) {
        // A frame covers the time the last one was on screen, so a frame that runs past a refresh doesn't
        // slow the game down (it runs more fixed steps instead). Capped so a long stall (a load) doesn't
        // jump the rider ahead.
        uint32_t n = fg_frame_refreshes();
        const float dt = refresh * (float)(n < 1 ? 1 : n > 4 ? 4 : n);
        FgPadRaw pad;
        fg_read_pad(&pad);
        if (pad.flags & FG_FLAG_EXIT) {
            break;
        }
        int32_t quit = fg_rust_frame(&pad, dt);
        fg_log_tick();
        if (quit) {
            break;
        }
    }
    fg_log_stop();
    // Returning from main goes back to the Homebrew Channel / loader.
    return 0;
}
