// flatground platform layer over libogc: video + GX setup, a vertex-colour 3D pass, a 2D overlay
// pass, indexed textured meshes (imported characters), file loading from SD/USB, GameCube pad + Wii
// Remote HOME input, and a panic screen.

#include <gccore.h>
#include <wiiuse/wpad.h>
#include <fat.h>
#include <ogc/lwp_watchdog.h>
#include <ogc/cond.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <malloc.h>
#include <time.h>

#include "fg_log.h"
#include "fg_read.h"
#include "fg_platform.h"

#define FIFO_SIZE (256 * 1024)
// Per-frame copies of indexed vertex arrays (the GPU reads them after fg_draw_indexed returns).
#define ARENA_SIZE (1024 * 1024)

static GXRModeObj *rmode;
static void *xfb[2];
static int cur_fb;
static float aspect = 4.0f / 3.0f;
static Mtx view;
static Mtx44 proj;
static int in_2d;
static u8 *arena;
static u32 arena_used;
// Which vertex/TEV setup is loaded: direct colour, indexed colour, or indexed textured.
enum { MODE_DIRECT, MODE_INDEXED, MODE_TEXTURED };
static int mode = MODE_DIRECT;
static int alpha_test_on;
// Last frame's waits, measured in fg_end_frame, and the arena use behind it.
static u32 gpu_wait_us, vsync_wait_us, last_arena_used, last_retrace, missed;
// Refreshes so far, counted by the video interrupt.
static volatile u32 retraces;
static void on_retrace(u32 count) {
    retraces = count;
}
// Storage: 0 untried, 1 mounted, -1 none.
static int fat_state;

static int mount(void) {
    if (fat_state == 0) {
        fat_state = fatInitDefault() ? 1 : -1;
    }
    return fat_state > 0;
}

void fg_platform_init(void) {
    VIDEO_Init();
    PAD_Init();
    WPAD_Init();

    VIDEO_SetPostRetraceCallback(on_retrace);
    rmode = VIDEO_GetPreferredMode(NULL);
    if (CONF_GetAspectRatio() == CONF_ASPECT_16_9) {
        aspect = 16.0f / 9.0f;
    }
    xfb[0] = MEM_K0_TO_K1(SYS_AllocateFramebuffer(rmode));
    xfb[1] = MEM_K0_TO_K1(SYS_AllocateFramebuffer(rmode));
    VIDEO_Configure(rmode);
    VIDEO_SetNextFramebuffer(xfb[cur_fb]);
    VIDEO_SetBlack(false);
    VIDEO_Flush();
    VIDEO_WaitVSync();
    if (rmode->viTVMode & VI_NON_INTERLACE) {
        VIDEO_WaitVSync();
    }

    void *fifo = memalign(32, FIFO_SIZE);
    memset(fifo, 0, FIFO_SIZE);
    GX_Init(fifo, FIFO_SIZE);

    GXColor sky = {150, 196, 232, 0xff};
    GX_SetCopyClear(sky, GX_MAX_Z24);
    GX_SetViewport(0, 0, rmode->fbWidth, rmode->efbHeight, 0, 1);
    f32 yscale = GX_GetYScaleFactor(rmode->efbHeight, rmode->xfbHeight);
    u32 xfb_height = GX_SetDispCopyYScale(yscale);
    GX_SetScissor(0, 0, rmode->fbWidth, rmode->efbHeight);
    GX_SetDispCopySrc(0, 0, rmode->fbWidth, rmode->efbHeight);
    GX_SetDispCopyDst(rmode->fbWidth, xfb_height);
    GX_SetCopyFilter(rmode->aa, rmode->sample_pattern, GX_TRUE, rmode->vfilter);
    GX_SetFieldMode(rmode->field_rendering, (rmode->viHeight == 2 * rmode->xfbHeight) ? GX_ENABLE : GX_DISABLE);
    GX_SetPixelFmt(rmode->aa ? GX_PF_RGB565_Z16 : GX_PF_RGB8_Z24, GX_ZC_LINEAR);
    GX_SetDispCopyGamma(GX_GM_1_0);
    GX_SetCullMode(GX_CULL_NONE);
    GX_CopyDisp(xfb[cur_fb], GX_TRUE);

    // Vertex format 0: f32 position + direct RGBA8 colour. Colour passes straight through the TEV.
    GX_ClearVtxDesc();
    GX_SetVtxDesc(GX_VA_POS, GX_DIRECT);
    GX_SetVtxDesc(GX_VA_CLR0, GX_DIRECT);
    GX_SetVtxAttrFmt(GX_VTXFMT0, GX_VA_POS, GX_POS_XYZ, GX_F32, 0);
    GX_SetVtxAttrFmt(GX_VTXFMT0, GX_VA_CLR0, GX_CLR_RGBA, GX_RGBA8, 0);
    // Vertex format 1: the same plus an f32 texture coordinate, read through 16-bit indices from
    // FgTexVertex arrays (fg_draw_indexed).
    GX_SetVtxAttrFmt(GX_VTXFMT1, GX_VA_POS, GX_POS_XYZ, GX_F32, 0);
    GX_SetVtxAttrFmt(GX_VTXFMT1, GX_VA_CLR0, GX_CLR_RGBA, GX_RGBA8, 0);
    GX_SetVtxAttrFmt(GX_VTXFMT1, GX_VA_TEX0, GX_TEX_ST, GX_F32, 0);
    GX_SetTexCoordGen(GX_TEXCOORD0, GX_TG_MTX2x4, GX_TG_TEX0, GX_IDENTITY);
    arena = memalign(32, ARENA_SIZE);
    GX_SetNumChans(1);
    GX_SetChanCtrl(GX_COLOR0A0, GX_DISABLE, GX_SRC_REG, GX_SRC_VTX, GX_LIGHTNULL, GX_DF_NONE, GX_AF_NONE);
    GX_SetNumTexGens(0);
    GX_SetNumTevStages(1);
    GX_SetTevOrder(GX_TEVSTAGE0, GX_TEXCOORDNULL, GX_TEXMAP_NULL, GX_COLOR0A0);
    GX_SetTevOp(GX_TEVSTAGE0, GX_PASSCLR);
    GX_SetBlendMode(GX_BM_BLEND, GX_BL_SRCALPHA, GX_BL_INVSRCALPHA, GX_LO_CLEAR);
    GX_SetAlphaUpdate(GX_TRUE);
    GX_SetColorUpdate(GX_TRUE);
}

