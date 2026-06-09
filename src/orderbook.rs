use std::collections::HashMap;

use crate::events::{Event, RejectReason};
use crate::ladder::PriceLadder;
use crate::ops::Op;
use crate::order::Order;
use crate::pool::{OrderHandle, OrderPool};
use crate::types::{OrderId, Price, Qty, SelfTradeMode, Side, SubaccountId};

pub(crate) struct MatchResult {
    pub remaining_qty: i64,
    pub stp_triggered: bool,
}

pub struct OrderBook {
    pub(crate) pool: OrderPool,
    pub(crate) bids: PriceLadder,
    pub(crate) asks: PriceLadder,
    pub(crate) id_index: HashMap<OrderId, OrderHandle>,
}

impl OrderBook {
    pub fn new(reference: Price, capacity: usize) -> Self {
        Self {
            pool: OrderPool::new(capacity),
            bids: PriceLadder::new(reference, Side::Bid),
            asks: PriceLadder::new(reference, Side::Ask),
            id_index: HashMap::with_capacity(capacity),
        }
    }

    /// Лучшая цена бида (самая высокая), либо `None` если бидов нет.
    pub fn best_bid(&self) -> Option<Price> {
        self.bids.best_price()
    }

    /// Лучшая цена аска (самая низкая), либо `None` если асков нет.
    pub fn best_ask(&self) -> Option<Price> {
        self.asks.best_price()
    }

    /// Суммарный объём на лучшем биде (0, если бидов нет).
    pub fn best_bid_qty(&self) -> i64 {
        self.bids.best_level().map_or(0, |l| l.total_qty())
    }

    /// Суммарный объём на лучшем аске (0, если асков нет).
    pub fn best_ask_qty(&self) -> i64 {
        self.asks.best_level().map_or(0, |l| l.total_qty())
    }

    pub fn apply(&mut self, op: Op, events: &mut Vec<Event>) {
        match op {
            Op::Place {
                order_id,
                subaccount,
                side,
                price,
                qty,
                stp,
            } => self.place(order_id, subaccount, side, price, qty, stp, events),

            Op::Cancel {
                order_id,
                subaccount,
            } => self.cancel(order_id, subaccount, events),
        }
    }

    fn place(
        &mut self,
        order_id: OrderId,
        subaccount: SubaccountId,
        side: Side,
        price: Price,
        qty: Qty,
        stp: SelfTradeMode,
        events: &mut Vec<Event>,
    ) {
        if self.id_index.contains_key(&order_id) {
            events.push(Event::Rejected {
                order_id,
                reason: RejectReason::DuplicateOrderId,
            });
            return;
        }

        events.push(Event::Accepted {
            order_id,
            subaccount,
            side,
            price,
            qty,
        });

        let match_result = match side {
            Side::Bid => self.match_buy(order_id, subaccount, price, qty, stp, events),
            Side::Ask => self.match_sell(order_id, subaccount, price, qty, stp, events),
        };

        if match_result.stp_triggered || match_result.remaining_qty == 0 {
            return;
        }

        let remaining_qty = Qty::new(match_result.remaining_qty).unwrap();
        let mut order = Order::new(order_id, subaccount, side, price, qty);
        order.remaining = match_result.remaining_qty;

        let handle = match self.pool.insert(order) {
            Some(h) => h,
            None => {
                events.push(Event::Cancelled {
                    order_id,
                    subaccount,
                    remaining_qty,
                });
                return;
            }
        };

        let result = match side {
            Side::Bid => {
                self.bids
                    .add_order(handle, price, match_result.remaining_qty, &mut self.pool)
            }
            Side::Ask => {
                self.asks
                    .add_order(handle, price, match_result.remaining_qty, &mut self.pool)
            }
        };

        if result.is_err() {
            self.pool.remove(handle);
            events.push(Event::Rejected {
                order_id,
                reason: RejectReason::PriceOutOfRange,
            });
            return;
        }

        self.id_index.insert(order_id, handle);
    }

