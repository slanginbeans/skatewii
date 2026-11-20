// fg_log: the session log. One text file per play session in <app folder>/logs/session_YYYYMMDD_HHMMSS.txt,
// the newest LOG_KEEP kept.
//
// Lines are buffered in memory and written out every LOG_FLUSH_MS (or at once for important ones), so writing to
// the SD card never stalls a frame. The last lines are also kept for the crash screen. Plain C + stdio, so it
// builds and is tested on the PC too (wii/platform/tests/log_test.c).
//
// Based on the session log of the author's earlier Wii project.
#ifndef FG_LOG_H
#define FG_LOG_H

#include <stdint.h>
#include <time.h>

#define LOG_KEEP 10       // session files kept
#define LOG_TAIL_LINES 16 // lines kept for the crash screen
#define LOG_FLUSH_MS 2000 // buffered lines reach the card at least this often

// dir: the app folder ("sd:/apps/flatground"); now: wall-clock time for the file name. Returns 0 if the folder
// can't be written (the log then only keeps the in-memory tail).
int log_open(const char *dir, time_t now);
// Milliseconds since boot, for the timestamps.
void log_set_clock(uint32_t (*ms)(void));
// One line (a trailing newline is dropped). `important`: written out at once (errors, crashes).
void log_line(const char *text, uint32_t len, int important);
void log_printf(const char *fmt, ...)
#ifdef __GNUC__
    __attribute__((format(printf, 1, 2)))
#endif
    ;
void log_important(const char *fmt, ...)
#ifdef __GNUC__
    __attribute__((format(printf, 1, 2)))
#endif
    ;
void log_flush(void);
// Call every frame: flushes now and then.
void log_tick(void);
void log_close(void);
// The open file's path ("" when none).
const char *log_path(void);
// The last n lines kept in memory, oldest first; returns how many.
int log_tail(const char **lines, int n);

#endif
