// fg_log: buffered session log (see fg_log.h).

#include <dirent.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "fg_log.h"

#define BUF_SIZE 32768
#define LINE_MAX_ 400 // the stats line runs long

static FILE *s_file;
static char s_path[224];
// Lines waiting to be written, and the copy being written (outside the lock).
static char s_buf[BUF_SIZE];
static int s_len;
static char s_out[BUF_SIZE];
static unsigned s_dropped;
static uint32_t (*s_ms)(void);
static uint32_t s_last_flush;
static char s_tail[LOG_TAIL_LINES][LINE_MAX_];
static int s_tail_n, s_tail_next;
// Set when a background writer owns the file (see log_set_writer).
static void (*s_lock)(void);
static void (*s_unlock)(void);
static void (*s_kick)(void);

void log_set_clock(uint32_t (*ms)(void)) { s_ms = ms; }
const char *log_path(void) { return s_file ? s_path : ""; }

void log_set_writer(void (*lock_fn)(void), void (*unlock_fn)(void), void (*kick_fn)(void)) {
    s_lock = lock_fn;
    s_unlock = unlock_fn;
    s_kick = kick_fn;
}

static void lock(void) {
    if (s_lock) s_lock();
}
static void unlock(void) {
    if (s_unlock) s_unlock();
}

static int cmp_names(const void *a, const void *b) {
    return strcmp(*(const char *const *)a, *(const char *const *)b);
}

// Deletes the oldest session files beyond LOG_KEEP - 1 (the new one makes LOG_KEEP).
static void prune(const char *logdir) {
    DIR *d = opendir(logdir);
    struct dirent *e;
    char *names[256];
    int n = 0, i;
    if (!d) return;
    while ((e = readdir(d)) != NULL && n < 256)
        if (!strncmp(e->d_name, "session_", 8) && strstr(e->d_name, ".txt")) {
            names[n] = malloc(strlen(e->d_name) + 1);
            if (names[n]) strcpy(names[n++], e->d_name);
        }
    closedir(d);
    qsort(names, (size_t)n, sizeof(names[0]), cmp_names);
    for (i = 0; i < n; i++) {
        if (i < n - (LOG_KEEP - 1)) {
            char p[200];
            snprintf(p, sizeof(p), "%s/%s", logdir, names[i]);
            remove(p);
        }
        free(names[i]);
    }
}

int log_open(const char *dir, time_t now) {
    char logdir[160];
    struct tm *t = localtime(&now);
    if (s_file) return 1;
    snprintf(logdir, sizeof(logdir), "%.140s/logs", dir);
    mkdir(logdir, 0777);
    prune(logdir);
    if (t)
        snprintf(s_path, sizeof(s_path), "%s/session_%04d%02d%02d_%02d%02d%02d.txt", logdir, t->tm_year + 1900,
                 t->tm_mon + 1, t->tm_mday, t->tm_hour, t->tm_min, t->tm_sec);
    else
        snprintf(s_path, sizeof(s_path), "%s/session_%lu.txt", logdir, (unsigned long)now);
    s_file = fopen(s_path, "w");
    s_last_flush = s_ms ? s_ms() : 0;
    if (s_file && s_len) log_flush(); // lines logged before the card was ready
    return s_file != NULL;
}

// Appends a finished line; returns 0 if the buffer had no room (the line is still in the tail).
static int append(const char *line, int n) {
    if (s_len + n + 2 > BUF_SIZE) return 0;
    memcpy(s_buf + s_len, line, (size_t)n);
    s_len += n;
    s_buf[s_len++] = '\n';
    return 1;
}

static void add(const char *text, uint32_t len) {
    char line[LINE_MAX_];
    int n, ts, room, busy;
    uint32_t ms = s_ms ? s_ms() : 0;
    ts = snprintf(line, sizeof(line), "[%5u.%03u] ", (unsigned)(ms / 1000), (unsigned)(ms % 1000));
    if (len > (uint32_t)(LINE_MAX_ - ts - 1)) len = (uint32_t)(LINE_MAX_ - ts - 1);
    memcpy(line + ts, text, len);
    n = ts + (int)len;
    line[n] = 0;
    while (n > 0 && (line[n - 1] == '\n' || line[n - 1] == '\r')) line[--n] = 0;
    lock();
    // The in-memory tail, for the crash screen.
    snprintf(s_tail[s_tail_next], LINE_MAX_, "%s", line);
    s_tail_next = (s_tail_next + 1) % LOG_TAIL_LINES;
    if (s_tail_n < LOG_TAIL_LINES) s_tail_n++;
    room = append(line, n);
    if (!room && !s_kick) {
        // No writer thread: write out here and try again.
        unlock();
        log_flush();
        lock();
        room = append(line, n);
    }
    if (!room) s_dropped++;
    busy = s_len > BUF_SIZE * 3 / 4;
    unlock();
    if (busy && s_kick) s_kick();
}

void log_line(const char *text, uint32_t len, int important) {
    add(text, len);
    if (important) {
        if (s_kick)
            s_kick();
        else
            log_flush();
    }
}

static void addv(const char *fmt, va_list ap) {
    char text[LINE_MAX_];
    int n = vsnprintf(text, sizeof(text), fmt, ap);
    if (n < 0) return;
    add(text, n < (int)sizeof(text) ? (uint32_t)n : (uint32_t)sizeof(text) - 1);
}

void log_printf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    addv(fmt, ap);
    va_end(ap);
}

void log_important(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    addv(fmt, ap);
    va_end(ap);
    if (s_kick)
        s_kick();
    else
        log_flush();
}

void log_flush(void) {
    int n;
    unsigned dropped;
    // Take the waiting lines under the lock, write them outside it.
    lock();
    n = s_len;
    if (n) memcpy(s_out, s_buf, (size_t)n);
    s_len = 0;
    dropped = s_dropped;
    s_dropped = 0;
    unlock();
    if (s_file && (n || dropped)) {
        if (dropped) fprintf(s_file, "(%u log lines dropped: the buffer was full)\n", dropped);
        fwrite(s_out, 1, (size_t)n, s_file);
        fflush(s_file);
        // Commit to the card: the SD driver keeps the file's size in a cache until the file is closed or
        // synced, and a crash, power-off or emulator stop never closes it.
        fsync(fileno(s_file));
    }
    s_last_flush = s_ms ? s_ms() : 0;
}

void log_tick(void) {
    uint32_t now;
    if (s_kick) return; // the writer thread does it
    now = s_ms ? s_ms() : 0;
    if (s_len && now - s_last_flush >= LOG_FLUSH_MS) log_flush();
}

void log_close(void) {
    log_flush();
    if (s_file) fclose(s_file);
    s_file = NULL;
}

int log_tail(const char **lines, int n) {
    int i, k = s_tail_n < n ? s_tail_n : n;
    int start = (s_tail_next - k + LOG_TAIL_LINES) % LOG_TAIL_LINES;
    for (i = 0; i < k; i++) lines[i] = s_tail[(start + i) % LOG_TAIL_LINES];
    return k;
}
