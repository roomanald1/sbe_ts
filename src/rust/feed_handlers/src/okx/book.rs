use anyhow::{Context, Result, anyhow, ensure};

const PRICE_SCALE: u128 = 1_000_000_000_000_000_000;

pub struct Snapshot<'a> {
    pub asks: Vec<Vec<&'a str>>,
    pub bids: Vec<Vec<&'a str>>,
    pub ts: &'a str,
    pub seq: i64,
    pub prev_seq: i64,
}

pub struct Update<'a> {
    pub asks: Vec<Vec<&'a str>>,
    pub bids: Vec<Vec<&'a str>>,
    pub ts: &'a str,
    pub seq: i64,
    pub prev_seq: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookLevel {
    pub price: Price,
    pub size: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Price(u128);

include!("book_core.rs");
include!("book_level_updates.rs");
include!("book_decimal.rs");
#[cfg(test)]
include!("book_tests.rs");
