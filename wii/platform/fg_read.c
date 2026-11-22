// fg_read: background whole-file reads (see fg_read.h).
#include "fg_read.h"

#include <stddef.h>
#include <string.h>

enum { FREE, QUEUED, READING, DONE, FAILED };

typedef struct {
    int state;
    int cancelled; // READING, and nobody wants it: the worker frees it when the read ends
    uint32_t gen;  // bumped each time the slot is taken, so an old ticket never matches a new read
    uint32_t seq;  // queue order
    void *data;
    uint32_t size;
    char path[READ_PATH_MAX];
} Slot;

static Slot slots[READ_SLOTS];
static uint32_t next_seq;
static read_file_fn read_fn;
static free_file_fn free_fn;
static void (*lock)(void);
static void (*unlock)(void);
static void (*kick)(void);

static void lock_q(void) {
    if (lock) {
        lock();
    }
}
static void unlock_q(void) {
    if (unlock) {
        unlock();
    }
}

// Tickets: generation * READ_SLOTS + slot, kept positive.
static int32_t ticket_of(int i) {
    return (int32_t)((slots[i].gen & 0x7ffffu) * READ_SLOTS + (uint32_t)i);
}
// The slot a ticket names, or -1.
static int slot_of(int32_t ticket) {
    if (ticket < 0) {
        return -1;
    }
    int i = (int)(ticket % READ_SLOTS);
    if (slots[i].state == FREE || slots[i].cancelled || ticket_of(i) != ticket) {
        return -1;
    }
    return i;
}

void read_setup(read_file_fn read, free_file_fn free_file, void (*lock_fn)(void), void (*unlock_fn)(void),
                void (*kick_fn)(void)) {
    read_fn = read;
    free_fn = free_file;
    lock = lock_fn;
    unlock = unlock_fn;
    kick = kick_fn;
}

int32_t read_start(const char *path) {
    size_t n = strlen(path);
    if (!read_fn || n >= READ_PATH_MAX) {
        return -1;
    }
    int32_t ticket = -1;
    lock_q();
    for (int i = 0; i < READ_SLOTS; i++) {
        if (slots[i].state == FREE) {
            Slot *s = &slots[i];
            s->state = QUEUED;
            s->cancelled = 0;
            s->gen++;
            s->seq = next_seq++;
            s->data = NULL;
            s->size = 0;
            memcpy(s->path, path, n + 1);
            ticket = ticket_of(i);
            break;
        }
    }
    unlock_q();
    if (ticket >= 0 && kick) {
        kick();
    }
    return ticket;
}

int32_t read_poll(int32_t ticket, void **data, uint32_t *size) {
    int32_t r = -1;
    lock_q();
    int i = slot_of(ticket);
    if (i >= 0) {
        Slot *s = &slots[i];
        if (s->state == QUEUED || s->state == READING) {
            r = 0;
        } else if (s->state == DONE) {
            *data = s->data;
            *size = s->size;
            s->data = NULL;
            s->state = FREE;
            r = 1;
        } else {
            s->state = FREE;
        }
    }
    unlock_q();
    return r;
}

void read_cancel(int32_t ticket) {
    void *drop = NULL;
    lock_q();
    int i = slot_of(ticket);
    if (i >= 0) {
        Slot *s = &slots[i];
        if (s->state == READING) {
            s->cancelled = 1;
        } else {
            drop = s->data;
            s->data = NULL;
            s->state = FREE;
        }
    }
    unlock_q();
    if (drop && free_fn) {
        free_fn(drop);
    }
}

int read_service(void) {
    char path[READ_PATH_MAX];
    int pick = -1;
    lock_q();
    for (int i = 0; i < READ_SLOTS; i++) {
        // Oldest first (wrap-around safe).
        if (slots[i].state == QUEUED && (pick < 0 || (int32_t)(slots[i].seq - slots[pick].seq) < 0)) {
            pick = i;
        }
    }
    if (pick >= 0) {
        slots[pick].state = READING;
        memcpy(path, slots[pick].path, READ_PATH_MAX);
    }
    unlock_q();
    if (pick < 0) {
        return 0;
    }

    uint32_t size = 0;
    void *data = read_fn(path, &size);

    void *drop = NULL;
    lock_q();
    Slot *s = &slots[pick];
    if (s->cancelled) {
        drop = data;
        s->cancelled = 0;
        s->state = FREE;
    } else {
        s->data = data;
        s->size = data ? size : 0;
        s->state = data ? DONE : FAILED;
    }
    unlock_q();
    if (drop && free_fn) {
        free_fn(drop);
    }
    return 1;
}

int read_pending(void) {
    int n = 0;
    lock_q();
    for (int i = 0; i < READ_SLOTS; i++) {
        n += slots[i].state == QUEUED || slots[i].state == READING;
    }
    unlock_q();
    return n;
}
