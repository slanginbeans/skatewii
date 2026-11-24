// flatground sound output: the game mixes in Rust (game_core::sound::mix: 32 kHz stereo PCM16, big-endian,
// interleaved) and hands the samples here once a frame. The Wii's audio interface (AI) plays exactly that
// straight from memory by DMA, so no DSP microcode is involved: libogc's AUDIO_* calls, double buffered from a
// queue of fixed blocks. The AI's DMA interrupt (once a block starts playing) programs the next block, so a
// slow frame doesn't stop the sound; when the queue runs dry it plays silence (counted as an underrun) until
// the game catches up.
//
// (libogc's ASND mixer was tried first: in Dolphin its DMA interrupt handler crashed on an invalid pointer in
// DSP_SendMailTo, and its voices never asked for data. Mixing in Rust needs no DSP mixer at all.)

#include <gccore.h>
#include <string.h>

#include "fg_platform.h"

// A block is 512 stereo frames (16 ms at 32 kHz, 2 KiB: a multiple of 32 bytes as the AI's DMA needs).
#define BLOCK_FRAMES 512
#define BLOCK_BYTES (BLOCK_FRAMES * 4)
#define BLOCKS 16
// The queue is kept this many blocks deep (64 ms; one more plays and one is programmed): latency against
// riding out a dropped frame or two.
#define TARGET_BLOCKS 4

static int16_t blocks[BLOCKS][BLOCK_FRAMES * 2] __attribute__((aligned(32)));
static int16_t silence[BLOCK_FRAMES * 2] __attribute__((aligned(32)));
// The queue: blocks [head, tail) are filled and waiting; the interrupt takes from head, the game fills at
// tail. The two the AI holds (playing and programmed next) stay out of the way: the game never writes within
// 2 of head.
static volatile uint32_t head, tail;
// The block being filled, and how many frames it has.
static uint32_t fill;
static volatile uint32_t underruns;

static uint32_t queued(void) {
    return (tail - head) % BLOCKS;
}

// The AI started playing the block programmed last: program the one after it.
static void dma_callback(void) {
    const void *next = silence;
    if (head != tail) {
        next = blocks[head];
        head = (head + 1) % BLOCKS;
    } else {
        underruns++;
    }
    AUDIO_InitDMA((u32)next, BLOCK_BYTES);
}

void fg_audio_init(void) {
    memset(silence, 0, sizeof silence);
    DCFlushRange(silence, sizeof silence);
    AUDIO_Init(NULL);
    AUDIO_SetDSPSampleRate(AI_SAMPLERATE_32KHZ);
    AUDIO_RegisterDMACallback(dma_callback);
    // Silence until the game's first samples come.
    AUDIO_InitDMA((u32)silence, BLOCK_BYTES);
    AUDIO_StartDMA();
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
        // The block at tail must not be one the AI still holds, nor wrap onto head.
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
}
