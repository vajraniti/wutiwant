#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Price(i64);

impl Price {
    pub fn new(value: i64) -> Option<Self> {
        if value > 0 { Some(Price(value)) } else { None }
    }

    pub fn raw(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Qty(i64);

impl Qty {
    pub fn new(value: i64) -> Option<Self> {
        if value > 0 { Some(Qty(value)) } else { None }
    }

    pub fn raw(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubaccountId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Bid,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfTradeMode {
    ExpireTaker,
    ExpireMaker,
    ExpireBoth,
}