float fg_frame_dt(void) {
    // viTVMode = (format << 2) | mode; only true PAL is 50 Hz (EURGB60 is a separate format).
    return ((rmode->viTVMode >> 2) == VI_PAL) ? (1.0f / 50.0f) : (1.0f / 60.0f);
}

void fg_read_pad(FgPadRaw *out) {
    memset(out, 0, sizeof(*out));
    u32 connected = PAD_ScanPads();
    WPAD_ScanPads();

    if (connected & 1) {
        out->flags |= FG_FLAG_CONNECTED;
    }
    out->stick_x = PAD_StickX(0);
    out->stick_y = PAD_StickY(0);
    out->cstick_x = PAD_SubStickX(0);
    out->cstick_y = PAD_SubStickY(0);
    out->trig_l = PAD_TriggerL(0);
    out->trig_r = PAD_TriggerR(0);

    u16 held = PAD_ButtonsHeld(0);
    static const struct { u16 pad; u32 fg; } map[] = {
        {PAD_BUTTON_A, FG_BTN_A},         {PAD_BUTTON_B, FG_BTN_B},           {PAD_BUTTON_X, FG_BTN_X},
        {PAD_BUTTON_Y, FG_BTN_Y},         {PAD_TRIGGER_Z, FG_BTN_Z},          {PAD_BUTTON_START, FG_BTN_START},
        {PAD_TRIGGER_L, FG_BTN_L},        {PAD_TRIGGER_R, FG_BTN_R},          {PAD_BUTTON_UP, FG_BTN_DPAD_UP},
        {PAD_BUTTON_DOWN, FG_BTN_DPAD_DOWN}, {PAD_BUTTON_LEFT, FG_BTN_DPAD_LEFT}, {PAD_BUTTON_RIGHT, FG_BTN_DPAD_RIGHT},
    };
    for (unsigned i = 0; i < sizeof(map) / sizeof(map[0]); i++) {
        if (held & map[i].pad) {
            out->buttons |= map[i].fg;
        }
    }

    // Quit: HOME on any Wii Remote, or START + Z on the GameCube pad.
    if ((WPAD_ButtonsDown(0) & WPAD_BUTTON_HOME) ||
        ((held & PAD_BUTTON_START) && (held & PAD_TRIGGER_Z))) {
        out->flags |= FG_FLAG_EXIT;
    }
}

