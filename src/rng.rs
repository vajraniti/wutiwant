//! Крошечный детерминированный PRNG (SplitMix64) — ноль зависимостей.
//!
//! Незачем тащить крейт `rand` ради синтетики: SplitMix64 — это ~5 строк,
//! проходит статтесты для наших нужд (генерация потока ордеров), и фиксированный
//! `seed` даёт бит-в-бит воспроизводимый прогон. Воспроизводимость важна: один и
//! тот же датасет можно пересобрать под анализ в любой момент.

pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // 0 как seed для SplitMix64 рабочий, но смешаем на всякий случай.
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// Сырое 64-битное число. Ядро SplitMix64.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `f64` в `[0, 1)`. Берём старшие 53 бита — ровно мантисса double.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `true` с вероятностью `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Целое в `[lo, hi]` включительно. `lo <= hi` обязателен.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(lo <= hi);
        let span = (hi - lo + 1) as u64;
        lo + (self.next_u64() % span) as i64
    }

    /// Индекс в `[0, n)`. `n > 0` обязателен.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seed_diverges() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn unit_in_range() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn range_respects_bounds() {
        let mut r = Rng::new(123);
        for _ in 0..10_000 {
            let v = r.range(-5, 5);
            assert!((-5..=5).contains(&v));
        }
    }

    #[test]
    fn chance_roughly_calibrated() {
        let mut r = Rng::new(99);
        let n = 100_000;
        let hits = (0..n).filter(|_| r.chance(0.3)).count();
        let frac = hits as f64 / n as f64;
        assert!((0.28..0.32).contains(&frac), "ожидали ~0.3, получили {frac}");
    }
}
