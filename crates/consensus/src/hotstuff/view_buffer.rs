//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeMap, HashMap, VecDeque, btree_map::Entry, hash_map},
    fmt::Display,
    hash::Hash,
    mem,
    ops::Bound::{Excluded, Unbounded},
};

use tari_ootle_common_types::{Epoch, NodeHeight};

/// A consensus view, ordered by epoch then height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct View {
    pub epoch: Epoch,
    pub height: NodeHeight,
}

impl View {
    pub fn new(epoch: Epoch, height: NodeHeight) -> Self {
        Self { epoch, height }
    }
}

impl Display for View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.epoch, self.height)
    }
}

/// What an [`insert`](ViewBuffer::insert) did to make room for the new item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inserted {
    /// Items dropped from the furthest views to admit this one.
    pub num_evicted: usize,
}

/// A buffer of items belonging to views we have not reached yet, keyed by view and FIFO within a view. Each item
/// is held on behalf of the sender it came from.
///
/// Two budgets bound it, both across every view at once: the size the items hold, and how many there are. A
/// per-view budget would bound nothing, because the view an item names is chosen by its sender. Size is the
/// primary budget — items range from a vote to a block-sized proposal, so a count that is generous to ordinary
/// traffic admits that many of the largest items; the count is the secondary guard, covering the per-item
/// overhead that size alone misses.
///
/// Within either budget, the item furthest in the future is evicted to make room for a nearer one, and an item
/// that is itself the furthest is refused. Consensus consumes views in order, so the nearest views are the
/// ones it is about to need and a flood aimed at distant views cannot crowd them out. Views are ordered
/// epoch-major, so everything in the next epoch counts as further away than anything in this one: a caller
/// that admits arbitrary heights within the current epoch must bound them itself.
///
/// The nearest views are never evicted, so a third budget bounds the size each sender holds, given with every
/// insert. Within it, the sender's own furthest item is evicted to make room for its nearer one. A sender that
/// fills its share of a view leaves the rest of that view to the others.
pub struct ViewBuffer<K, T> {
    buffer: BTreeMap<View, VecDeque<Held<K, T>>>,
    sender_sizes: HashMap<K, usize>,
    len: usize,
    size: usize,
    max_items: usize,
    max_size: usize,
}

struct Held<K, T> {
    sender: K,
    size: usize,
    item: T,
}