float fg_aspect(void) {
    return aspect;
}

static GXColor unpack(uint32_t rgba) {
    GXColor c = {(u8)(rgba >> 24), (u8)(rgba >> 16), (u8)(rgba >> 8), (u8)rgba};
    return c;
}

void fg_begin_frame(const float eye[3], const float target[3], const float up_dir[3], float fov_y_deg, float near_z,
                    float far_z, uint32_t clear_rgba) {
    // The copy-clear colour applies when this frame is copied out in fg_end_frame.
    GX_SetCopyClear(unpack(clear_rgba), GX_MAX_Z24);
    GX_SetViewport(0, 0, rmode->fbWidth, rmode->efbHeight, 0, 1);

    guVector cam = {eye[0], eye[1], eye[2]};
    guVector look = {target[0], target[1], target[2]};
    guVector up = {up_dir[0], up_dir[1], up_dir[2]};
    guLookAt(view, &cam, &up, &look);
    guPerspective(proj, fov_y_deg, aspect, near_z, far_z);
    GX_LoadProjectionMtx(proj, GX_PERSPECTIVE);
    GX_SetZMode(GX_TRUE, GX_LEQUAL, GX_TRUE);
    in_2d = 0;
    // Last frame finished in fg_end_frame (GX_DrawDone): its vertex copies can be reused.
    arena_used = 0;
    GX_InvalidateTexAll();
}

// Switches the vertex descriptors and TEV between direct colour triangles and indexed meshes.
static void set_mode(int m, int alpha_test) {
    if (m != mode) {
        GX_ClearVtxDesc();
        if (m == MODE_DIRECT) {
            GX_SetVtxDesc(GX_VA_POS, GX_DIRECT);
            GX_SetVtxDesc(GX_VA_CLR0, GX_DIRECT);
        } else {
            GX_SetVtxDesc(GX_VA_POS, GX_INDEX16);
            GX_SetVtxDesc(GX_VA_CLR0, GX_INDEX16);
            if (m == MODE_TEXTURED) {
                GX_SetVtxDesc(GX_VA_TEX0, GX_INDEX16);
            }
        }
        if (m == MODE_TEXTURED) {
            // Texture times vertex colour.
            GX_SetNumTexGens(1);
            GX_SetTevOrder(GX_TEVSTAGE0, GX_TEXCOORD0, GX_TEXMAP0, GX_COLOR0A0);
            GX_SetTevOp(GX_TEVSTAGE0, GX_MODULATE);
        } else {
            GX_SetNumTexGens(0);
            GX_SetTevOrder(GX_TEVSTAGE0, GX_TEXCOORDNULL, GX_TEXMAP_NULL, GX_COLOR0A0);
            GX_SetTevOp(GX_TEVSTAGE0, GX_PASSCLR);
        }
        mode = m;
    }
    alpha_test = alpha_test && m == MODE_TEXTURED;
    if (alpha_test != alpha_test_on) {
        if (alpha_test) {
            // Discard see-through texels, and test depth after texturing so they write no depth.
            GX_SetAlphaCompare(GX_GREATER, 127, GX_AOP_AND, GX_ALWAYS, 0);
            GX_SetZCompLoc(GX_FALSE);
        } else {
            GX_SetAlphaCompare(GX_ALWAYS, 0, GX_AOP_AND, GX_ALWAYS, 0);
            GX_SetZCompLoc(GX_TRUE);
        }
        alpha_test_on = alpha_test;
    }
}

static void emit(const FgVertex *v, uint32_t count) {
    set_mode(MODE_DIRECT, 0);
    GX_Begin(GX_TRIANGLES, GX_VTXFMT0, (u16)count);
    for (uint32_t i = 0; i < count; i++) {
        GX_Position3f32(v[i].pos[0], v[i].pos[1], v[i].pos[2]);
        GX_Color1u32(v[i].rgba);
    }
    GX_End();
}

