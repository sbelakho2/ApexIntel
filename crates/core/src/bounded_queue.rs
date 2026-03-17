//! Bounded queue utilities to prevent unbounded memory growth.
//!
//! This module provides bounded queue abstractions with configurable backpressure
//! strategies. All queues enforce strict capacity limits to prevent memory exhaustion.
//!
//! # B296: Backpressure to queues
//!
//! Production systems must never allow unbounded queue growth. This module ensures:
//! - All queues have explicit capacity limits
//! - Push operations fail gracefully when full
//! - Multiple backpressure strategies (drop-oldest, drop-newest, block)
//! - Metrics for queue depth and overflow events

use std::collections::VecDeque;
use tracing::warn;

/// Maximum allowed capacity for any bounded queue to prevent misconfiguration.
pub const MAX_QUEUE_CAPACITY: usize = 100_000;

/// Backpressure strategy when queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressureStrategy {
    /// Drop the oldest item to make room (circular buffer).
    DropOldest,
    /// Drop the newest item (reject incoming).
    DropNewest,
    /// Return error without modifying queue.
    Reject,
}

/// A bounded queue with configurable backpressure.
///
/// # Safety
///
/// This queue enforces strict capacity bounds. `push` operations when full
/// will apply the configured backpressure strategy rather than growing unbounded.
///
/// # Examples
///
/// ```
/// use apex_core::bounded_queue::{BoundedQueue, BackpressureStrategy};
///
/// let mut queue = BoundedQueue::new(3, BackpressureStrategy::DropOldest);
/// assert!(queue.push(1).is_ok());
/// assert!(queue.push(2).is_ok());
/// assert!(queue.push(3).is_ok());
/// // Queue is full, DropOldest will remove 1
/// assert!(queue.push(4).is_ok());
/// assert_eq!(queue.len(), 3);
/// assert_eq!(queue.pop(), Some(2)); // 1 was dropped
/// ```
#[derive(Debug, Clone)]
pub struct BoundedQueue<T> {
    inner: VecDeque<T>,
    capacity: usize,
    strategy: BackpressureStrategy,
    drop_count: u64,
}

impl<T> BoundedQueue<T> {
    /// Create a new bounded queue with the given capacity and backpressure strategy.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is 0 or exceeds `MAX_QUEUE_CAPACITY`.
    pub fn new(capacity: usize, strategy: BackpressureStrategy) -> Self {
        assert!(capacity > 0, "Queue capacity must be > 0");
        assert!(
            capacity <= MAX_QUEUE_CAPACITY,
            "Queue capacity {} exceeds maximum {}",
            capacity,
            MAX_QUEUE_CAPACITY
        );
        Self {
            inner: VecDeque::with_capacity(capacity),
            capacity,
            strategy,
            drop_count: 0,
        }
    }

    /// Push an item onto the queue, applying backpressure if full.
    ///
    /// Returns `Ok(())` if the item was enqueued (possibly after dropping another),
    /// or `Err(item)` if the backpressure strategy is `Reject` or `DropNewest` and queue is full.
    pub fn push(&mut self, item: T) -> Result<(), T> {
        if self.inner.len() < self.capacity {
            self.inner.push_back(item);
            return Ok(());
        }

        // Queue is full, apply backpressure
        match self.strategy {
            BackpressureStrategy::DropOldest => {
                self.inner.pop_front();
                self.inner.push_back(item);
                self.drop_count += 1;
                if self.drop_count % 100 == 1 {
                    // Log every 100th drop to avoid log spam
                    warn!(
                        capacity = self.capacity,
                        drop_count = self.drop_count,
                        "BoundedQueue: dropping oldest items due to capacity"
                    );
                }
                Ok(())
            }
            BackpressureStrategy::DropNewest => {
                // DropNewest rejects the incoming item (the "newest")
                // We still count this as a drop for consistent metrics
                self.drop_count += 1;
                if self.drop_count % 100 == 1 {
                    warn!(
                        capacity = self.capacity,
                        drop_count = self.drop_count,
                        "BoundedQueue: rejecting newest items due to capacity"
                    );
                }
                Err(item)
            }
            BackpressureStrategy::Reject => {
                // Reject strategy: return error without counting as a "drop"
                // This distinguishes intentional backpressure (DropOldest/DropNewest)
                // from simple capacity rejection in metrics
                Err(item)
            }
        }
    }

