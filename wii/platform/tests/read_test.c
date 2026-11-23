// Background reads on the PC: queued reads are done oldest first, collected once, failures reported, cancelled
// reads freed whether they had started or not, old tickets never match new reads.
//   cc -O1 -Wall -Wextra -Iwii/platform wii/platform/fg_read.c wii/platform/tests/read_test.c -o read_test && ./read_test
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "fg_read.h"

static int ok = 1;
#define CHECK(c, what)                                         \
    do {                                                       \
        int c_ = !!(c);                                        \
        printf("%-56s %s\n", what, c_ ? "ok" : "FAIL");        \
        ok &= c_;                                              \
    } while (0)

static char last_read[READ_PATH_MAX];
static int reads, frees, kicks;
static int32_t cancel_during_read = -1;

// A "file" holds its own path; paths starting with "missing" aren't there.
static void *fake_read(const char *path, uint32_t *size) {
    reads++;
    snprintf(last_read, sizeof(last_read), "%s", path);
    if (cancel_during_read >= 0) {
        read_cancel(cancel_during_read);
        cancel_during_read = -1;
    }
    if (!strncmp(path, "missing", 7)) {
        return NULL;
    }
    *size = (uint32_t)strlen(path);
    char *d = malloc(*size + 1);
    memcpy(d, path, *size + 1);
    return d;
}
static void fake_free(void *d) {
    frees++;
    free(d);
}
static void fake_kick(void) { kicks++; }

int main(void) {
    void *data = NULL;
    uint32_t size = 0;

    CHECK(read_start("a") == -1, "nothing queued before setup");
    read_setup(fake_read, fake_free, NULL, NULL, fake_kick);

    int32_t a = read_start("sd:/a.fgpak");
    int32_t b = read_start("missing.fgpak");
    int32_t c = read_start("sd:/c.fgpak");
    CHECK(a >= 0 && b >= 0 && c >= 0 && a != b && b != c, "three tickets");
    CHECK(kicks == 3, "each read wakes the worker");
    CHECK(read_pending() == 3, "three pending");
    CHECK(read_poll(a, &data, &size) == 0, "waiting before the worker runs");

    CHECK(read_service() == 1 && !strcmp(last_read, "sd:/a.fgpak"), "oldest first");
    CHECK(read_poll(a, &data, &size) == 1 && size == 11 && !memcmp(data, "sd:/a.fgpak", 11), "collected");
    fake_free(data);
    CHECK(read_poll(a, &data, &size) == -1, "collected only once");

    CHECK(read_service() == 1 && read_poll(b, &data, &size) == -1, "a missing file fails");
    CHECK(read_poll(b, &data, &size) == -1, "and stays failed");

    // c is queued: cancelling drops it without reading.
    int before = reads;
    read_cancel(c);
    CHECK(read_service() == 0 && reads == before, "a cancelled queued read is never done");
    CHECK(read_pending() == 0, "nothing pending");

    // Cancelled while it's being read: freed when the read ends, and the ticket is gone.
    int32_t d = read_start("sd:/d.fgpak");
    cancel_during_read = d;
    int frees_before = frees;
    CHECK(read_service() == 1 && frees == frees_before + 1, "cancelled mid-read: freed when it ends");
    CHECK(read_poll(d, &data, &size) == -1, "and never handed out");

    // Finished but cancelled before collecting: freed at once.
    int32_t e = read_start("sd:/e.fgpak");
    read_service();
    frees_before = frees;
    read_cancel(e);
    CHECK(frees == frees_before + 1 && read_poll(e, &data, &size) == -1, "cancelled after it finished: freed");

    // Slots run out, then come back; an old ticket doesn't match the slot's next read.
    int32_t t[READ_SLOTS];
    int all = 1;
    for (int i = 0; i < READ_SLOTS; i++) {
        char p[32];
        snprintf(p, sizeof(p), "sd:/f%d", i);
        t[i] = read_start(p);
        all &= t[i] >= 0;
    }
    CHECK(all && read_start("sd:/one_too_many") == -1, "full queue refuses");
    read_cancel(t[0]);
    int32_t again = read_start("sd:/again");
    CHECK(again >= 0 && again != t[0], "a freed slot is reused with a new ticket");
    CHECK(read_poll(t[0], &data, &size) == -1, "the old ticket doesn't see the new read");
    while (read_service()) {
    }
    int collected = 0;
    for (int i = 1; i < READ_SLOTS; i++) {
        if (read_poll(t[i], &data, &size) == 1) {
            collected++;
            fake_free(data);
        }
    }
    CHECK(collected == READ_SLOTS - 1, "the rest all arrive");
    CHECK(read_poll(again, &data, &size) == 1 && !strcmp(last_read, "sd:/again"), "the newest read last");
    fake_free(data);

    char longp[READ_PATH_MAX + 1];
    memset(longp, 'x', READ_PATH_MAX);
    longp[READ_PATH_MAX] = 0;
    CHECK(read_start(longp) == -1, "too long a path refused");
    CHECK(read_poll(-5, &data, &size) == -1, "a bad ticket is no read");
    read_cancel(-5);

    printf("%s\n", ok ? "all ok" : "FAILED");
    return ok ? 0 : 1;
}
