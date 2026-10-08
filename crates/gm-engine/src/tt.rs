//! Fixed-size transposition table: power-of-two number of 64-byte buckets with four 16-byte
//! entries each, depth-preferred replacement with aging. Single-threaded (one table per Engine),
//! so no atomics are needed. Memory is allocated once in `Tt::new` and never grows.

pub const BOUND_NONE: u8 = 0;
pub const BOUND_UPPER: u8 = 1; // fail-low: score is an upper bound
pub const BOUND_LOWER: u8 = 2; // fail-high: score is a lower bound
pub const BOUND_EXACT: u8 = 3;

#[derive(Clone, Copy, Default, Debug)]
pub struct Entry {
    pub key: u64,
    pub mv: u16,
    pub score: i16,
    pub eval: i16,
    pub depth: u8,
    /// low 2 bits: bound; high 6 bits: generation
    pub gen_bound: u8,
}

impl Entry {
    #[inline]
    pub fn bound(&self) -> u8 {
        self.gen_bound & 3
    }
    #[inline]
    fn generation(&self) -> u8 {
        self.gen_bound >> 2
    }
}

const BUCKET: usize = 4;

#[derive(Clone, Copy, Default)]
#[repr(align(64))]
struct Bucket {
    e: [Entry; BUCKET],
}

pub struct Tt {
    buckets: Vec<Bucket>,
    mask: usize,
    generation: u8,
}

impl Tt {
    /// Allocate a table of (at most) `mb` mebibytes, rounded down to a power of two.
    pub fn new(mb: usize) -> Tt {
        let bytes = mb.clamp(1, 4096) * 1024 * 1024;
        let wanted = bytes / std::mem::size_of::<Bucket>();
        let mut n = 1usize;
        while n * 2 <= wanted {
            n *= 2;
        }
        Tt {
            buckets: vec![Bucket::default(); n],
            mask: n - 1,
            generation: 0,
        }
    }

    pub fn clear(&mut self) {
        self.buckets.iter_mut().for_each(|b| *b = Bucket::default());
        self.generation = 0;
    }

    /// Start a new search: older entries become preferred replacement victims.
    pub fn new_search(&mut self) {
        self.generation = (self.generation + 1) & 0x3F;
    }

    #[inline]
    fn index(&self, key: u64) -> usize {
        // use the high bits for the index; the full key is verified on probe
        ((key >> 32) as usize ^ key as usize) & self.mask
    }

    #[inline]
    pub fn probe(&self, key: u64) -> Option<Entry> {
        let b = self.buckets.get(self.index(key))?;
        b.e.iter()
            .find(|e| e.key == key && e.bound() != BOUND_NONE)
            .copied()
    }

    pub fn store(&mut self, key: u64, mv: u16, score: i32, eval: i32, depth: i32, bound: u8) {
        let generation = self.generation;
        let idx = self.index(key);
        let Some(bucket) = self.buckets.get_mut(idx) else {
            return;
        };
        let depth = depth.clamp(0, 255) as u8;

        // Pick the slot: same key, else an empty slot, else the least valuable entry.
        let mut slot = 0;
        let mut worst = i32::MAX;
        for (i, e) in bucket.e.iter().enumerate() {
            if e.key == key || e.bound() == BOUND_NONE {
                slot = i;
                break;
            }
            let age = (generation.wrapping_sub(e.generation()) & 0x3F) as i32;
            let value = e.depth as i32 - 8 * age;
            if value < worst {
                worst = value;
                slot = i;
            }
        }
        let e = &mut bucket.e[slot];
        let same = e.key == key;
        // Keep a deeper entry for the same position unless the new one is exact or the old one
        // is from an older search.
        if same
            && bound != BOUND_EXACT
            && e.generation() == generation
            && (depth as i32) + 3 < e.depth as i32
        {
            return;
        }
        let mv = if mv == 0 && same { e.mv } else { mv };
        *e = Entry {
            key,
            mv,
            score: score.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            eval: eval.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            depth,
            gen_bound: (generation << 2) | (bound & 3),
        };
    }

    pub fn size_bytes(&self) -> usize {
        self.buckets.len() * std::mem::size_of::<Bucket>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(std::mem::size_of::<Entry>(), 16);
        assert_eq!(std::mem::size_of::<Bucket>(), 64);
        let t = Tt::new(16);
        assert_eq!(t.size_bytes(), 16 * 1024 * 1024);
        let t = Tt::new(3);
        assert_eq!(t.size_bytes(), 2 * 1024 * 1024);
    }

    #[test]
    fn store_probe() {
        let mut t = Tt::new(1);
        t.store(42, 7, 100, 5, 6, BOUND_EXACT);
        let e = t.probe(42).unwrap();
        assert_eq!(
            (e.mv, e.score, e.depth, e.bound()),
            (7, 100, 6, BOUND_EXACT)
        );
        assert!(t.probe(43).is_none());
        // shallower non-exact store for the same key does not clobber a much deeper entry
        t.store(42, 9, -50, 5, 1, BOUND_LOWER);
        assert_eq!(t.probe(42).unwrap().depth, 6);
        t.clear();
        assert!(t.probe(42).is_none());
    }
}
