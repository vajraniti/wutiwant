//! Синтетический генератор потока операций (`Op`).
//!
//! Модель простая, но даёт правдоподобную микроструктуру: латентная справедливая
//! цена `mid` блуждает случайно, вокруг неё выставляются лимитные заявки, часть
//! ордеров — агрессивные (пересекают спред и матчатся), часть ранее выставленных
//! отменяется. В отличие от «маргинального» генератора событий, тут поток идёт
//! через **настоящую** книгу — сделки рождаются реальным матчингом, а не печатаются
//! вручную.
//!
//! Цена держится в узком коридоре вокруг `reference`, потому что ладдер книги
//! фиксирован на `±LADDER_SIZE/2` тиков от точки отсчёта — выход за коридор книга
//! отвергнет. Параметры дефолта подобраны так, чтобы случайное блуждание за разумный
//! горизонт из коридора не выходило.

use crate::ops::Op;
use crate::rng::Rng;
use crate::types::{OrderId, Price, Qty, SelfTradeMode, Side, SubaccountId};

#[derive(Debug, Clone, Copy)]
pub struct GeneratorConfig {
    /// Стартовая (и центральная) `mid`, тики. Совпадает с `reference` книги.
    pub start_mid: i64,
    /// Жёсткий коридор: `mid` зажимается в `[start_mid - drift_clamp, start_mid + drift_clamp]`,
    /// чтобы цены не вышли за фиксированный ладдер. Должен быть заметно меньше
    /// `LADDER_SIZE/2` с запасом на глубину выставления.
    pub drift_clamp: i64,
    /// Шаг случайного блуждания `mid`: на каждом событии `mid` сдвигается на
    /// `range(-step, step)` тиков.
    pub walk_step: i64,
    /// Перекос сторон: `P(Bid)`. `0.5` — баланс.
    pub side_bias: f64,
    /// Доля агрессивных ордеров (пересекают спред → матчатся).
    pub aggressive_fraction: f64,
    /// Доля операций-отмен (если есть что отменять).
    pub cancel_fraction: f64,
    /// На сколько тиков лимитная заявка отступает от `mid` (в свою сторону).
    /// Реальный отступ — `range(0, passive_depth)`.
    pub passive_depth: i64,
    /// На сколько тиков агрессивный ордер заходит за `mid` (в чужую сторону),
    /// чтобы гарантированно пересечь. Реальный заход — `range(1, aggressive_reach)`.
    pub aggressive_reach: i64,
    /// Базовый размер заявки: `range(1, base_qty)`.
    pub base_qty: i64,
    /// Вероятность «крупной» заявки с тяжёлым хвостом.
    pub big_qty_fraction: f64,
    /// Добавка к размеру для крупной заявки: `range(base_qty, base_qty + big_qty_extra)`.
    pub big_qty_extra: i64,
    /// Число субаккаунтов (`1..=n_subaccounts`). >1 — иногда срабатывает STP.
    pub n_subaccounts: u64,
    /// «Поводок» к книге: latent `mid` не отпускается дальше `leash` тиков от
    /// середины текущей книги. Без него скрытая цена убегает быстрее, чем матчинг
    /// разгребает стоячую ликвидность, и книга отстаёт на сотни тиков (огромный
    /// спред, навал ордеров на старых уровнях). Маркет-мейкер котирует вокруг
    /// текущего стакана, а не вокруг оторванной «справедливой» цены — поводок это и
    /// моделирует. Должен быть соизмерим с `passive_depth`.
    pub leash: i64,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            start_mid: 10_000,
            drift_clamp: 400,
            walk_step: 2,
            side_bias: 0.5,
            aggressive_fraction: 0.35,
            cancel_fraction: 0.15,
            passive_depth: 8,
            aggressive_reach: 3,
            base_qty: 20,
            big_qty_fraction: 0.05,
            big_qty_extra: 300,
            n_subaccounts: 4,
            leash: 12,
        }
    }
}

/// Что именно сгенерил генератор на этом шаге — нужно драйверу для разметки CSV
/// (агрессивный ордер ожидаемо порождает сделки, пассивный встаёт в книгу).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    PassivePlace,
    AggressivePlace,
    Cancel,
}

pub struct Generator {
    cfg: GeneratorConfig,
    rng: Rng,
    mid: i64,
    next_id: u64,
    /// Ранее выставленные пассивные ордера — кандидаты на отмену.
    /// Храним владельца: отмена чужого ордера была бы отвергнута (`NotOwner`).
    live: Vec<(OrderId, SubaccountId)>,
}

impl Generator {
    pub fn new(cfg: GeneratorConfig, seed: u64) -> Self {
        Self {
            cfg,
            rng: Rng::new(seed),
            mid: cfg.start_mid,
            next_id: 1,
            live: Vec::new(),
        }
    }

    pub fn mid(&self) -> i64 {
        self.mid
    }

    fn fresh_id(&mut self) -> OrderId {
        let id = OrderId(self.next_id);
        self.next_id += 1;
        id
    }