    /// Pop the oldest item from the queue.
    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop_front()
    }

    /// Get the number of items currently in the queue.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Check if the queue is at capacity.
    pub fn is_full(&self) -> bool {
        self.inner.len() >= self.capacity
    }

    /// Get the configured capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the total number of items dropped due to backpressure.
    pub fn drop_count(&self) -> u64 {
        self.drop_count
    }

    /// Clear all items from the queue.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Peek at the oldest item without removing it.
    pub fn peek(&self) -> Option<&T> {
        self.inner.front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounded_queue_basic_operations() {
        let mut q = BoundedQueue::new(3, BackpressureStrategy::Reject);
        assert!(q.is_empty());
        assert!(!q.is_full());
        assert_eq!(q.len(), 0);

        assert!(q.push(1).is_ok());
        assert!(q.push(2).is_ok());
        assert_eq!(q.len(), 2);
        assert_eq!(q.peek(), Some(&1));

        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), None);
        assert!(q.is_empty());
    }

    #[test]
    fn test_drop_oldest_strategy() {
        let mut q = BoundedQueue::new(3, BackpressureStrategy::DropOldest);
        assert!(q.push(1).is_ok());
        assert!(q.push(2).is_ok());
        assert!(q.push(3).is_ok());
        assert!(q.is_full());

        // Pushing 4th item should drop the oldest (1)
        assert!(q.push(4).is_ok());
        assert_eq!(q.len(), 3);
        assert_eq!(q.drop_count(), 1);
        assert_eq!(q.pop(), Some(2)); // 1 was dropped
        assert_eq!(q.pop(), Some(3));
        assert_eq!(q.pop(), Some(4));
    }

    #[test]
    fn test_drop_newest_strategy() {
        let mut q = BoundedQueue::new(3, BackpressureStrategy::DropNewest);
        assert!(q.push(1).is_ok());
        assert!(q.push(2).is_ok());
        assert!(q.push(3).is_ok());

        // Pushing 4th item should be rejected
        assert!(q.push(4).is_err());
        assert_eq!(q.len(), 3);
        assert_eq!(q.drop_count(), 1);
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), Some(3));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn test_reject_strategy() {
        let mut q = BoundedQueue::new(2, BackpressureStrategy::Reject);
        assert!(q.push(1).is_ok());
        assert!(q.push(2).is_ok());

        let result = q.push(3);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), 3); // Item returned
        assert_eq!(q.len(), 2);
        assert_eq!(q.drop_count(), 0); // Reject doesn't count as drop
    }

    #[test]
    fn test_clear() {
        let mut q = BoundedQueue::new(5, BackpressureStrategy::Reject);
        q.push(1).unwrap();
        q.push(2).unwrap();
        q.push(3).unwrap();
        assert_eq!(q.len(), 3);

        q.clear();
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
    }

    #[test]
    fn test_capacity_enforcement() {
        let q = BoundedQueue::<i32>::new(100, BackpressureStrategy::Reject);
        assert_eq!(q.capacity(), 100);
        assert!(!q.is_full());
    }

    #[test]
    #[should_panic(expected = "Queue capacity must be > 0")]
    fn test_zero_capacity_panics() {
        BoundedQueue::<i32>::new(0, BackpressureStrategy::Reject);
    }

    #[test]
    #[should_panic(expected = "exceeds maximum")]
    fn test_excessive_capacity_panics() {
        BoundedQueue::<i32>::new(MAX_QUEUE_CAPACITY + 1, BackpressureStrategy::Reject);
    }

    #[test]
    fn test_drop_count_tracking() {
        let mut q = BoundedQueue::new(2, BackpressureStrategy::DropOldest);
        q.push(1).unwrap();
        q.push(2).unwrap();
        assert_eq!(q.drop_count(), 0);

        q.push(3).unwrap(); // Drop 1
        assert_eq!(q.drop_count(), 1);

        q.push(4).unwrap(); // Drop 2
        assert_eq!(q.drop_count(), 2);
    }

    #[test]
    fn test_peek_does_not_remove() {
        let mut q = BoundedQueue::new(3, BackpressureStrategy::Reject);
        q.push(10).unwrap();
        q.push(20).unwrap();

        assert_eq!(q.peek(), Some(&10));
        assert_eq!(q.len(), 2); // Still 2 items
        assert_eq!(q.peek(), Some(&10)); // Can peek multiple times
        assert_eq!(q.pop(), Some(10)); // Now remove
        assert_eq!(q.peek(), Some(&20));
    }

    #[test]
    fn test_is_full_accurate() {
        let mut q = BoundedQueue::new(2, BackpressureStrategy::Reject);
        assert!(!q.is_full());

        q.push(1).unwrap();
        assert!(!q.is_full());

        q.push(2).unwrap();
        assert!(q.is_full());

        q.pop();
        assert!(!q.is_full());
    }

    #[test]
    fn test_circular_buffer_pattern_with_drop_oldest() {
        // Common use case: circular buffer for last N items
        let mut q = BoundedQueue::new(5, BackpressureStrategy::DropOldest);

        for i in 0..20 {
            q.push(i).unwrap();
        }

        // Should contain last 5: 15, 16, 17, 18, 19
        assert_eq!(q.len(), 5);
        assert_eq!(q.pop(), Some(15));
        assert_eq!(q.pop(), Some(16));
        assert_eq!(q.pop(), Some(17));
        assert_eq!(q.pop(), Some(18));
        assert_eq!(q.pop(), Some(19));
        assert_eq!(q.drop_count(), 15); // Dropped 0-14
    }
}
