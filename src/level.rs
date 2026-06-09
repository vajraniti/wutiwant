use crate::pool::{OrderHandle, OrderPool};

pub struct PriceLevel {
    head: Option<u32>,
    tail: Option<u32>,
    total_qty: i64,
    len: u32,
}

impl PriceLevel {
    pub fn new() -> Self {
        Self {
            head: None,
            tail: None,
            total_qty: 0,
            len: 0,
        }
    }

    pub fn add_order(&mut self, handle: OrderHandle, qty: i64, pool: &mut OrderPool) {
        let new_idx = handle.index();

        match self.tail {
            None => {
                self.head = Some(new_idx);
                self.tail = Some(new_idx);
            }
            Some(old_tail_idx) => {
                pool.get_by_index_mut(old_tail_idx).next = Some(new_idx);
                pool.get_by_index_mut(new_idx).prev = Some(old_tail_idx);
                self.tail = Some(new_idx);
            }
        }

        self.total_qty += qty;
        self.len += 1;
    }

    pub fn front(&self) -> Option<u32> {
        self.head
    }

    pub fn remove_order(&mut self, handle: OrderHandle, qty: i64, pool: &mut OrderPool) {
        let idx = handle.index();
        let order = pool.get_by_index(idx);
        let prev = order.prev;
        let next = order.next;

        match prev {
            Some(p) => pool.get_by_index_mut(p).next = next,
            None => self.head = next,
        }

        match next {
            Some(n) => pool.get_by_index_mut(n).prev = prev,
            None => self.tail = prev,
        }

        let order = pool.get_by_index_mut(idx);
        order.prev = None;
        order.next = None;

        self.total_qty -= qty;
        self.len -= 1;
    }

    pub fn decrease_qty(&mut self, by: i64) {
        self.total_qty -= by;
    }

    pub fn len(&self) -> u32 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn total_qty(&self) -> i64 {
        self.total_qty
    }
}
