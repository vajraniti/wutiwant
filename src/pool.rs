use crate::order::Order;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderHandle {
    index: u32,
    generation: u32,
}
impl OrderHandle {
    pub(crate) fn index(self) -> u32 {
        self.index
    }
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    generation: u32,
    state: SlotState,
}

#[derive(Debug, Clone, Copy)]
enum SlotState {
    Free { next_free: Option<u32> },
    Occupied { order: Order },
}

pub struct OrderPool {
    slots: Vec<Slot>,
    free_head: Option<u32>,
}

impl OrderPool {
    pub fn new(capacity: usize) -> Self {
        let mut slots = Vec::with_capacity(capacity);

        for i in 0..capacity {
            let next_free = if i + 1 < capacity {
                Some((i + 1) as u32)
            } else {
                None
            };
            slots.push(Slot {
                generation: 0,
                state: SlotState::Free { next_free },
            });
        }
        let free_head = if capacity > 0 { Some(0) } else { None };

        Self { slots, free_head }
    }
    pub fn insert(&mut self, order: Order) -> Option<OrderHandle> {
        let index = self.free_head?;
        let slot = &mut self.slots[index as usize];

        let next_free = match slot.state {
            SlotState::Free { next_free } => next_free,
            SlotState::Occupied { .. } => unreachable!("free_head pointed to the occupied slot"),
        };

        slot.generation = slot.generation.wrapping_add(1);
        if slot.generation == 0 {
            slot.generation = 1;
        }

        slot.state = SlotState::Occupied { order };
        self.free_head = next_free;

        Some(OrderHandle {
            index,
            generation: slot.generation,
        })
    }
    pub fn remove(&mut self, handle: OrderHandle) -> Option<Order> {
        let slot = self.slots.get_mut(handle.index as usize)?;

        if slot.generation != handle.generation {
            return None;
        }

        let order = match slot.state {
            SlotState::Occupied { order } => order,
            SlotState::Free { .. } => return None,
        };

        slot.state = SlotState::Free {
            next_free: self.free_head,
        };
        self.free_head = Some(handle.index);

        Some(order)
    }
    pub fn get(&self, handle: OrderHandle) -> Option<&Order> {
        let slot = self.slots.get(handle.index as usize)?;

        if slot.generation != handle.generation {
            return None;
        }

        match &slot.state {
            SlotState::Occupied { order } => Some(order),
            SlotState::Free { .. } => None,
        }
    }
    pub fn get_mut(&mut self, handle: OrderHandle) -> Option<&mut Order> {
        let slot = self.slots.get_mut(handle.index as usize)?;

        if slot.generation != handle.generation {
            return None;
        }

        match &mut slot.state {
            SlotState::Occupied { order } => Some(order),
            SlotState::Free { .. } => None,
        }
    }
    pub(crate) fn get_by_index(&self, index: u32) -> &Order {
        match &self.slots[index as usize].state {
            SlotState::Occupied { order } => order,
            SlotState::Free { .. } => unreachable!("intrusive list pointed to free slot"),
        }
    }
    pub(crate) fn get_by_index_mut(&mut self, index: u32) -> &mut Order {
        match &mut self.slots[index as usize].state {
            SlotState::Occupied { order } => order,
            SlotState::Free { .. } => unreachable!("intrusive list pointed to free slot"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OrderId, Price, Qty, Side, SubaccountId};

    fn make_order(id: u64) -> Order {
        Order::new(
            OrderId(id),
            SubaccountId(1),
            Side::Bid,
            Price::new(100).unwrap(),
            Qty::new(10).unwrap(),
        )
    }

    #[test]
    fn insert_returns_handle() {
        let mut pool = OrderPool::new(4);
        let handle = pool.insert(make_order(1));
        assert!(handle.is_some());
    }

    #[test]
    fn get_returns_inserted_order() {
        let mut pool = OrderPool::new(4);
        let handle = pool.insert(make_order(42)).unwrap();

        let order = pool.get(handle).unwrap();
        assert_eq!(order.order_id, OrderId(42));
    }

    #[test]
    fn remove_returns_order_and_invalidates_handle() {
        let mut pool = OrderPool::new(4);
        let handle = pool.insert(make_order(1)).unwrap();

        let removed = pool.remove(handle);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().order_id, OrderId(1));

        // handle протух
        assert!(pool.get(handle).is_none());
    }

    #[test]
    fn pool_full_returns_none() {
        let mut pool = OrderPool::new(2);
        assert!(pool.insert(make_order(1)).is_some());
        assert!(pool.insert(make_order(2)).is_some());
        assert!(pool.insert(make_order(3)).is_none()); // pool полный
    }

    #[test]
    fn slot_is_reused_after_remove() {
        let mut pool = OrderPool::new(2);
        let h1 = pool.insert(make_order(1)).unwrap();
        let _h2 = pool.insert(make_order(2)).unwrap();

        // pool полный
        assert!(pool.insert(make_order(3)).is_none());

        // удалили один — слот должен переиспользоваться
        pool.remove(h1).unwrap();
        let h3 = pool.insert(make_order(3));
        assert!(h3.is_some());
    }

    #[test]
    fn generation_protects_from_use_after_free() {
        let mut pool = OrderPool::new(2);
        let old_handle = pool.insert(make_order(1)).unwrap();

        pool.remove(old_handle).unwrap();
        let _new_handle = pool.insert(make_order(2)).unwrap();

        // старый handle указывает на тот же слот, но в нём теперь другой ордер
        // generation не совпадёт → None
        assert!(pool.get(old_handle).is_none());
    }

    #[test]
    fn get_mut_allows_mutation() {
        let mut pool = OrderPool::new(2);
        let handle = pool.insert(make_order(1)).unwrap();

        let order = pool.get_mut(handle).unwrap();
        order.remaining = 5;

        assert_eq!(pool.get(handle).unwrap().remaining, 5);
    }

    #[test]
    fn remove_twice_returns_none() {
        let mut pool = OrderPool::new(2);
        let handle = pool.insert(make_order(1)).unwrap();

        pool.remove(handle).unwrap();
        assert!(pool.remove(handle).is_none()); // второй remove — None
    }
}
