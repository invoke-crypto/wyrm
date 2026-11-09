//! A fixed-capacity ring on storage. Entries are numbered by a sequence that never repeats; entry `seq` lives in slot
//! `seq % cap`, so the newest `cap` entries are kept and storage never grows past `cap` slots.
use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::store::Vector;
use near_sdk::{near, IntoStorageKey};

#[near(serializers = [borsh])]
pub struct Ring<T>
where
    T: BorshSerialize,
{
    items: Vector<T>,
    pushed: u64,
    cap: u32,
}

impl<T> Ring<T>
where
    T: BorshSerialize + BorshDeserialize,
{
    pub fn new(prefix: impl IntoStorageKey, cap: u32) -> Self {
        Self {
            items: Vector::new(prefix),
            pushed: 0,
            cap,
        }
    }

    /// Appends and returns the entry's sequence number, overwriting the oldest entry when full.
    pub fn push(&mut self, item: T) -> u64 {
        let seq = self.pushed;
        if self.items.len() < self.cap {
            self.items.push(item);
        } else {
            self.items.set((seq % self.cap as u64) as u32, item);
        }
        self.pushed += 1;
        seq
    }

    pub fn len(&self) -> u32 {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.items.len() == self.cap
    }

    pub fn free_slots(&self) -> u32 {
        self.cap - self.items.len()
    }

    /// The sequence number of the oldest entry kept.
    pub fn first_seq(&self) -> u64 {
        self.pushed - self.items.len() as u64
    }

    pub fn next_seq(&self) -> u64 {
        self.pushed
    }

    pub fn get(&self, seq: u64) -> Option<&T> {
        if seq < self.first_seq() || seq >= self.pushed {
            return None;
        }
        self.items.get((seq % self.cap as u64) as u32)
    }

    pub fn get_mut(&mut self, seq: u64) -> Option<&mut T> {
        if seq < self.first_seq() || seq >= self.pushed {
            return None;
        }
        self.items.get_mut((seq % self.cap as u64) as u32)
    }

    /// The `i`-th kept entry, oldest first.
    pub fn nth(&self, i: u32) -> Option<&T> {
        self.get(self.first_seq() + i as u64)
    }

    pub fn newest(&self) -> Option<&T> {
        self.pushed.checked_sub(1).and_then(|s| self.get(s))
    }

    /// Up to `n` entries, newest first.
    pub fn newest_first(&self, n: u32) -> impl Iterator<Item = &T> {
        let first = self.first_seq();
        (first..self.pushed)
            .rev()
            .take(n as usize)
            .filter_map(move |s| self.get(s))
    }
}
