// flatground entry point: bring up the platform, then one Rust frame per vsync until the player quits.

#include <stdlib.h>

#include "fg_platform.h"

int main(int argc, char **argv) {
    fg_platform_init();
    fg_log_start(argc > 0 ? argv[0] : NULL);
    fg_rust_init();
    const float dt = fg_frame_dt();
    for (;;) {
        FgPadRaw pad;
        fg_read_pad(&pad);
        if (pad.flags & FG_FLAG_EXIT) {
            break;
        }
        fg_rust_frame(&pad, dt);
        fg_log_tick();
    }
    fg_log_stop();
    // Returning from main goes back to the Homebrew Channel / loader.
    return 0;
}