    fn cancel(&mut self, order_id: OrderId, subaccount: SubaccountId, events: &mut Vec<Event>) {
        let handle = match self.id_index.get(&order_id) {
            Some(&h) => h,
            None => {
                events.push(Event::Rejected {
                    order_id,
                    reason: RejectReason::OrderNotFound,
                });
                return;
            }
        };

        let order = match self.pool.get(handle) {
            Some(o) => *o,
            None => {
                events.push(Event::Rejected {
                    order_id,
                    reason: RejectReason::OrderNotFound,
                });
                return;
            }
        };

        if order.subaccount != subaccount {
            events.push(Event::Rejected {
                order_id,
                reason: RejectReason::NotOwner,
            });
            return;
        }

        match order.side {
            Side::Bid => {
                self.bids
                    .remove_order(handle, order.price, order.remaining, &mut self.pool)
            }
            Side::Ask => {
                self.asks
                    .remove_order(handle, order.price, order.remaining, &mut self.pool)
            }
        }

        self.pool.remove(handle);
        self.id_index.remove(&order_id);

        events.push(Event::Cancelled {
            order_id,
            subaccount,
            remaining_qty: Qty::new(order.remaining).unwrap(),
        });
    }

    pub(crate) fn match_buy(
        &mut self,
        taker_id: OrderId,
        taker_subaccount: SubaccountId,
        taker_price: Price,
        taker_qty: Qty,
        stp: SelfTradeMode,
        events: &mut Vec<Event>,
    ) -> MatchResult {
        let mut remaining = taker_qty.raw();

        loop {
            if remaining == 0 {
                break;
            }

            let best_ask_price = match self.asks.best_price() {
                Some(p) => p,
                None => break,
            };

            if best_ask_price.raw() > taker_price.raw() {
                break;
            }

            let level = self
                .asks
                .best_level()
                .expect("best_price set, level must exist");
            let head_idx = match level.front() {
                Some(idx) => idx,
                None => unreachable!("best points to non-empty level"),
            };

            let maker = *self.pool.get_by_index(head_idx);

            if maker.subaccount == taker_subaccount {
                match stp {
                    SelfTradeMode::ExpireTaker => {
                        events.push(Event::Cancelled {
                            order_id: taker_id,
                            subaccount: taker_subaccount,
                            remaining_qty: Qty::new(remaining).unwrap(),
                        });
                        return MatchResult {
                            remaining_qty: 0,
                            stp_triggered: true,
                        };
                    }
                    SelfTradeMode::ExpireMaker => {
                        self.cancel_maker_in_place_ask(&maker, events);
                        continue;
                    }
                    SelfTradeMode::ExpireBoth => {
                        self.cancel_maker_in_place_ask(&maker, events);
                        events.push(Event::Cancelled {
                            order_id: taker_id,
                            subaccount: taker_subaccount,
                            remaining_qty: Qty::new(remaining).unwrap(),
                        });
                        return MatchResult {
                            remaining_qty: 0,
                            stp_triggered: true,
                        };
                    }
                }
            }

            let fill_qty = remaining.min(maker.remaining);

            events.push(Event::Trade {
                maker_order_id: maker.order_id,
                taker_order_id: taker_id,
                maker_subaccount: maker.subaccount,
                taker_subaccount,
                price: maker.price,
                qty: Qty::new(fill_qty).unwrap(),
            });

            remaining -= fill_qty;
            let new_maker_remaining = maker.remaining - fill_qty;

            if new_maker_remaining == 0 {
                let handle = *self
                    .id_index
                    .get(&maker.order_id)
                    .expect("maker in id_index");
                self.asks
                    .remove_order(handle, maker.price, maker.remaining, &mut self.pool);
                self.pool.remove(handle);
                self.id_index.remove(&maker.order_id);
            } else {
                let m = self.pool.get_by_index_mut(head_idx);
                m.remaining = new_maker_remaining;
                self.asks.decrease_level_qty(maker.price, fill_qty);
            }
        }

        MatchResult {
            remaining_qty: remaining,
            stp_triggered: false,
        }
    }