void fg_draw_tris(const FgVertex *verts, uint32_t count, const float model[3][4]) {
    if (in_2d) {
        GX_LoadProjectionMtx(proj, GX_PERSPECTIVE);
        GX_SetZMode(GX_TRUE, GX_LEQUAL, GX_TRUE);
        in_2d = 0;
    }
    Mtx m, mv;
    memcpy(m, model, sizeof(Mtx));
    guMtxConcat(view, m, mv);
    GX_LoadPosMtxImm(mv, GX_PNMTX0);
    emit(verts, count);
}

// Draws an indexed mesh whose vertex array `buf` the GPU reads (32-byte aligned, flushed, alive until the
// frame is done).
static void draw_indexed(const u8 *buf, const uint16_t *indices, uint32_t index_count, const FgTexture *tex,
                         const float model[3][4]) {
    if (in_2d) {
        GX_LoadProjectionMtx(proj, GX_PERSPECTIVE);
        GX_SetZMode(GX_TRUE, GX_LEQUAL, GX_TRUE);
        in_2d = 0;
    }
    Mtx m, mv;
    memcpy(m, model, sizeof(Mtx));
    guMtxConcat(view, m, mv);
    GX_LoadPosMtxImm(mv, GX_PNMTX0);

    int textured = tex && tex->data;
    if (textured) {
        GXTexObj obj;
        GX_InitTexObj(&obj, (void *)tex->data, tex->width, tex->height, tex->format, GX_REPEAT, GX_REPEAT,
                      GX_FALSE);
        GX_InitTexObjLOD(&obj, GX_LINEAR, GX_LINEAR, 0.0f, 0.0f, 0.0f, GX_FALSE, GX_FALSE, GX_ANISO_1);
        GX_LoadTexObj(&obj, GX_TEXMAP0);
    }
    set_mode(textured ? MODE_TEXTURED : MODE_INDEXED, textured && tex->alpha_test);
    GX_SetArray(GX_VA_POS, (void *)buf, sizeof(FgTexVertex));
    GX_SetArray(GX_VA_CLR0, (void *)(buf + 12), sizeof(FgTexVertex));
    GX_SetArray(GX_VA_TEX0, (void *)(buf + 16), sizeof(FgTexVertex));
    GX_InvVtxCache();

    GX_Begin(GX_TRIANGLES, GX_VTXFMT1, (u16)index_count);
    for (uint32_t i = 0; i < index_count; i++) {
        u16 idx = indices[i];
        GX_Position1x16(idx);
        GX_Color1x16(idx);
        if (textured) {
            GX_TexCoord1x16(idx);
        }
    }
    GX_End();
}

static int bad_counts(uint32_t vert_count, uint32_t index_count) {
    return vert_count == 0 || index_count == 0 || index_count > 0xFFFF || vert_count > 0x10000;
}

void fg_draw_indexed(const FgTexVertex *verts, uint32_t vert_count, const uint16_t *indices,
                     uint32_t index_count, const FgTexture *tex, const float model[3][4]) {
    if (bad_counts(vert_count, index_count)) {
        return;
    }
    // Copy the vertices where the GPU can read them until the frame is done.
    u32 bytes = vert_count * sizeof(FgTexVertex);
    u32 need = (bytes + 31) & ~31u;
    if (!arena || arena_used + need > ARENA_SIZE) {
        return; // over budget this frame: skip rather than overwrite what the GPU still reads
    }
    u8 *buf = arena + arena_used;
    arena_used += need;
    memcpy(buf, verts, bytes);
    DCFlushRange(buf, need);
    draw_indexed(buf, indices, index_count, tex, model);
}

void fg_draw_indexed_static(const FgTexVertex *verts, uint32_t vert_count, const uint16_t *indices,
                            uint32_t index_count, const FgTexture *tex, const float model[3][4]) {
    if (bad_counts(vert_count, index_count) || ((uintptr_t)verts & 31)) {
        return;
    }
    draw_indexed((const u8 *)verts, indices, index_count, tex, model);
}

