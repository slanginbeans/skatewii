// flatground entry point: bring up the platform, then one Rust frame per vsync until the player quits.

#include <stdlib.h>

#include "fg_platform.h"

int main(void) {
    fg_platform_init();
    fg_rust_init();
    const float dt = fg_frame_dt();
    for (;;) {
        FgPadRaw pad;
        fg_read_pad(&pad);
        if (pad.flags & FG_FLAG_EXIT) {
            break;
        }
        fg_rust_frame(&pad, dt);
    }
    // Returning from main goes back to the Homebrew Channel / loader.
    return 0;
}