    fn sample_qty(&mut self) -> i64 {
        let base = self.rng.range(1, self.cfg.base_qty);
        if self.rng.chance(self.cfg.big_qty_fraction) {
            base + self.rng.range(self.cfg.base_qty, self.cfg.base_qty + self.cfg.big_qty_extra)
        } else {
            base
        }
    }

    fn sample_subaccount(&mut self) -> SubaccountId {
        SubaccountId(1 + self.rng.next_u64() % self.cfg.n_subaccounts)
    }

    /// Следующая операция. Вместе с `Op` отдаём `OpKind` для разметки данных.
    ///
    /// `best_bid`/`best_ask` — текущий верх книги (сырые тики), нужны чтобы держать
    /// latent `mid` на поводке у стакана. `None` на стороне — её ещё нет (книга
    /// наполняется), тогда поводок не натягиваем.
    pub fn next_op(&mut self, best_bid: Option<i64>, best_ask: Option<i64>) -> (Op, OpKind) {
        // 1. Латентная mid делает шаг случайного блуждания и зажимается в коридор.
        let step = self.rng.range(-self.cfg.walk_step, self.cfg.walk_step);
        let lo = self.cfg.start_mid - self.cfg.drift_clamp;
        let hi = self.cfg.start_mid + self.cfg.drift_clamp;
        self.mid = (self.mid + step).clamp(lo, hi);

        // 1b. Поводок к книге: не отпускаем mid дальше leash от середины стакана.
        // Так цена не убегает от матчируемых уровней, и книга остаётся тугой.
        if let (Some(b), Some(a)) = (best_bid, best_ask) {
            let book_mid = (a + b) / 2;
            self.mid = self
                .mid
                .clamp(book_mid - self.cfg.leash, book_mid + self.cfg.leash);
        }

        // 2. Иногда — отмена ранее выставленного ордера.
        if !self.live.is_empty() && self.rng.chance(self.cfg.cancel_fraction) {
            let idx = self.rng.below(self.live.len());
            let (order_id, subaccount) = self.live.swap_remove(idx);
            return (Op::Cancel { order_id, subaccount }, OpKind::Cancel);
        }

        // 3. Иначе — новая заявка.
        let side = if self.rng.chance(self.cfg.side_bias) {
            Side::Bid
        } else {
            Side::Ask
        };
        let subaccount = self.sample_subaccount();
        let qty = self.sample_qty();
        let order_id = self.fresh_id();

        let aggressive = self.rng.chance(self.cfg.aggressive_fraction);
        let (price_val, kind) = if aggressive {
            // Заходим за mid в чужую сторону, чтобы пересечь спред и сматчиться.
            let reach = self.rng.range(1, self.cfg.aggressive_reach);
            let p = match side {
                Side::Bid => self.mid + reach,
                Side::Ask => self.mid - reach,
            };
            (p, OpKind::AggressivePlace)
        } else {
            // Отступаем от mid в свою сторону — пассивная заявка встаёт в книгу.
            let offset = self.rng.range(0, self.cfg.passive_depth);
            let p = match side {
                Side::Bid => self.mid - offset,
                Side::Ask => self.mid + offset,
            };
            (p, OpKind::PassivePlace)
        };

        let price = Price::new(price_val.max(1)).expect("price_val.max(1) >= 1");
        let qty = Qty::new(qty).expect("sample_qty >= 1");

        if kind == OpKind::PassivePlace {
            self.live.push((order_id, subaccount));
        }

        let op = Op::Place {
            order_id,
            subaccount,
            side,
            price,
            qty,
            stp: SelfTradeMode::ExpireTaker,
        };
        (op, kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = Generator::new(GeneratorConfig::default(), 42);
        let mut b = Generator::new(GeneratorConfig::default(), 42);
        for _ in 0..1000 {
            let (oa, ka) = a.next_op(None, None);
            let (ob, kb) = b.next_op(None, None);
            assert_eq!(ka, kb);
            // сверяем хотя бы id и kind — Op не derive PartialEq, этого достаточно
            assert_eq!(op_id(&oa), op_id(&ob));
        }
    }

    fn op_id(op: &Op) -> u64 {
        match op {
            Op::Place { order_id, .. } => order_id.0,
            Op::Cancel { order_id, .. } => order_id.0,
        }
    }

    #[test]
    fn mid_stays_in_corridor() {
        let cfg = GeneratorConfig::default();
        let mut g = Generator::new(cfg, 7);
        let lo = cfg.start_mid - cfg.drift_clamp;
        let hi = cfg.start_mid + cfg.drift_clamp;
        for _ in 0..100_000 {
            g.next_op(None, None);
            assert!((lo..=hi).contains(&g.mid()), "mid вышла из коридора: {}", g.mid());
        }
    }

    #[test]
    fn produces_all_three_kinds() {
        let mut g = Generator::new(GeneratorConfig::default(), 3);
        let mut passive = false;
        let mut aggressive = false;
        let mut cancel = false;
        for _ in 0..10_000 {
            match g.next_op(None, None).1 {
                OpKind::PassivePlace => passive = true,
                OpKind::AggressivePlace => aggressive = true,
                OpKind::Cancel => cancel = true,
            }
        }
        assert!(passive && aggressive && cancel, "должны встретиться все три типа");
    }
}
