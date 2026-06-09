use crate::level::PriceLevel;
use crate::pool::{OrderHandle, OrderPool};
use crate::types::{Price, Side};

pub const LADDER_SIZE: usize = 1024;

pub fn price_to_index(price: Price, reference: Price) -> Option<usize> {
    let offset = price.raw() - reference.raw();
    let centered = offset + (LADDER_SIZE / 2) as i64;

    if centered < 0 || centered >= LADDER_SIZE as i64 {
        return None;
    }

    Some(centered as usize)
}

pub fn index_to_price(idx: usize, reference: Price) -> Price {
    let offset = idx as i64 - (LADDER_SIZE / 2) as i64;
    Price::new(reference.raw() + offset).unwrap()
}

pub struct PriceLadder {
    levels: Box<[PriceLevel; LADDER_SIZE]>,
    reference: Price,
    best: Option<u32>,
    side: Side,
}

impl PriceLadder {
    pub fn new(reference: Price, side: Side) -> Self {
        Self {
            levels: Box::new(std::array::from_fn(|_| PriceLevel::new())),
            reference,
            best: None,
            side,
        }
    }

    fn is_better(&self, new_idx: u32, current_best: u32) -> bool {
        match self.side {
            Side::Bid => new_idx > current_best,
            Side::Ask => new_idx < current_best,
        }
    }

    pub fn add_order(
        &mut self,
        handle: OrderHandle,
        price: Price,
        qty: i64,
        pool: &mut OrderPool,
    ) -> Result<(), ()> {
        let idx = price_to_index(price, self.reference).ok_or(())?;

        self.levels[idx].add_order(handle, qty, pool);

        let idx_u32 = idx as u32;
        match self.best {
            None => self.best = Some(idx_u32),
            Some(current) if self.is_better(idx_u32, current) => self.best = Some(idx_u32),
            _ => {}
        }

        Ok(())
    }

    pub fn remove_order(
        &mut self,
        handle: OrderHandle,
        price: Price,
        qty: i64,
        pool: &mut OrderPool,
    ) {
        let idx = match price_to_index(price, self.reference) {
            Some(i) => i,
            None => return,
        };

        self.levels[idx].remove_order(handle, qty, pool);

        if self.best != Some(idx as u32) {
            return;
        }

        if !self.levels[idx].is_empty() {
            return;
        }

        self.best = self.find_next_best(idx as u32);
    }

    fn find_next_best(&self, from: u32) -> Option<u32> {
        match self.side {
            Side::Bid => {
                let mut i = from;
                while i > 0 {
                    i -= 1;
                    if !self.levels[i as usize].is_empty() {
                        return Some(i);
                    }
                }
                None
            }
            Side::Ask => {
                let mut i = from + 1;
                while (i as usize) < LADDER_SIZE {
                    if !self.levels[i as usize].is_empty() {
                        return Some(i);
                    }
                    i += 1;
                }
                None
            }
        }
    }

    pub fn best_price(&self) -> Option<Price> {
        self.best
            .map(|idx| index_to_price(idx as usize, self.reference))
    }

    pub fn best_level(&self) -> Option<&PriceLevel> {
        self.best.map(|idx| &self.levels[idx as usize])
    }

    pub fn decrease_level_qty(&mut self, price: Price, by: i64) {
        let idx = price_to_index(price, self.reference).expect("price was valid when added");
        self.levels[idx].decrease_qty(by);
    }
}

#[cfg(test)]
mod ladder_tests {
    use super::*;
    use crate::order::Order;
    use crate::types::{OrderId, Qty, SubaccountId};

    fn setup() -> (PriceLadder, OrderPool) {
        let reference = Price::new(100).unwrap();
        let ladder = PriceLadder::new(reference, Side::Bid);
        let pool = OrderPool::new(64);
        (ladder, pool)
    }

    fn place(pool: &mut OrderPool, id: u64, price_val: i64) -> (OrderHandle, Price) {
        let price = Price::new(price_val).unwrap();
        let order = Order::new(
            OrderId(id),
            SubaccountId(1),
            Side::Bid,
            price,
            Qty::new(10).unwrap(),
        );
        let handle = pool.insert(order).unwrap();
        (handle, price)
    }

    #[test]
    fn empty_ladder_has_no_best() {
        let (ladder, _) = setup();
        assert_eq!(ladder.best_price(), None);
    }

    #[test]
    fn add_single_order_sets_best() {
        let (mut ladder, mut pool) = setup();
        let (h, p) = place(&mut pool, 1, 100);
        ladder.add_order(h, p, 10, &mut pool).unwrap();
        assert_eq!(ladder.best_price(), Some(p));
    }

