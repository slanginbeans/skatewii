//! Whole-file reads that finish later, for streaming a district's cells in while the game runs.
//!
//! The game starts a read and gets a [`Ticket`], then polls it once a frame until the file is there. On the Wii
//! the reads are done by a background thread (`wii/platform/fg_read.c`); [`MemReader`] serves files from memory
//! for the PC and the tests, one read each time it's [`MemReader::service`]d, so a test decides when reads end.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

/// A read in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticket(pub i32);

#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    /// Still reading.
    Waiting,
    /// The whole file, now the caller's: kept until given back with [`Reader::free`].
    Done(&'static [u8]),
    /// No such file, no memory for it, or not a read in progress.
    Failed,
}

pub trait Reader {
    /// Queues a read of `path`. None when no more reads can be queued (try again later).
    fn start(&mut self, path: &str) -> Option<Ticket>;
    /// After [`Poll::Done`] or [`Poll::Failed`] the ticket is finished with.
    fn poll(&mut self, ticket: Ticket) -> Poll;
    /// The read isn't wanted any more; whatever it reads is dropped.
    fn cancel(&mut self, ticket: Ticket);
    /// Gives back a file from [`Poll::Done`].
    ///
    /// # Safety
    /// `data` came from this reader's [`Poll::Done`], nothing refers to it any more, and no draw still queued
    /// for the GPU reads it.
    unsafe fn free(&mut self, data: &'static [u8]);
}

enum State {
    Queued(String),
    Done(&'static [u8]),
    Failed,
}

/// Files in memory, read one per [`MemReader::service`], oldest first.
#[derive(Default)]
pub struct MemReader {
    files: BTreeMap<String, Vec<u8>>,
    reads: BTreeMap<i32, State>,
    next: i32,
    /// Reads that may be queued at once (0: no limit).
    pub slots: usize,
    /// Files handed out and not yet freed.
    pub live: usize,
    /// Reads done so far (cancelled ones that had started included).
    pub done: usize,
}

impl MemReader {
    pub fn new() -> MemReader {
        MemReader::default()
    }

    pub fn add(&mut self, path: &str, data: Vec<u8>) {
        self.files.insert(path.into(), data);
    }

    /// Does the oldest queued read. False when none was queued.
    pub fn service(&mut self) -> bool {
        let Some((&t, path)) = self.reads.iter().find_map(|(t, s)| match s {
            State::Queued(p) => Some((t, p.clone())),
            _ => None,
        }) else {
            return false;
        };
        self.done += 1;
        let state = match self.files.get(&path) {
            Some(d) => {
                self.live += 1;
                State::Done(Box::leak(d.clone().into_boxed_slice()))
            }
            None => State::Failed,
        };
        self.reads.insert(t, state);
        true
    }

    /// Reads queued and not yet done.
    pub fn queued(&self) -> usize {
        self.reads
            .values()
            .filter(|s| matches!(s, State::Queued(_)))
            .count()
    }
}

impl Reader for MemReader {
    fn start(&mut self, path: &str) -> Option<Ticket> {
        if self.slots != 0 && self.reads.len() >= self.slots {
            return None;
        }
        let t = self.next;
        self.next += 1;
        self.reads.insert(t, State::Queued(path.into()));
        Some(Ticket(t))
    }

    fn poll(&mut self, ticket: Ticket) -> Poll {
        match self.reads.get(&ticket.0) {
            None => Poll::Failed,
            Some(State::Queued(_)) => Poll::Waiting,
            Some(_) => match self.reads.remove(&ticket.0) {
                Some(State::Done(d)) => Poll::Done(d),
                _ => Poll::Failed,
            },
        }
    }

    fn cancel(&mut self, ticket: Ticket) {
        if let Some(State::Done(d)) = self.reads.remove(&ticket.0) {
            unsafe { self.free(d) };
        }
    }

    unsafe fn free(&mut self, data: &'static [u8]) {
        self.live -= 1;
        // Made by Box::leak in `service`.
        drop(unsafe { Box::from_raw(data as *const [u8] as *mut [u8]) });
    }
}

/// A reader shared with whoever drives it (a test servicing a [`MemReader`] the game owns too).
impl<R: Reader> Reader for Rc<RefCell<R>> {
    fn start(&mut self, path: &str) -> Option<Ticket> {
        self.borrow_mut().start(path)
    }
    fn poll(&mut self, ticket: Ticket) -> Poll {
        self.borrow_mut().poll(ticket)
    }
    fn cancel(&mut self, ticket: Ticket) {
        self.borrow_mut().cancel(ticket)
    }
    unsafe fn free(&mut self, data: &'static [u8]) {
        unsafe { self.borrow_mut().free(data) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn reads_end_when_serviced_oldest_first() {
        let mut r = MemReader::new();
        r.add("a", vec![1, 2, 3]);
        r.add("b", vec![4]);
        r.slots = 2;
        let a = r.start("a").unwrap();
        let b = r.start("b").unwrap();
        assert_eq!(r.start("c"), None, "full");
        assert_eq!(r.poll(a), Poll::Waiting);
        assert!(r.service());
        assert_eq!(r.poll(b), Poll::Waiting, "a first");
        let Poll::Done(d) = r.poll(a) else {
            panic!("a read")
        };
        assert_eq!(d, &[1, 2, 3]);
        assert_eq!(r.poll(a), Poll::Failed, "finished with");
        unsafe { r.free(d) };
        let c = r.start("missing").unwrap();
        r.cancel(b);
        assert_eq!(r.queued(), 1);
        assert!(r.service());
        assert!(!r.service());
        assert_eq!(r.poll(c), Poll::Failed);
        assert_eq!(r.live, 0);
    }

    #[test]
    fn a_cancelled_read_that_finished_is_freed() {
        let mut r = MemReader::new();
        r.add("a", vec![1]);
        let a = r.start("a").unwrap();
        r.service();
        assert_eq!(r.live, 1);
        r.cancel(a);
        assert_eq!(r.live, 0);
        assert_eq!(r.poll(a), Poll::Failed);
    }
}
