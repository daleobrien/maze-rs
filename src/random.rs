//! The random number generator behind the mazes: wyrand, seeded from a hash
//! of the maze id, so a given id always produces the same maze.
//!
//! It is chosen for speed rather than to match CPython's `random`, so the
//! same id gives a different maze here than in the Python version.

/// wyrand's increment and output-mixing constants.
const WY_INC: u64 = 0xa076_1d64_78bd_642f;
const WY_MIX: u64 = 0xe703_7ed1_a0b4_28db;

pub struct Rng {
    state: u64,
}

impl Rng {
    /// Seed from a string, such as a maze id.
    #[must_use]
    pub fn from_str_seed(s: &str) -> Self {
        // FNV-1a over the bytes, then a SplitMix64 finaliser so that ids
        // differing in one character still start far apart
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in s.as_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        Rng { state: h ^ (h >> 31) }
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(WY_INC);
        let t = u128::from(self.state) * u128::from(self.state ^ WY_MIX);
        (t >> 64) as u64 ^ t as u64
    }

    /// A uniform integer in `0..n`, by Lemire's nearly divisionless method:
    /// one multiply, and a division only on the rare draws that need
    /// rejecting to stay unbiased.
    ///
    /// # Panics
    ///
    /// If `n` is zero.
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below needs a non-empty range");
        let mut m = u128::from(self.next_u64()) * u128::from(n);
        if (m as u64) < n {
            let threshold = n.wrapping_neg() % n;
            while (m as u64) < threshold {
                m = u128::from(self.next_u64()) * u128::from(n);
            }
        }
        (m >> 64) as u64
    }

    /// A fair coin flip.
    #[inline]
    pub fn coin(&mut self) -> bool {
        self.next_u64() >> 63 == 1
    }

    /// Shuffle `x` in place (Fisher-Yates).
    pub fn shuffle<T>(&mut self, x: &mut [T]) {
        for i in (1..x.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            x.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_numbers() {
        let mut a = Rng::from_str_seed("abc");
        let mut b = Rng::from_str_seed("abc");
        let mut c = Rng::from_str_seed("abd");
        let xs: Vec<u64> = (0..10).map(|_| a.below(100)).collect();
        assert_eq!(xs, (0..10).map(|_| b.below(100)).collect::<Vec<_>>());
        assert_ne!(xs, (0..10).map(|_| c.below(100)).collect::<Vec<_>>());
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut r = Rng::from_str_seed("range");
        let mut seen = [0u32; 7];
        for _ in 0..7000 {
            seen[r.below(7) as usize] += 1;
        }
        // each value should come up roughly 1000 times
        assert!(seen.iter().all(|&n| (800..1200).contains(&n)), "{seen:?}");
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut r = Rng::from_str_seed("abc");
        let mut x: Vec<i32> = (0..100).collect();
        r.shuffle(&mut x);
        assert_ne!(x, (0..100).collect::<Vec<_>>());
        x.sort_unstable();
        assert_eq!(x, (0..100).collect::<Vec<_>>());
    }
}
