//! SplitMix64: a small seeded generator, the same numbers on every
//! platform and in every build (AGENTS.md rule 5).

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut rng = Rng(seed ^ 0x3d3d_c4af_7f00_0001);
        rng.next_u64();
        rng
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A whole number in `lo..=hi` (`lo` when the range is empty).
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        let span = hi.abs_diff(lo).saturating_add(1);
        let pick = self.next_u64() % span;
        lo.saturating_add_unsigned(pick)
    }

    /// An index below `n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        usize::try_from(self.next_u64() % (n as u64)).unwrap_or(0)
    }

    /// True with probability `percent` / 100.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }

    /// One of `items`; `None` when there is none.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        let at = self.below(items.len());
        items.get(at)
    }

    /// An index chosen with the given weights; 0 when they are all zero.
    pub fn weighted(&mut self, weights: &[u64]) -> usize {
        let total: u64 = weights.iter().sum();
        if total == 0 {
            return 0;
        }
        let mut at = self.next_u64() % total;
        for (i, w) in weights.iter().enumerate() {
            if at < *w {
                return i;
            }
            at -= w;
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_numbers() {
        let a: Vec<u64> = (0..8)
            .scan(Rng::new(7), |r, _| Some(r.next_u64()))
            .collect();
        let b: Vec<u64> = (0..8)
            .scan(Rng::new(7), |r, _| Some(r.next_u64()))
            .collect();
        assert_eq!(a, b);
        let c: Vec<u64> = (0..8)
            .scan(Rng::new(8), |r, _| Some(r.next_u64()))
            .collect();
        assert_ne!(a, c);
    }

    #[test]
    fn ranges_stay_inside() {
        let mut r = Rng::new(1);
        for _ in 0..1000 {
            let x = r.range(-4, 4);
            assert!((-4..=4).contains(&x));
            assert!(r.below(3) < 3);
        }
        assert_eq!(r.range(5, 5), 5);
        assert_eq!(r.below(0), 0);
        assert_eq!(r.weighted(&[0, 0]), 0);
    }
}
