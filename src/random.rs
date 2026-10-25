//! A bit-for-bit reimplementation of the parts of CPython's `random` module
//! that the maze generator uses (`seed(str)`, `randint`, `shuffle`).
//!
//! Matching CPython exactly means a given maze id produces the same maze in
//! both the Python and the Rust versions.

use sha2::{Digest, Sha512};

const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

pub struct PyRandom {
    mt: [u32; N],
    index: usize,
}

impl PyRandom {
    /// Equivalent to `random.seed(s)` for a `str` argument (seed version 2).
    pub fn from_str_seed(s: &str) -> Self {
        let mut bytes = s.as_bytes().to_vec();
        bytes.extend_from_slice(&Sha512::digest(s.as_bytes()));

        // int.from_bytes(bytes, 'big'), split into 32-bit words, least
        // significant word first.
        let pad = (4 - bytes.len() % 4) % 4;
        let mut padded = vec![0u8; pad];
        padded.extend_from_slice(&bytes);
        let mut key: Vec<u32> = padded
            .chunks(4)
            .rev()
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        while key.len() > 1 && *key.last().unwrap() == 0 {
            key.pop();
        }

        let mut r = PyRandom { mt: [0; N], index: N };
        r.init_by_array(&key);
        r
    }

    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..N {
            let prev = self.mt[i - 1];
            self.mt[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        self.index = N;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19_650_218);
        let mut i = 1usize;
        let mut j = 0usize;
        for _ in 0..N.max(key.len()) {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..N - 1 {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_566_083_941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
        }
        self.mt[0] = 0x8000_0000;
        self.index = N;
    }

    fn genrand_u32(&mut self) -> u32 {
        if self.index >= N {
            for kk in 0..N {
                let y = (self.mt[kk] & UPPER_MASK) | (self.mt[(kk + 1) % N] & LOWER_MASK);
                let mag = if y & 1 == 1 { MATRIX_A } else { 0 };
                self.mt[kk] = self.mt[(kk + M) % N] ^ (y >> 1) ^ mag;
            }
            self.index = 0;
        }
        let mut y = self.mt[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    /// `getrandbits(k)` for 0 <= k <= 32.
    fn getrandbits(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        self.genrand_u32() >> (32 - k)
    }

    /// `_randbelow_with_getrandbits(n)`.
    fn randbelow(&mut self, n: u32) -> u32 {
        let k = 32 - n.leading_zeros();
        let mut r = self.getrandbits(k);
        while r >= n {
            r = self.getrandbits(k);
        }
        r
    }

    /// `randint(a, b)`, inclusive of both ends.
    pub fn randint(&mut self, a: i64, b: i64) -> i64 {
        a + self.randbelow((b - a + 1) as u32) as i64
    }

    /// `shuffle(x)`.
    pub fn shuffle<T>(&mut self, x: &mut [T]) {
        for i in (1..x.len()).rev() {
            let j = self.randbelow((i + 1) as u32) as usize;
            x.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_cpython_randint() {
        // python3 -c "import random; random.seed('abc'); print([random.randint(0, 99) for _ in range(10)])"
        let mut r = PyRandom::from_str_seed("abc");
        let got: Vec<i64> = (0..10).map(|_| r.randint(0, 99)).collect();
        assert_eq!(got, vec![98, 71, 58, 90, 45, 99, 45, 20, 34, 7]);
    }

    #[test]
    fn matches_cpython_shuffle() {
        // python3 -c "import random; random.seed('abc'); x=list(range(10)); random.shuffle(x); print(x)"
        let mut r = PyRandom::from_str_seed("abc");
        let mut x: Vec<i32> = (0..10).collect();
        r.shuffle(&mut x);
        assert_eq!(x, vec![4, 3, 0, 9, 1, 2, 6, 5, 7, 8]);
    }
}