    pub(crate) fn match_sell(
        &mut self,
        taker_id: OrderId,
        taker_subaccount: SubaccountId,
        taker_price: Price,
        taker_qty: Qty,
        stp: SelfTradeMode,
        events: &mut Vec<Event>,
    ) -> MatchResult {
        let mut remaining = taker_qty.raw();

        loop {
            if remaining == 0 {
                break;
            }

            let best_bid_price = match self.bids.best_price() {
                Some(p) => p,
                None => break,
            };

            if best_bid_price.raw() < taker_price.raw() {
                break;
            }

            let level = self
                .bids
                .best_level()
                .expect("best_price set, level must exist");
            let head_idx = match level.front() {
                Some(idx) => idx,
                None => unreachable!("best points to non-empty level"),
            };

            let maker = *self.pool.get_by_index(head_idx);

            if maker.subaccount == taker_subaccount {
                match stp {
                    SelfTradeMode::ExpireTaker => {
                        events.push(Event::Cancelled {
                            order_id: taker_id,
                            subaccount: taker_subaccount,
                            remaining_qty: Qty::new(remaining).unwrap(),
                        });
                        return MatchResult {
                            remaining_qty: 0,
                            stp_triggered: true,
                        };
                    }
                    SelfTradeMode::ExpireMaker => {
                        self.cancel_maker_in_place_bid(&maker, events);
                        continue;
                    }
                    SelfTradeMode::ExpireBoth => {
                        self.cancel_maker_in_place_bid(&maker, events);
                        events.push(Event::Cancelled {
                            order_id: taker_id,
                            subaccount: taker_subaccount,
                            remaining_qty: Qty::new(remaining).unwrap(),
                        });
                        return MatchResult {
                            remaining_qty: 0,
                            stp_triggered: true,
                        };
                    }
                }
            }

            let fill_qty = remaining.min(maker.remaining);

            events.push(Event::Trade {
                maker_order_id: maker.order_id,
                taker_order_id: taker_id,
                maker_subaccount: maker.subaccount,
                taker_subaccount,
                price: maker.price,
                qty: Qty::new(fill_qty).unwrap(),
            });

            remaining -= fill_qty;
            let new_maker_remaining = maker.remaining - fill_qty;

            if new_maker_remaining == 0 {
                let handle = *self
                    .id_index
                    .get(&maker.order_id)
                    .expect("maker in id_index");
                self.bids
                    .remove_order(handle, maker.price, maker.remaining, &mut self.pool);
                self.pool.remove(handle);
                self.id_index.remove(&maker.order_id);
            } else {
                let m = self.pool.get_by_index_mut(head_idx);
                m.remaining = new_maker_remaining;
                self.bids.decrease_level_qty(maker.price, fill_qty);
            }
        }

        MatchResult {
            remaining_qty: remaining,
            stp_triggered: false,
        }
    }

    fn cancel_maker_in_place_ask(&mut self, maker: &Order, events: &mut Vec<Event>) {
        let handle = *self
            .id_index
            .get(&maker.order_id)
            .expect("maker in id_index");
        self.asks
            .remove_order(handle, maker.price, maker.remaining, &mut self.pool);
        self.pool.remove(handle);
        self.id_index.remove(&maker.order_id);

        events.push(Event::Cancelled {
            order_id: maker.order_id,
            subaccount: maker.subaccount,
            remaining_qty: Qty::new(maker.remaining).unwrap(),
        });
    }

