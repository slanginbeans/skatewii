// fg_read: whole-file reads done in the background, so loading a city cell never holds up a frame.
//
// The game queues a read (read_start) and gets a ticket, then asks after it each frame (read_poll) until the
// file is there or the read failed. A worker takes queued reads oldest first and does them one at a time with
// the read function it was given (on the Wii, fg_load_file on a low-priority thread that runs while the game
// waits for the next refresh). A read that is no longer wanted is cancelled (read_cancel): dropped if it hasn't
// started, freed when it finishes otherwise. Plain C, so it builds and is tested on the PC too
// (wii/platform/tests/read_test.c).
#ifndef FG_READ_H
#define FG_READ_H

#include <stdint.h>

#define READ_SLOTS 16     // reads queued, in progress or finished but not yet collected
#define READ_PATH_MAX 128 // longest path, with its terminating zero

typedef void *(*read_file_fn)(const char *path, uint32_t *size);
typedef void (*free_file_fn)(void *data);

// read: reads a whole file (NULL: none); free: gives back what read returned. lock/unlock guard the queue
// (NULL: one thread, the tests); kick wakes the worker when a read is queued (may be NULL).
void read_setup(read_file_fn read, free_file_fn free_fn, void (*lock_fn)(void), void (*unlock_fn)(void),
                void (*kick_fn)(void));
// Queues a read of `path`. A ticket (>= 0), or -1 when every slot is taken or the path is too long.
int32_t read_start(const char *path);
// 0: still waiting. 1: done; *data and *size are the file, now the caller's (freed with the free function).
// -1: the read failed (no such file, or no memory for it), or the ticket isn't a read in progress. A ticket is
// finished with after it returned 1 or -1.
int32_t read_poll(int32_t ticket, void **data, uint32_t *size);
// The read isn't wanted any more (unknown tickets are ignored).
void read_cancel(int32_t ticket);
// The worker: does the oldest queued read. 1 if it did one, 0 if none were queued.
int read_service(void);
// Reads queued or in progress.
int read_pending(void);

#endif
