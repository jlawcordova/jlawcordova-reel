//! The single seeded PRNG every random choice comes from.
//!
//! xoshiro256** seeded through splitmix64. Independent streams are derived
//! with [`Rng::fork`], which hashes a label into the parent's key, so a stream
//! only depends on the seed and on its label path, never on how many numbers
//! another stream consumed. That keeps sections, roles and notes independent:
//! restricting one section's instruments cannot shift the noise of another.

#[derive(Clone, Debug)]
pub struct Rng {
    key: u64,
    s: [u64; 4],
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a, used to turn labels into fork keys.
pub fn hash_label(label: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in label.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self::from_key(seed ^ 0x636F_7A79_2D73_636F)
    }

    fn from_key(key: u64) -> Self {
        let mut st = key;
        let s = [
            splitmix64(&mut st),
            splitmix64(&mut st),
            splitmix64(&mut st),
            splitmix64(&mut st),
        ];
        Rng { key, s }
    }

    /// An independent stream named by `label` under this one.
    pub fn fork(&self, label: &str) -> Rng {
        self.fork_n(hash_label(label))
    }

    /// An independent stream named by a number under this one.
    pub fn fork_n(&self, n: u64) -> Rng {
        let mut st = self.key ^ n.rotate_left(17);
        let a = splitmix64(&mut st);
        let b = splitmix64(&mut st);
        Self::from_key(a ^ b.rotate_left(29))
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }

    /// Uniform integer in [0, n).
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0);
        ((self.next_u64() >> 11) % n as u64) as usize
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.uniform() < p
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// Standard normal sample (Box-Muller, libm only).
    pub fn gauss(&mut self) -> f64 {
        let u1 = 1.0 - self.uniform(); // (0, 1]
        let u2 = self.uniform();
        libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(2.0 * core::f64::consts::PI * u2)
    }

    /// Index chosen with probability proportional to `weights`.
    pub fn weighted(&mut self, weights: &[f64]) -> usize {
        let total: f64 = weights.iter().sum();
        let mut x = self.uniform() * total;
        for (i, w) in weights.iter().enumerate() {
            if x < *w {
                return i;
            }
            x -= w;
        }
        weights.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn forks_do_not_depend_on_parent_consumption() {
        let a = Rng::new(7);
        let mut b = Rng::new(7);
        b.next_u64();
        assert_eq!(a.fork("x").next_u64(), b.fork("x").next_u64());
        assert_ne!(a.fork("x").next_u64(), a.fork("y").next_u64());
    }

    #[test]
    fn gauss_is_roughly_standard() {
        let mut r = Rng::new(1);
        let n = 20000;
        let xs: Vec<f64> = (0..n).map(|_| r.gauss()).collect();
        let mean = xs.iter().sum::<f64>() / n as f64;
        let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n as f64;
        assert!(mean.abs() < 0.03, "{mean}");
        assert!((var - 1.0).abs() < 0.05, "{var}");
    }
}
