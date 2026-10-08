//! Deterministic phrase variation: the same position + move always produces the same sentence,
//! but different positions produce different wording so the coach doesn't sound robotic.

/// FNV-1a 64-bit hash.
pub fn hash(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A small deterministic picker; each call with a different `salt` gives an independent choice.
#[derive(Clone, Copy, Debug)]
pub struct Picker(u64);

impl Picker {
    pub fn new(parts: &[&str]) -> Self {
        Picker(hash(parts))
    }

    pub fn pick<'a>(&self, salt: u64, options: &[&'a str]) -> &'a str {
        if options.is_empty() {
            return "";
        }
        let mut x = self.0 ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        // splitmix64 finalizer
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^= x >> 31;
        options[(x % options.len() as u64) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a = Picker::new(&["fen", "e2e4"]);
        let b = Picker::new(&["fen", "e2e4"]);
        assert_eq!(a.pick(1, &["x", "y", "z"]), b.pick(1, &["x", "y", "z"]));
    }
}