void fg_flush_range(const void *data, uint32_t size) {
    DCFlushRange((void *)data, size);
}

uint32_t fg_time_us(void) {
    return (uint32_t)ticks_to_microsecs(gettime());
}

void *fg_load_file(const char *path, uint32_t *size) {
    if (!mount()) {
        return NULL;
    }
    FILE *f = fopen(path, "rb");
    if (!f) {
        return NULL;
    }
    void *buf = NULL;
    long n = -1;
    if (fseek(f, 0, SEEK_END) == 0) {
        n = ftell(f);
    }
    if (n > 0 && fseek(f, 0, SEEK_SET) == 0) {
        u32 padded = ((u32)n + 31) & ~31u;
        buf = memalign(32, padded);
        if (buf && fread(buf, 1, (size_t)n, f) == (size_t)n) {
            memset((u8 *)buf + n, 0, padded - (u32)n);
            DCFlushRange(buf, padded);
            *size = (uint32_t)n;
        } else {
            free(buf);
            buf = NULL;
        }
    }
    fclose(f);
    return buf;
}

void fg_free_file(void *data) {
    free(data);
}

int32_t fg_save_file(const char *path, const void *data, uint32_t size) {
    if (!mount()) {
        return -1;
    }
    FILE *f = fopen(path, "wb");
    if (!f) {
        return -1;
    }
    int ok = fwrite(data, 1, size, f) == size;
    ok = (fclose(f) == 0) && ok;
    return ok ? 0 : -1;
}

void fg_draw_2d(const FgVertex *verts, uint32_t count) {
    if (!in_2d) {
        Mtx44 ortho;
        guOrtho(ortho, 0, 479, 0, 639, 0, 10);
        GX_LoadProjectionMtx(ortho, GX_ORTHOGRAPHIC);
        GX_SetZMode(GX_FALSE, GX_ALWAYS, GX_FALSE);
        in_2d = 1;
    }
    // Push the overlay to z = -1, inside the ortho near/far range.
    Mtx m;
    guMtxIdentity(m);
    guMtxTransApply(m, m, 0.0f, 0.0f, -1.0f);
    GX_LoadPosMtxImm(m, GX_PNMTX0);
    emit(verts, count);
}

void fg_end_frame(void) {
    u32 t0 = fg_time_us();
    GX_DrawDone();
    u32 t1 = fg_time_us();
    cur_fb ^= 1;
    GX_SetZMode(GX_TRUE, GX_LEQUAL, GX_TRUE);
    GX_SetColorUpdate(GX_TRUE);
    GX_CopyDisp(xfb[cur_fb], GX_TRUE);
    VIDEO_SetNextFramebuffer(xfb[cur_fb]);
    VIDEO_Flush();
    // A frame that ran past a refresh shows up as more than one retrace since the last one.
    u32 r = retraces;
    if (last_retrace && r - last_retrace > 1) {
        missed += r - last_retrace - 1;
    }
    VIDEO_WaitVSync();
    last_retrace = retraces;
    gpu_wait_us = t1 - t0;
    vsync_wait_us = fg_time_us() - t1;
    last_arena_used = arena_used;
}

void fg_frame_stats(FgFrameStats *out) {
    struct mallinfo mi = mallinfo();
    out->gpu_wait_us = gpu_wait_us;
    out->vsync_wait_us = vsync_wait_us;
    out->missed_vsyncs = missed;
    missed = 0;
    out->arena_used = last_arena_used;
    out->mem1_free = (u32)SYS_GetArena1Hi() - (u32)SYS_GetArena1Lo();
    out->mem2_free = (u32)SYS_GetArena2Hi() - (u32)SYS_GetArena2Lo();
    out->heap_used = (u32)mi.uordblks;
    out->_pad = 0;
}