impl<K: Eq + Hash + Clone, T> ViewBuffer<K, T> {
    /// Creates a buffer holding at most `max_items` items totalling at most `max_size`.
    ///
    /// # Panics
    /// Panics if either budget is zero.
    pub fn new(max_items: usize, max_size: usize) -> Self {
        assert!(max_items > 0, "ViewBuffer max_items must be non-zero");
        assert!(max_size > 0, "ViewBuffer max_size must be non-zero");
        Self {
            buffer: BTreeMap::new(),
            sender_sizes: HashMap::new(),
            len: 0,
            size: 0,
            max_items,
            max_size,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn max_items(&self) -> usize {
        self.max_items
    }

    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// The size of the items held on behalf of `sender`.
    pub fn sender_size(&self, sender: &K) -> usize {
        self.sender_sizes.get(sender).copied().unwrap_or(0)
    }

    /// Buffers `item` from `sender` for `view`, charging `size` against the size budget and against the
    /// sender's budget of `max_sender_size`. Returns `Err(item)` if the items held for views at or nearer than
    /// `view` already fill a budget, or if `size` alone exceeds one. A refused item evicts nothing.
    pub fn insert(
        &mut self,
        view: View,
        sender: K,
        item: T,
        size: usize,
        max_sender_size: usize,
    ) -> Result<Inserted, T> {
        if size > self.max_size || size > max_sender_size {
            return Err(item);
        }

        let over_budget = self.sender_size(&sender).saturating_add(size) > max_sender_size ||
            self.len >= self.max_items ||
            self.size.saturating_add(size) > self.max_size;
        if over_budget && !self.fits_without_views_beyond(view, &sender, size, max_sender_size) {
            return Err(item);
        }

        let mut num_evicted = 0;
        while self.sender_size(&sender).saturating_add(size) > max_sender_size {
            if !self.evict_furthest_from(&sender, view) {
                return Err(item);
            }
            num_evicted += 1;
        }

        while self.len >= self.max_items || self.size.saturating_add(size) > self.max_size {
            let Some(mut furthest) = self.buffer.last_entry() else {
                break;
            };
            if *furthest.key() <= view {
                return Err(item);
            }
            let Some(evicted) = furthest.get_mut().pop_back() else {
                break;
            };
            if furthest.get().is_empty() {
                furthest.remove();
            }
            self.release(&evicted);
            num_evicted += 1;
        }

        *self.sender_sizes.entry(sender.clone()).or_default() += size;
        self.buffer
            .entry(view)
            .or_default()
            .push_back(Held { sender, size, item });
        self.len += 1;
        self.size += size;
        Ok(Inserted { num_evicted })
    }

    /// Removes and returns the oldest item buffered for `view`, with its sender.
    pub fn pop_front(&mut self, view: &View) -> Option<(K, T)> {
        let Entry::Occupied(mut entry) = self.buffer.entry(*view) else {
            return None;
        };
        let held = entry.get_mut().pop_front()?;
        if entry.get().is_empty() {
            entry.remove();
        }
        self.release(&held);
        Some((held.sender, held.item))
    }

    /// Drops every item buffered for a view before `view`, returning the number dropped.
    pub fn discard_before(&mut self, view: View) -> usize {
        let mut discarded = mem::take(&mut self.buffer);
        self.buffer = discarded.split_off(&view);
        let mut num_discarded = 0;
        for held in discarded.into_values().flatten() {
            num_discarded += 1;
            self.release(&held);
        }
        num_discarded
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.sender_sizes.clear();
        self.len = 0;
        self.size = 0;
    }

    /// Whether an item of `size` from `sender` fits every budget once every item for a view beyond `view` is
    /// gone, which is as much as an insert for `view` may evict.
    fn fits_without_views_beyond(&self, view: View, sender: &K, size: usize, max_sender_size: usize) -> bool {
        let (mut len, mut total_size, mut sender_size) = (0usize, 0usize, 0usize);
        for held in self.buffer.range(..=view).flat_map(|(_, items)| items) {
            len += 1;
            total_size += held.size;
            if held.sender == *sender {
                sender_size += held.size;
            }
        }
        len < self.max_items &&
            total_size.saturating_add(size) <= self.max_size &&
            sender_size.saturating_add(size) <= max_sender_size
    }

    /// Evicts the item `sender` holds for the furthest view beyond `view`, returning false if it holds none.
    fn evict_furthest_from(&mut self, sender: &K, view: View) -> bool {
        let Some((furthest, index)) = self
            .buffer
            .range((Excluded(view), Unbounded))
            .rev()
            .find_map(|(v, items)| items.iter().rposition(|held| held.sender == *sender).map(|i| (*v, i)))
        else {
            return false;
        };
        let Entry::Occupied(mut entry) = self.buffer.entry(furthest) else {
            return false;
        };
        let Some(evicted) = entry.get_mut().remove(index) else {
            return false;
        };
        if entry.get().is_empty() {
            entry.remove();
        }
        self.release(&evicted);
        true
    }

    /// Takes a removed item off every budget it was charged to.
    fn release(&mut self, held: &Held<K, T>) {
        self.len -= 1;
        self.size -= held.size;
        if let hash_map::Entry::Occupied(mut entry) = self.sender_sizes.entry(held.sender.clone()) {
            *entry.get_mut() -= held.size;
            if *entry.get() == 0 {
                entry.remove();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_SIZE: usize = 0;
    const UNLIMITED: usize = usize::MAX;

    fn view(height: u64) -> View {
        View::new(Epoch(1), NodeHeight(height))
    }

    const SENDER: &str = "sender";

    type Buffer<T> = ViewBuffer<&'static str, T>;

    fn insert(buffer: &mut Buffer<&'static str>, view: View, item: &'static str) -> Result<Inserted, &'static str> {
        insert_sized(buffer, view, item, NO_SIZE)
    }

    fn insert_sized<T>(buffer: &mut Buffer<T>, view: View, item: T, size: usize) -> Result<Inserted, T> {
        buffer.insert(view, SENDER, item, size, UNLIMITED)
    }

    fn pop<T>(buffer: &mut Buffer<T>, view: &View) -> Option<T> {
        buffer.pop_front(view).map(|(_, item)| item)
    }

    #[test]
    fn it_pops_items_for_a_view_in_order() {
        let mut buffer = ViewBuffer::new(10, UNLIMITED);
        insert(&mut buffer, view(1), "a").unwrap();
        insert(&mut buffer, view(2), "b").unwrap();
        insert(&mut buffer, view(1), "c").unwrap();

        assert_eq!(pop(&mut buffer, &view(1)), Some("a"));
        assert_eq!(pop(&mut buffer, &view(1)), Some("c"));
        assert_eq!(pop(&mut buffer, &view(1)), None);
        assert_eq!(pop(&mut buffer, &view(2)), Some("b"));
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn it_discards_items_before_a_view() {
        let mut buffer = ViewBuffer::new(10, UNLIMITED);
        insert_sized(&mut buffer, view(1), "a", 10).unwrap();
        insert_sized(&mut buffer, view(1), "b", 10).unwrap();
        insert_sized(&mut buffer, view(2), "c", 10).unwrap();
        insert_sized(&mut buffer, View::new(Epoch(2), NodeHeight(1)), "d", 10).unwrap();

        assert_eq!(buffer.discard_before(view(2)), 2);
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.size(), 20);
        assert_eq!(pop(&mut buffer, &view(1)), None);
        assert_eq!(pop(&mut buffer, &view(2)), Some("c"));
    }

    #[test]
    fn a_single_view_cannot_exceed_the_item_budget() {
        let mut buffer = ViewBuffer::new(3, UNLIMITED);
        for i in 0..3 {
            insert_sized(&mut buffer, view(1), i, NO_SIZE).unwrap();
        }
        assert_eq!(insert_sized(&mut buffer, view(1), 3, NO_SIZE), Err(3));
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn many_views_cannot_exceed_the_item_budget() {
        let mut buffer = ViewBuffer::new(3, UNLIMITED);
        for height in 1..=100 {
            let _ignore = insert_sized(&mut buffer, view(height), height, NO_SIZE);
        }
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn many_small_items_cannot_exceed_the_size_budget() {
        let mut buffer = ViewBuffer::new(UNLIMITED, 100);
        for height in 1..=100 {
            let _ignore = insert_sized(&mut buffer, view(height), height, 10);
        }
        assert_eq!(buffer.size(), 100);
        assert_eq!(buffer.len(), 10);
    }

    #[test]
    fn a_few_large_items_cannot_exceed_the_size_budget() {
        let mut buffer = ViewBuffer::new(UNLIMITED, 100);
        insert_sized(&mut buffer, view(10), "large", 60).unwrap();
        insert_sized(&mut buffer, view(11), "large", 40).unwrap();

        // Nearer, so it evicts both of the above to fit.
        let inserted = insert_sized(&mut buffer, view(1), "largest", 100).unwrap();

        assert_eq!(inserted.num_evicted, 2);
        assert_eq!(buffer.size(), 100);
        assert_eq!(buffer.len(), 1);
        assert_eq!(pop(&mut buffer, &view(1)), Some("largest"));
        assert_eq!(buffer.size(), 0);
    }

    #[test]
    fn an_item_larger_than_the_size_budget_is_refused() {
        let mut buffer = ViewBuffer::new(UNLIMITED, 100);
        assert_eq!(insert_sized(&mut buffer, view(1), "too large", 101), Err("too large"));
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.size(), 0);
    }

    #[test]
    fn a_nearer_view_evicts_the_furthest_item() {
        let mut buffer = ViewBuffer::new(2, UNLIMITED);
        insert(&mut buffer, view(10), "far").unwrap();
        insert(&mut buffer, view(11), "further").unwrap();

        let inserted = insert(&mut buffer, view(1), "near").unwrap();

        assert_eq!(inserted.num_evicted, 1);
        assert_eq!(buffer.len(), 2);
        assert_eq!(pop(&mut buffer, &view(1)), Some("near"));
        assert_eq!(pop(&mut buffer, &view(10)), Some("far"));
        assert_eq!(pop(&mut buffer, &view(11)), None);
    }

    #[test]
    fn an_item_for_the_furthest_view_is_refused() {
        let mut buffer = ViewBuffer::new(2, UNLIMITED);
        insert(&mut buffer, view(1), "near").unwrap();
        insert(&mut buffer, view(10), "far").unwrap();

        assert_eq!(insert(&mut buffer, view(10), "same"), Err("same"));
        assert_eq!(insert(&mut buffer, view(11), "furthest"), Err("furthest"));
        assert_eq!(buffer.len(), 2);
        assert_eq!(pop(&mut buffer, &view(1)), Some("near"));
    }

    #[test]
    fn a_sender_cannot_exceed_its_budget_at_a_view() {
        let mut buffer = Buffer::new(UNLIMITED, 100);
        buffer.insert(view(1), "flooder", "a", 30, 50).unwrap();
        assert_eq!(buffer.insert(view(1), "flooder", "b", 30, 50), Err("b"));

        buffer.insert(view(1), "other", "c", 30, 50).unwrap();
        assert_eq!(buffer.sender_size(&"flooder"), 30);
        assert_eq!(buffer.sender_size(&"other"), 30);
        assert_eq!(buffer.pop_front(&view(1)), Some(("flooder", "a")));
        assert_eq!(buffer.pop_front(&view(1)), Some(("other", "c")));
        assert_eq!(buffer.sender_size(&"flooder"), 0);
    }

    #[test]
    fn a_sender_evicts_its_own_furthest_item_for_a_nearer_one() {
        let mut buffer = Buffer::new(UNLIMITED, 100);
        buffer.insert(view(5), "sender", "far", 30, 50).unwrap();
        buffer.insert(view(10), "other", "other", 30, 50).unwrap();

        let inserted = buffer.insert(view(1), "sender", "near", 30, 50).unwrap();

        assert_eq!(inserted.num_evicted, 1);
        assert_eq!(buffer.sender_size(&"sender"), 30);
        assert_eq!(buffer.pop_front(&view(5)), None);
        assert_eq!(buffer.pop_front(&view(10)), Some(("other", "other")));
        assert_eq!(buffer.pop_front(&view(1)), Some(("sender", "near")));
    }

    #[test]
    fn a_refused_item_evicts_nothing() {
        let mut buffer = Buffer::new(UNLIMITED, 100);
        buffer.insert(view(1), SENDER, "near", 30, 50).unwrap();
        buffer.insert(view(10), SENDER, "far", 10, 50).unwrap();
        buffer.insert(view(1), "other", "other", 60, 100).unwrap();

        assert_eq!(buffer.insert(view(2), SENDER, "refused", 20, 50), Err("refused"));
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.sender_size(&SENDER), 40);
        assert_eq!(buffer.pop_front(&view(10)), Some((SENDER, "far")));
    }

    #[test]
    fn an_item_larger_than_the_sender_budget_is_refused() {
        let mut buffer = Buffer::new(UNLIMITED, 100);
        assert_eq!(buffer.insert(view(1), SENDER, "too large", 51, 50), Err("too large"));
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn removed_items_are_released_from_their_sender_budget() {
        let mut buffer = Buffer::new(UNLIMITED, 100);
        buffer.insert(view(1), SENDER, "discarded", 30, 50).unwrap();
        buffer.insert(view(10), SENDER, "evicted", 20, 50).unwrap();
        buffer.insert(view(2), "other", "nearer", 70, 100).unwrap();
        assert_eq!(buffer.sender_size(&SENDER), 30);

        assert_eq!(buffer.discard_before(view(2)), 1);
        assert_eq!(buffer.sender_size(&SENDER), 0);
        assert_eq!(buffer.size(), 70);
    }
}
