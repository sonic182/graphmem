use std::fmt;

pub mod billing {
    pub fn charge(amount: u64) -> u64 {
        amount
    }
}

pub struct Invoice {
    pub total: u64,
}

impl Invoice {
    pub fn new(total: u64) -> Self {
        Self { total }
    }
}

impl fmt::Display for Invoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.total)
    }
}

pub trait Priced {
    fn price(&self) -> u64;

    fn discounted(&self) -> u64 {
        self.price() / 2
    }
}

pub enum Status {
    Draft,
    Paid,
}