// The log's background writer: a low-priority thread that writes buffered lines to the card every couple of
// seconds (or sooner when kicked), so the game never waits for the card. It runs while the game waits for the
// next refresh.
static lwp_t writer;
static mutex_t log_mutex;
static cond_t log_cond;
static volatile int writer_on;
static void writer_lock(void) {
    LWP_MutexLock(log_mutex);
}
static void writer_unlock(void) {
    LWP_MutexUnlock(log_mutex);
}
static void writer_kick(void) {
    LWP_CondSignal(log_cond);
}
static void *writer_main(void *arg) {
    (void)arg;
    while (writer_on) {
        struct timespec wait = {LOG_FLUSH_MS / 1000, (LOG_FLUSH_MS % 1000) * 1000000};
        LWP_MutexLock(log_mutex);
        LWP_CondTimedWait(log_cond, log_mutex, &wait);
        LWP_MutexUnlock(log_mutex);
        log_flush();
    }
    return NULL;
}
static void start_writer(void) {
    if (LWP_MutexInit(&log_mutex, false) < 0 || LWP_CondInit(&log_cond) < 0) {
        return;
    }
    writer_on = 1;
    if (LWP_CreateThread(&writer, writer_main, NULL, NULL, 16 * 1024, 20) < 0) {
        writer_on = 0;
        return;
    }
    log_set_writer(writer_lock, writer_unlock, writer_kick);
}
static void stop_writer(void) {
    if (!writer_on) {
        return;
    }
    writer_on = 0;
    writer_kick();
    LWP_JoinThread(writer, NULL);
    log_set_writer(NULL, NULL, NULL);
}

// The background reader (fg_read.h): a low-priority thread doing queued whole-file reads with fg_load_file, so
// streaming a city cell in never holds up a frame. Started by the first fg_read_start (on the game's thread,
// which mounts the storage first), stopped with the log.
static lwp_t reader;
static mutex_t read_mutex;
static cond_t read_cond;
static volatile int reader_on;
static int read_kicked; // under read_mutex: a read was queued (or the reader stopped) since it last looked
static void reader_lock(void) {
    LWP_MutexLock(read_mutex);
}
static void reader_unlock(void) {
    LWP_MutexUnlock(read_mutex);
}
static void reader_kick(void) {
    LWP_MutexLock(read_mutex);
    read_kicked = 1;
    LWP_CondSignal(read_cond);
    LWP_MutexUnlock(read_mutex);
}
static void *reader_main(void *arg) {
    (void)arg;
    while (reader_on) {
        if (read_service()) {
            continue;
        }
        // Nothing queued: sleep until kicked (a kick since the last look means look again at once).
        struct timespec wait = {0, 500 * 1000000};
        LWP_MutexLock(read_mutex);
        if (reader_on && !read_kicked) {
            LWP_CondTimedWait(read_cond, read_mutex, &wait);
        }
        read_kicked = 0;
        LWP_MutexUnlock(read_mutex);
    }
    return NULL;
}
// 1 if the reader is running (or now started).
static int start_reader(void) {
    static int tried;
    if (reader_on || tried) {
        return reader_on;
    }
    tried = 1;
    if (!mount() || LWP_MutexInit(&read_mutex, false) < 0 || LWP_CondInit(&read_cond) < 0) {
        return 0;
    }
    read_setup(fg_load_file, fg_free_file, reader_lock, reader_unlock, reader_kick);
    reader_on = 1;
    // Below the game's thread (64) and above the log writer (20): it reads while the game waits for the refresh.
    if (LWP_CreateThread(&reader, reader_main, NULL, NULL, 16 * 1024, 30) < 0) {
        reader_on = 0;
        read_setup(NULL, NULL, NULL, NULL, NULL);
        log_important("background reader: no thread");
        return 0;
    }
    return 1;
}
static void stop_reader(void) {
    if (!reader_on) {
        return;
    }
    reader_on = 0;
    reader_kick();
    LWP_JoinThread(reader, NULL);
}

int32_t fg_read_start(const char *path) {
    return start_reader() ? read_start(path) : -1;
}

int32_t fg_read_poll(int32_t ticket, void **data, uint32_t *size) {
    return read_poll(ticket, data, size);
}

void fg_read_cancel(int32_t ticket) {
    read_cancel(ticket);
}

// Milliseconds since the log was started (the CPU timer counts from 2000, not from power-on).
static u64 log_epoch;
static uint32_t boot_ms(void) {
    return (uint32_t)(ticks_to_millisecs(gettime() - log_epoch));
}