    #[test]
    fn bid_best_is_highest_price() {
        let (mut ladder, mut pool) = setup();
        let (h1, p1) = place(&mut pool, 1, 99);
        let (h2, p2) = place(&mut pool, 2, 101);
        let (h3, p3) = place(&mut pool, 3, 100);

        ladder.add_order(h1, p1, 10, &mut pool).unwrap();
        ladder.add_order(h2, p2, 10, &mut pool).unwrap();
        ladder.add_order(h3, p3, 10, &mut pool).unwrap();

        assert_eq!(ladder.best_price(), Some(p2)); // 101 — самая высокая
    }

    #[test]
    fn ask_best_is_lowest_price() {
        let reference = Price::new(100).unwrap();
        let mut ladder = PriceLadder::new(reference, Side::Ask);
        let mut pool = OrderPool::new(64);

        let (h1, p1) = place(&mut pool, 1, 99);
        let (h2, p2) = place(&mut pool, 2, 101);
        let (h3, p3) = place(&mut pool, 3, 100);

        ladder.add_order(h1, p1, 10, &mut pool).unwrap();
        ladder.add_order(h2, p2, 10, &mut pool).unwrap();
        ladder.add_order(h3, p3, 10, &mut pool).unwrap();

        assert_eq!(ladder.best_price(), Some(p1)); // 99 — самая низкая
    }

    #[test]
    fn out_of_range_price_rejected() {
        let (mut ladder, mut pool) = setup();
        let p = Price::new(10000).unwrap(); // далеко за границей
        let order = Order::new(
            OrderId(1),
            SubaccountId(1),
            Side::Bid,
            p,
            Qty::new(10).unwrap(),
        );
        let h = pool.insert(order).unwrap();

        assert!(ladder.add_order(h, p, 10, &mut pool).is_err());
        assert_eq!(ladder.best_price(), None);
    }

    #[test]
    fn remove_non_best_does_not_change_best() {
        let (mut ladder, mut pool) = setup();
        let (h1, p1) = place(&mut pool, 1, 99);
        let (h2, p2) = place(&mut pool, 2, 101);

        ladder.add_order(h1, p1, 10, &mut pool).unwrap();
        ladder.add_order(h2, p2, 10, &mut pool).unwrap();

        ladder.remove_order(h1, p1, 10, &mut pool); // удаляем не с best
        assert_eq!(ladder.best_price(), Some(p2));
    }

    #[test]
    fn remove_best_when_level_not_empty() {
        let (mut ladder, mut pool) = setup();
        let (h1, p) = place(&mut pool, 1, 100);
        let (h2, p2) = place(&mut pool, 2, 100); // та же цена

        ladder.add_order(h1, p, 10, &mut pool).unwrap();
        ladder.add_order(h2, p2, 10, &mut pool).unwrap();

        ladder.remove_order(h1, p, 10, &mut pool);
        assert_eq!(ladder.best_price(), Some(p)); // best не сменился
    }

    #[test]
    fn remove_best_empties_level_finds_next() {
        let (mut ladder, mut pool) = setup();
        let (h1, p1) = place(&mut pool, 1, 99);
        let (h2, p2) = place(&mut pool, 2, 101);

        ladder.add_order(h1, p1, 10, &mut pool).unwrap();
        ladder.add_order(h2, p2, 10, &mut pool).unwrap();

        ladder.remove_order(h2, p2, 10, &mut pool); // снимаем best
        assert_eq!(ladder.best_price(), Some(p1)); // следующий вниз
    }

    #[test]
    fn remove_last_order_clears_best() {
        let (mut ladder, mut pool) = setup();
        let (h, p) = place(&mut pool, 1, 100);

        ladder.add_order(h, p, 10, &mut pool).unwrap();
        ladder.remove_order(h, p, 10, &mut pool);

        assert_eq!(ladder.best_price(), None);
    }

    #[test]
    fn ask_finds_next_best_upward() {
        let reference = Price::new(100).unwrap();
        let mut ladder = PriceLadder::new(reference, Side::Ask);
        let mut pool = OrderPool::new(64);

        let (h1, p1) = place(&mut pool, 1, 100);
        let (h2, p2) = place(&mut pool, 2, 102);

        ladder.add_order(h1, p1, 10, &mut pool).unwrap();
        ladder.add_order(h2, p2, 10, &mut pool).unwrap();

        ladder.remove_order(h1, p1, 10, &mut pool); // снимаем best ask
        assert_eq!(ladder.best_price(), Some(p2)); // следующий вверх
    }
}
