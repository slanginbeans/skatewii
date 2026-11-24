// flatground sound output: the game mixes in Rust (game_core::sound::mix, 32 kHz stereo PCM16) and hands the
// samples here once a frame; libogc's ASND plays them on one stereo voice, double buffered from a queue of
// fixed blocks. The DSP's interrupt takes the next block itself, so a slow frame or a short stall doesn't
// stop the sound; when the queue runs dry it plays silence (counted as underruns) until the game catches up.

#include <gccore.h>
#include <asndlib.h>
#include <string.h>

#include "fg_platform.h"

// A block is 512 stereo frames (16 ms at 32 kHz, 2 KiB: a multiple of 32 bytes as ASND's DMA needs).
#define BLOCK_FRAMES 512
#define BLOCK_BYTES (BLOCK_FRAMES * 4)
#define BLOCKS 16
// The queue is kept about this many blocks deep (64 ms): latency against riding out a dropped frame or two.
#define TARGET_BLOCKS 4
#define VOICE 0

static int16_t blocks[BLOCKS][BLOCK_FRAMES * 2] __attribute__((aligned(32)));
static int16_t silence[BLOCK_FRAMES * 2] __attribute__((aligned(32)));
// The queue: blocks [head, tail) are filled and waiting; the interrupt takes from head, the game fills at
// tail. The two the DSP holds (playing and next) stay out of the way: the game never writes within 2 of head.
static volatile uint32_t head, tail;
// The block being filled, and how many frames it has.
static uint32_t fill;
static volatile uint32_t underruns;
static int started;

static uint32_t queued(void) {
    return (tail - head) % BLOCKS;
}

// The voice wants its next buffer (ASND calls this from its interrupt when the second buffer is free).
static void voice_cb(s32 voice) {
    void *next = silence;
    if (head != tail) {
        next = blocks[head];
        head = (head + 1) % BLOCKS;
    } else {
        underruns++;
    }
    ASND_AddVoice(voice, next, BLOCK_BYTES);
}

void fg_audio_init(void) {
    memset(silence, 0, sizeof silence);
    DCFlushRange(silence, sizeof silence);
    ASND_Init();
    ASND_Pause(0);
}

uint32_t fg_audio_wanted(void) {
    uint32_t q = queued();
    if (q >= TARGET_BLOCKS) {
        return 0;
    }
    return (TARGET_BLOCKS - q) * BLOCK_FRAMES - fill;
}

uint32_t fg_audio_underruns(void) {
    return underruns;
}

void fg_audio_submit(const int16_t *frames, uint32_t count) {
    while (count > 0) {
        // The block at tail must not be one the DSP still holds, nor wrap onto head.
        if (queued() >= BLOCKS - 3) {
            return;
        }
        uint32_t n = BLOCK_FRAMES - fill;
        if (n > count) {
            n = count;
        }
        memcpy(&blocks[tail][fill * 2], frames, n * 4);
        fill += n;
        frames += n * 2;
        count -= n;
        if (fill == BLOCK_FRAMES) {
            DCFlushRange(blocks[tail], BLOCK_BYTES);
            fill = 0;
            tail = (tail + 1) % BLOCKS;
        }
    }
    // Start the voice once there is something to play (and again if it ever stopped).
    if (!started || ASND_StatusVoice(VOICE) == SND_UNUSED) {
        if (queued() >= 2) {
            void *first = blocks[head];
            head = (head + 1) % BLOCKS;
            ASND_SetVoice(VOICE, VOICE_STEREO_16BIT, FG_AUDIO_RATE, 0, first, BLOCK_BYTES, 255, 255, voice_cb);
            started = 1;
        }
    }
}
