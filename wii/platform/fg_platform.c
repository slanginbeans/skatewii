// flatground platform layer over libogc: video + GX setup, a vertex-colour 3D pass, a 2D overlay
// pass, GameCube pad + Wii Remote HOME input, and a panic screen.

#include <gccore.h>
#include <wiiuse/wpad.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <malloc.h>

#include "fg_platform.h"

#define FIFO_SIZE (256 * 1024)

static GXRModeObj *rmode;
static void *xfb[2];
static int cur_fb;
static float aspect = 4.0f / 3.0f;
static Mtx view;
static Mtx44 proj;
static int in_2d;

void fg_platform_init(void) {
    VIDEO_Init();
    PAD_Init();
    WPAD_Init();

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

static GXColor unpack(uint32_t rgba) {
    GXColor c = {(u8)(rgba >> 24), (u8)(rgba >> 16), (u8)(rgba >> 8), (u8)rgba};
    return c;
}

void fg_begin_frame(const float eye[3], const float target[3], float fov_y_deg, float near_z, float far_z,
                    uint32_t clear_rgba) {
    // The copy-clear colour applies when this frame is copied out in fg_end_frame.
    GX_SetCopyClear(unpack(clear_rgba), GX_MAX_Z24);
    GX_SetViewport(0, 0, rmode->fbWidth, rmode->efbHeight, 0, 1);

    guVector cam = {eye[0], eye[1], eye[2]};
    guVector look = {target[0], target[1], target[2]};
    guVector up = {0.0f, 1.0f, 0.0f};
    guLookAt(view, &cam, &up, &look);
    guPerspective(proj, fov_y_deg, aspect, near_z, far_z);
    GX_LoadProjectionMtx(proj, GX_PERSPECTIVE);
    GX_SetZMode(GX_TRUE, GX_LEQUAL, GX_TRUE);
    in_2d = 0;
}

static void emit(const FgVertex *v, uint32_t count) {
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
    GX_DrawDone();
    cur_fb ^= 1;
    GX_SetZMode(GX_TRUE, GX_LEQUAL, GX_TRUE);
    GX_SetColorUpdate(GX_TRUE);
    GX_CopyDisp(xfb[cur_fb], GX_TRUE);
    VIDEO_SetNextFramebuffer(xfb[cur_fb]);
    VIDEO_Flush();
    VIDEO_WaitVSync();
}

void fg_panic(const char *msg, uint32_t len) {
    GX_AbortFrame();
    void *fb = xfb[cur_fb];
    VIDEO_ClearFrameBuffer(rmode, fb, COLOR_BLACK);
    console_init(fb, 20, 20, rmode->fbWidth, rmode->xfbHeight, rmode->fbWidth * VI_DISPLAY_PIX_SZ);
    VIDEO_SetNextFramebuffer(fb);
    VIDEO_Flush();
    printf("\n\n  flatground crashed:\n\n  %.*s\n\n  Press HOME or START to return to the loader.\n",
           (int)len, msg);
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