void fg_log_start(const char *argv0) {
    char dir[128] = "sd:/apps/flatground";
    log_epoch = gettime();
    log_set_clock(boot_ms);
    // The loader passes where boot.dol was started from ("sd:/apps/flatground/boot.dol"): log next to it.
    if (argv0 && strchr(argv0, ':')) {
        const char *slash = strrchr(argv0, '/');
        if (slash && slash > argv0 && (size_t)(slash - argv0) < sizeof(dir)) {
            memcpy(dir, argv0, (size_t)(slash - argv0));
            dir[slash - argv0] = 0;
        }
    }
    int stored = mount() && log_open(dir, time(NULL));
    if (!stored && mount() && strncmp(dir, "usb:", 4) != 0) {
        stored = log_open("usb:/apps/flatground", time(NULL));
    }
    if (stored) {
        start_writer();
    }
    log_important("flatground session: started from %s, log %s", argv0 ? argv0 : "(no path)",
                  stored ? log_path() : "not stored (no writable SD/USB)");
    const char *tv = "?";
    switch (rmode->viTVMode >> 2) {
    case VI_NTSC: tv = "NTSC"; break;
    case VI_PAL: tv = "PAL50"; break;
    case VI_EURGB60: tv = "PAL60"; break;
    case VI_MPAL: tv = "MPAL"; break;
    }
    log_printf("video: %s %s, %ux%u framebuffer, %ux%u EFB, %s, aa %d, %.0f Hz", tv,
               (rmode->viTVMode & VI_NON_INTERLACE) ? "progressive" : "interlaced", rmode->fbWidth,
               rmode->xfbHeight, rmode->fbWidth, rmode->efbHeight, aspect > 1.5f ? "16:9" : "4:3", rmode->aa,
               1.0f / fg_frame_dt());
    log_printf("memory: MEM1 %u KB free, MEM2 %u KB free", ((u32)SYS_GetArena1Hi() - (u32)SYS_GetArena1Lo()) / 1024,
               ((u32)SYS_GetArena2Hi() - (u32)SYS_GetArena2Lo()) / 1024);
}

void fg_log_tick(void) {
    // With the writer thread running this does nothing; without one (it failed to start) the game writes.
    log_tick();
}

void fg_log_stop(void) {
    log_important("session over (quit to the loader)");
    stop_reader();
    stop_writer();
    log_close();
}

void fg_log(const char *line, uint32_t len, int32_t important) {
    log_line(line, len, important);
}

void fg_panic(const char *msg, uint32_t len) {
    log_important("CRASH: %.*s", (int)len, msg);
    char path[224];
    snprintf(path, sizeof(path), "%s", log_path()[0] ? log_path() : "(not stored)");
    stop_writer();
    log_close();
    GX_AbortFrame();
    void *fb = xfb[cur_fb];
    VIDEO_ClearFrameBuffer(rmode, fb, COLOR_BLACK);
    console_init(fb, 20, 20, rmode->fbWidth, rmode->xfbHeight, rmode->fbWidth * VI_DISPLAY_PIX_SZ);
    VIDEO_SetNextFramebuffer(fb);
    VIDEO_Flush();
    printf("\n\n  flatground crashed:\n\n  %.*s\n\n  Last log lines:\n", (int)len, msg);
    const char *tail[LOG_TAIL_LINES];
    int n = log_tail(tail, 10);
    for (int i = 0; i < n; i++) {
        printf("  %.74s\n", tail[i]);
    }
    printf("\n  Log: %s\n  Press HOME or START to return to the loader.\n", path);
    for (;;) {
        VIDEO_WaitVSync();
        PAD_ScanPads();
        WPAD_ScanPads();
        if ((PAD_ButtonsDown(0) & PAD_BUTTON_START) || (WPAD_ButtonsDown(0) & WPAD_BUTTON_HOME)) {
            exit(0);
        }
    }
}

// The prebuilt Rust `core` for powerpc-unknown-linux-gnu still references unwinding symbols, even
// though we build with panic = "abort" and they are never reached. Weak so a real one wins.
__attribute__((weak)) void _Unwind_Resume(void *e) {
    (void)e;
    abort();
}
__attribute__((weak)) int rust_eh_personality(void) {
    abort();
}