    fn cancel_maker_in_place_bid(&mut self, maker: &Order, events: &mut Vec<Event>) {
        let handle = *self
            .id_index
            .get(&maker.order_id)
            .expect("maker in id_index");
        self.bids
            .remove_order(handle, maker.price, maker.remaining, &mut self.pool);
        self.pool.remove(handle);
        self.id_index.remove(&maker.order_id);

        events.push(Event::Cancelled {
            order_id: maker.order_id,
            subaccount: maker.subaccount,
            remaining_qty: Qty::new(maker.remaining).unwrap(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place_op(id: u64, side: Side, price_val: i64, qty_val: i64) -> Op {
        place_op_stp(id, side, price_val, qty_val, 1, SelfTradeMode::ExpireTaker)
    }

    fn place_op_sub(
        id: u64,
        sub: u64,
        side: Side,
        price_val: i64,
        qty_val: i64,
        stp: SelfTradeMode,
    ) -> Op {
        place_op_stp(id, side, price_val, qty_val, sub, stp)
    }

    fn place_op_stp(
        id: u64,
        side: Side,
        price_val: i64,
        qty_val: i64,
        sub: u64,
        stp: SelfTradeMode,
    ) -> Op {
        Op::Place {
            order_id: OrderId(id),
            subaccount: SubaccountId(sub),
            side,
            price: Price::new(price_val).unwrap(),
            qty: Qty::new(qty_val).unwrap(),
            stp,
        }
    }

    fn cancel_op(id: u64) -> Op {
        Op::Cancel {
            order_id: OrderId(id),
            subaccount: SubaccountId(1),
        }
    }

    fn book() -> OrderBook {
        OrderBook::new(Price::new(100).unwrap(), 64)
    }

    fn count_trades(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, Event::Trade { .. }))
            .count()
    }

    fn total_traded_qty(events: &[Event]) -> i64 {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Trade { qty, .. } => Some(qty.raw()),
                _ => None,
            })
            .sum()
    }

    // ============== existing tests (Phase 4) ==============

