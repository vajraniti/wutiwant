use crate::types::{OrderId, Price, Qty, SelfTradeMode, Side, SubaccountId};

pub enum Op {
    Place {
        order_id: OrderId,
        subaccount: SubaccountId,
        side: Side,
        price: Price,
        qty: Qty,
        stp: SelfTradeMode,
    },

    Cancel {
        order_id: OrderId,
        subaccount: SubaccountId,
    },
}
