// Session log on the PC: one file per session in logs/, only the newest kept, buffered lines reach the file on
// flush, important ones at once, the tail is kept for the crash screen.
//   cc -O1 -Wall -Wextra -Iwii/platform wii/platform/fg_log.c wii/platform/tests/log_test.c -o log_test && ./log_test
#include <dirent.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "fg_log.h"

static int ok = 1;
#define CHECK(c, what)                                         \
    do {                                                       \
        int c_ = !!(c);                                        \
        printf("%-56s %s\n", what, c_ ? "ok" : "FAIL");        \
        ok &= c_;                                              \
    } while (0)
static uint32_t clock_ms;
static uint32_t ms(void) { return clock_ms; }

static int count_logs(const char *dir) {
    DIR *d = opendir(dir);
    struct dirent *e;
    int n = 0;
    if (!d) return 0;
    while ((e = readdir(d)) != NULL) n += !strncmp(e->d_name, "session_", 8);
    closedir(d);
    return n;
}

static int read_log(char *buf, int cap) {
    FILE *f = fopen(log_path(), "r");
    int n = f ? (int)fread(buf, 1, (size_t)cap - 1, f) : 0;
    if (f) fclose(f);
    buf[n > 0 ? n : 0] = 0;
    return n;
}

int main(void) {
    char dir[] = "/tmp/flatground_logtest", logdir[64], cmd[96], buf[8192];
    const char *tail[LOG_TAIL_LINES];
    int i, n;
    snprintf(cmd, sizeof(cmd), "rm -rf %s && mkdir -p %s", dir, dir);
    if (system(cmd)) return 1;
    snprintf(logdir, sizeof(logdir), "%s/logs", dir);
    log_set_clock(ms);

    // 12 sessions, a second apart: only the newest LOG_KEEP stay.
    for (i = 0; i < 12; i++) {
        int opened = log_open(dir, (time_t)1700000000 + i);
        if (i == 0) CHECK(opened, "log file created in logs/");
        log_printf("session %d", i);
        log_close();
    }
    CHECK(count_logs(logdir) == LOG_KEEP, "only the newest sessions are kept");

    log_open(dir, (time_t)1800000000);
    clock_ms = 1234;
    log_printf("hello %d", 42);
    CHECK(read_log(buf, sizeof(buf)) == 0, "lines wait in memory (no SD write every frame)");
    clock_ms = 1234 + LOG_FLUSH_MS;
    log_tick();
    read_log(buf, sizeof(buf));
    CHECK(strstr(buf, "[    1.234] hello 42") != NULL, "... and are written out a moment later, timestamped");
    log_line("BAIL: landed sideways\n", 22, 1);
    read_log(buf, sizeof(buf));
    CHECK(strstr(buf, "BAIL: landed sideways\n") != NULL, "important lines are written at once");
    for (i = 0; i < 40; i++) log_printf("line %d", i);
    n = log_tail(tail, LOG_TAIL_LINES);
    CHECK(n == LOG_TAIL_LINES && strstr(tail[n - 1], "line 39") && strstr(tail[0], "line 24"),
          "the last lines are kept for the crash screen");
    // A line longer than the buffer line is cut, not overflowed.
    {
        char big[1000];
        memset(big, 'x', sizeof(big));
        log_line(big, sizeof(big), 0);
        n = log_tail(tail, 1);
        CHECK(n == 1 && strlen(tail[0]) < 400, "over-long lines are cut");
    }
    log_close();
    snprintf(cmd, sizeof(cmd), "rm -rf %s", dir);
    if (system(cmd)) return 1;
    return !ok;
}
