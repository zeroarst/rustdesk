//! A single-frame mailbox between a capture callback and the consumer.
//!
//! The producer (a display-stream callback on its own thread) always
//! stores the *newest* frame, replacing any frame the consumer has not
//! collected yet; the consumer can wait for a frame with a timeout instead
//! of polling. Used by the macOS capturer, but platform independent so it
//! can be tested anywhere.
#![allow(dead_code)]

use std::{
    sync::{Condvar, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

pub struct FrameSlot<T> {
    slot: Mutex<Option<T>>,
    arrived: Condvar,
}

impl<T> Default for FrameSlot<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> FrameSlot<T> {
    pub fn new() -> Self {
        Self {
            slot: Mutex::new(None),
            arrived: Condvar::new(),
        }
    }

    /// Store the newest frame, dropping any frame not collected yet, and
    /// wake a waiting `take`.
    pub fn put(&self, frame: T) {
        *self.lock() = Some(frame);
        self.arrived.notify_one();
    }

    /// Take the stored frame, waiting up to `timeout` for one to arrive.
    /// Returns `None` if the timeout passes with nothing to take.
    pub fn take(&self, timeout: Duration) -> Option<T> {
        let mut guard = self.lock();
        if guard.is_none() && !timeout.is_zero() {
            let deadline = Instant::now() + timeout;
            while guard.is_none() {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                guard = self
                    .arrived
                    .wait_timeout(guard, deadline - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }
        guard.take()
    }

    fn lock(&self) -> MutexGuard<'_, Option<T>> {
        // A panic while holding the lock cannot leave the Option in a bad
        // state, so a poisoned mutex is still safe to use.
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::FrameSlot;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn take_returns_frame_put_before() {
        let slot = FrameSlot::new();
        slot.put(1u32);
        assert_eq!(slot.take(Duration::ZERO), Some(1));
    }

    #[test]
    fn take_on_empty_slot_times_out_with_none() {
        let slot: FrameSlot<u32> = FrameSlot::new();
        let t = Instant::now();
        assert_eq!(slot.take(Duration::from_millis(30)), None);
        let waited = t.elapsed();
        assert!(waited >= Duration::from_millis(30), "returned early: {:?}", waited);
        assert!(waited < Duration::from_millis(500), "waited far too long: {:?}", waited);
    }

    #[test]
    fn zero_timeout_on_empty_slot_returns_immediately() {
        let slot: FrameSlot<u32> = FrameSlot::new();
        let t = Instant::now();
        assert_eq!(slot.take(Duration::ZERO), None);
        assert!(t.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn put_replaces_an_unconsumed_frame_with_the_newer_one() {
        let slot = FrameSlot::new();
        slot.put(1u32);
        slot.put(2u32);
        assert_eq!(slot.take(Duration::ZERO), Some(2));
        assert_eq!(slot.take(Duration::ZERO), None, "slot must be empty after take");
    }

    #[test]
    fn waiting_take_wakes_as_soon_as_a_frame_arrives() {
        let slot = std::sync::Arc::new(FrameSlot::new());
        let producer = slot.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            producer.put(7u32);
        });
        let t = Instant::now();
        assert_eq!(slot.take(Duration::from_secs(2)), Some(7));
        let waited = t.elapsed();
        assert!(waited >= Duration::from_millis(15), "took before the frame existed: {:?}", waited);
        assert!(waited < Duration::from_millis(500), "woke by timeout, not by the frame: {:?}", waited);
    }
}
