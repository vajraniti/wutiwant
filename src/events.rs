use crate::types::{OrderId, Price, Qty, Side, SubaccountId};

pub enum Event {
    Accepted {
        order_id: OrderId,
        subaccount: SubaccountId,
        side: Side,
        price: Price,
        qty: Qty,
    },
    Trade {
        maker_order_id: OrderId,
        taker_order_id: OrderId,
        maker_subaccount: SubaccountId,
        taker_subaccount: SubaccountId,
        price: Price,
        qty: Qty,
    },
    Cancelled {
        order_id: OrderId,
        subaccount: SubaccountId,
        remaining_qty: Qty,
    },
    Rejected {
        order_id: OrderId,
        reason: RejectReason,
    },
}

pub enum RejectReason {
    DuplicateOrderId,
    PriceOutOfRange,
    SelfTradePrevention,
    OrderNotFound,
    NotOwner,
}