    #[test]
    fn place_emits_accepted() {
        let mut book = book();
        let mut events = Vec::new();
        book.apply(place_op(1, Side::Bid, 100, 10), &mut events);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Accepted { .. }));
    }

    #[test]
    fn duplicate_order_id_rejected() {
        let mut book = book();
        let mut events = Vec::new();
        book.apply(place_op(1, Side::Bid, 100, 10), &mut events);
        book.apply(place_op(1, Side::Bid, 101, 10), &mut events);
        assert!(matches!(
            events[1],
            Event::Rejected {
                reason: RejectReason::DuplicateOrderId,
                ..
            }
        ));
    }

    #[test]
    fn cancel_removes_order() {
        let mut book = book();
        let mut events = Vec::new();
        book.apply(place_op(1, Side::Bid, 100, 10), &mut events);
        book.apply(cancel_op(1), &mut events);
        assert!(matches!(events[1], Event::Cancelled { .. }));
        assert_eq!(book.bids.best_price(), None);
    }

    #[test]
    fn bid_and_ask_independent() {
        let mut book = book();
        let mut events = Vec::new();
        book.apply(place_op(1, Side::Bid, 99, 10), &mut events);
        book.apply(place_op(2, Side::Ask, 101, 10), &mut events);
        assert_eq!(book.bids.best_price(), Some(Price::new(99).unwrap()));
        assert_eq!(book.asks.best_price(), Some(Price::new(101).unwrap()));
    }

    // ============== Phase 5: matching ==============

    #[test]
    fn no_cross_no_trade() {
        let mut book = book();
        let mut events = Vec::new();
        book.apply(place_op(1, Side::Ask, 105, 10), &mut events);
        book.apply(place_op(2, Side::Bid, 100, 10), &mut events);

        assert_eq!(count_trades(&events), 0);
        assert_eq!(book.bids.best_price(), Some(Price::new(100).unwrap()));
        assert_eq!(book.asks.best_price(), Some(Price::new(105).unwrap()));
    }

    #[test]
    fn full_fill_taker_buy() {
        let mut book = book();
        let mut events = Vec::new();

        book.apply(
            place_op_sub(1, 1, Side::Ask, 105, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        book.apply(
            place_op_sub(2, 2, Side::Bid, 105, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );

        assert!(matches!(events[0], Event::Accepted { .. }));
        assert_eq!(count_trades(&events), 1);
        assert_eq!(total_traded_qty(&events), 10);
        assert_eq!(book.asks.best_price(), None);
        assert_eq!(book.bids.best_price(), None); // taker полностью съел, в книгу не встал
    }

    #[test]
    fn partial_fill_taker_remainder_in_book() {
        let mut book = book();
        let mut events = Vec::new();

        book.apply(
            place_op_sub(1, 1, Side::Ask, 105, 7, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        book.apply(
            place_op_sub(2, 2, Side::Bid, 105, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );

        assert_eq!(total_traded_qty(&events), 7);
        // остаток 3 встаёт в bids
        assert_eq!(book.bids.best_price(), Some(Price::new(105).unwrap()));
        assert_eq!(book.asks.best_price(), None);
    }

    #[test]
    fn partial_fill_maker_remains_with_decreased_qty() {
        let mut book = book();
        let mut events = Vec::new();

        book.apply(
            place_op_sub(1, 1, Side::Ask, 105, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        book.apply(
            place_op_sub(2, 2, Side::Bid, 105, 3, SelfTradeMode::ExpireTaker),
            &mut events,
        );

        assert_eq!(total_traded_qty(&events), 3);
        // maker остался в книге с remaining = 7
        assert_eq!(book.asks.best_price(), Some(Price::new(105).unwrap()));
        let level = book.asks.best_level().unwrap();
        assert_eq!(level.total_qty(), 7);
        assert_eq!(level.len(), 1);
    }

    #[test]
    fn walk_multiple_levels() {
        let mut book = book();
        let mut events = Vec::new();

        // три уровня asks: 101, 102, 103 — по 5 каждый
        book.apply(
            place_op_sub(1, 1, Side::Ask, 101, 5, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        book.apply(
            place_op_sub(2, 1, Side::Ask, 102, 5, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        book.apply(
            place_op_sub(3, 1, Side::Ask, 103, 5, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        // taker buy на 12 по цене 103 → съедает 5+5+2, на 102 остаётся 0, на 103 остаётся 3
        book.apply(
            place_op_sub(99, 2, Side::Bid, 103, 12, SelfTradeMode::ExpireTaker),
            &mut events,
        );

        assert_eq!(count_trades(&events), 3);
        assert_eq!(total_traded_qty(&events), 12);
        assert_eq!(book.asks.best_price(), Some(Price::new(103).unwrap()));
        assert_eq!(book.asks.best_level().unwrap().total_qty(), 3);
    }

    #[test]
    fn fifo_at_same_level() {
        let mut book = book();
        let mut events = Vec::new();

        // два ордера на одной цене — order 1 пришёл первым
        book.apply(
            place_op_sub(1, 1, Side::Ask, 105, 5, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        book.apply(
            place_op_sub(2, 1, Side::Ask, 105, 5, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        // taker buy на 7 → ордер 1 полностью, ордер 2 частично
        book.apply(
            place_op_sub(99, 2, Side::Bid, 105, 7, SelfTradeMode::ExpireTaker),
            &mut events,
        );

        let trades: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Trade {
                    maker_order_id,
                    qty,
                    ..
                } => Some((*maker_order_id, qty.raw())),
                _ => None,
            })
            .collect();

        assert_eq!(trades, vec![(OrderId(1), 5), (OrderId(2), 2)]);
    }

    #[test]
    fn trade_price_is_maker_price() {
        let mut book = book();
        let mut events = Vec::new();

        // maker по 102, taker готов платить 105
        book.apply(
            place_op_sub(1, 1, Side::Ask, 102, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        book.apply(
            place_op_sub(2, 2, Side::Bid, 105, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );

        let trade_price = events.iter().find_map(|e| match e {
            Event::Trade { price, .. } => Some(*price),
            _ => None,
        });
        assert_eq!(trade_price, Some(Price::new(102).unwrap()));
    }

    #[test]
    fn match_sell_works_symmetrically() {
        let mut book = book();
        let mut events = Vec::new();

        book.apply(
            place_op_sub(1, 1, Side::Bid, 100, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        book.apply(
            place_op_sub(2, 2, Side::Ask, 100, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );

        assert_eq!(count_trades(&events), 1);
        assert_eq!(total_traded_qty(&events), 10);
        assert_eq!(book.bids.best_price(), None);
    }

    // ============== Phase 5: STP ==============

    #[test]
    fn stp_expire_taker_cancels_taker() {
        let mut book = book();
        let mut events = Vec::new();

        book.apply(
            place_op_sub(1, 42, Side::Ask, 100, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        // тот же subaccount 42 присылает buy
        book.apply(
            place_op_sub(2, 42, Side::Bid, 100, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );

        assert_eq!(count_trades(&events), 0);
        // maker остался стоять
        assert_eq!(book.asks.best_price(), Some(Price::new(100).unwrap()));
        // taker отменён
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Cancelled {
                order_id: OrderId(2),
                ..
            }
        )));
    }

    #[test]
    fn stp_expire_maker_cancels_maker_continues() {
        let mut book = book();
        let mut events = Vec::new();

        // свой ордер на 100
        book.apply(
            place_op_sub(1, 42, Side::Ask, 100, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        // чужой ордер на 101
        book.apply(
            place_op_sub(2, 99, Side::Ask, 101, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        // taker buy на 101 с ExpireMaker — должен отменить свой maker (1) и матчиться с чужим (2)
        book.apply(
            place_op_sub(3, 42, Side::Bid, 101, 5, SelfTradeMode::ExpireMaker),
            &mut events,
        );

        // maker 1 отменён
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Cancelled {
                order_id: OrderId(1),
                ..
            }
        )));
        // matched с maker 2 на 5
        assert_eq!(total_traded_qty(&events), 5);
        // maker 2 частично съеден, остаток 5
        assert_eq!(book.asks.best_price(), Some(Price::new(101).unwrap()));
        assert_eq!(book.asks.best_level().unwrap().total_qty(), 5);
    }

    #[test]
    fn stp_expire_both_cancels_both() {
        let mut book = book();
        let mut events = Vec::new();

        book.apply(
            place_op_sub(1, 42, Side::Ask, 100, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        book.apply(
            place_op_sub(2, 42, Side::Bid, 100, 10, SelfTradeMode::ExpireBoth),
            &mut events,
        );

        assert_eq!(count_trades(&events), 0);
        // оба отменены
        let cancelled: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Cancelled { order_id, .. } => Some(*order_id),
                _ => None,
            })
            .collect();
        assert!(cancelled.contains(&OrderId(1)));
        assert!(cancelled.contains(&OrderId(2)));
        // книга пустая
        assert_eq!(book.asks.best_price(), None);
        assert_eq!(book.bids.best_price(), None);
    }

    #[test]
    fn cross_subaccount_no_stp_normal_trade() {
        let mut book = book();
        let mut events = Vec::new();

        // sub 1 ставит ask, sub 2 присылает bid — STP не должен срабатывать
        book.apply(
            place_op_sub(1, 1, Side::Ask, 100, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );
        events.clear();

        book.apply(
            place_op_sub(2, 2, Side::Bid, 100, 10, SelfTradeMode::ExpireTaker),
            &mut events,
        );

        assert_eq!(count_trades(&events), 1);
        assert_eq!(total_traded_qty(&events), 10);
    }
}
