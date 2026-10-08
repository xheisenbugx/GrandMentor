//! Tiny bounded LRU-ish caches (capacity-capped, least-recently-used eviction).

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

use parking_lot::Mutex;

/// Bounded cache with LRU eviction. `O(capacity)` on hit-refresh, which is fine for the
/// small capacities used here (<= a few hundred entries).
pub struct BoundedCache<K, V> {
    inner: Mutex<Inner<K, V>>,
    capacity: usize,
}

struct Inner<K, V> {
    map: HashMap<K, V>,
    order: VecDeque<K>, // front = least recently used
}

impl<K: Eq + Hash + Clone, V: Clone> BoundedCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        BoundedCache {
            inner: Mutex::new(Inner {
                map: HashMap::with_capacity(capacity),
                order: VecDeque::with_capacity(capacity),
            }),
            capacity,
        }
    }

    pub fn get(&self, key: &K) -> Option<V> {
        let mut g = self.inner.lock();
        let v = g.map.get(key).cloned()?;
        if let Some(pos) = g.order.iter().position(|k| k == key) {
            if let Some(k) = g.order.remove(pos) {
                g.order.push_back(k);
            }
        }
        Some(v)
    }

    pub fn insert(&self, key: K, value: V) {
        let mut g = self.inner.lock();
        if g.map.insert(key.clone(), value).is_some() {
            if let Some(pos) = g.order.iter().position(|k| k == &key) {
                g.order.remove(pos);
            }
        }
        g.order.push_back(key);
        while g.map.len() > self.capacity {
            match g.order.pop_front() {
                Some(old) => {
                    g.map.remove(&old);
                }
                None => break,
            }
        }
    }

    pub fn remove(&self, key: &K) {
        let mut g = self.inner.lock();
        if g.map.remove(key).is_some() {
            if let Some(pos) = g.order.iter().position(|k| k == key) {
                g.order.remove(pos);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Bounded "recently seen" set: remembers the last `capacity` keys (FIFO).
pub struct RecentSet<K> {
    inner: Mutex<(VecDeque<K>, std::collections::HashSet<K>)>,
    capacity: usize,
}

impl<K: Eq + Hash + Clone> RecentSet<K> {
    pub fn new(capacity: usize) -> Self {
        RecentSet {
            inner: Mutex::new((VecDeque::new(), std::collections::HashSet::new())),
            capacity: capacity.max(1),
        }
    }

    pub fn insert(&self, key: K) {
        let mut g = self.inner.lock();
        let (order, set) = &mut *g;
        if set.contains(&key) {
            if let Some(pos) = order.iter().position(|k| k == &key) {
                order.remove(pos);
            }
        } else {
            set.insert(key.clone());
        }
        order.push_back(key);
        while order.len() > self.capacity {
            if let Some(old) = order.pop_front() {
                set.remove(&old);
            }
        }
    }

    pub fn contains(&self, key: &K) -> bool {
        self.inner.lock().1.contains(key)
    }

    /// Snapshot of the set (for filtering many candidates under one lock).
    pub fn snapshot(&self) -> std::collections::HashSet<K> {
        self.inner.lock().1.clone()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_is_bounded_and_lru() {
        let c = BoundedCache::new(2);
        c.insert(1, "a");
        c.insert(2, "b");
        assert_eq!(c.get(&1), Some("a")); // 1 becomes most recent
        c.insert(3, "c"); // evicts 2
        assert_eq!(c.get(&2), None);
        assert_eq!(c.get(&1), Some("a"));
        assert_eq!(c.get(&3), Some("c"));
        assert_eq!(c.len(), 2);
        c.insert(3, "d");
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(&3), Some("d"));
    }

    #[test]
    fn recent_set_is_bounded() {
        let r = RecentSet::new(3);
        for i in 0..10 {
            r.insert(i);
        }
        assert_eq!(r.len(), 3);
        assert!(r.contains(&9) && r.contains(&7) && !r.contains(&6));
        r.insert(7);
        assert_eq!(r.len(), 3);
        r.insert(10);
        assert!(r.contains(&7) && !r.contains(&8));
    }
}
