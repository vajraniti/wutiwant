use crate::types::{OrderId, Price, Qty, Side, SubaccountId};

#[derive(Debug, Clone, Copy)]
pub struct Order {
    pub order_id: OrderId,
    pub subaccount: SubaccountId,
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
    pub remaining: i64,

    pub prev: Option<u32>,
    pub next: Option<u32>,
}

impl Order {
    pub fn new(
        order_id: OrderId,
        subaccount: SubaccountId,
        side: Side,
        price: Price,
        qty: Qty,
    ) -> Self {
        Self {
            order_id,
            subaccount,
            side,
            price,
            qty,
            remaining: qty.raw(),
            prev: None,
            next: None,
        }
    }
}
