pub const BINANCE_PX_SCALE:  u64 = 100_000_000;
pub const BINANCE_QTY_SCALE: u64 = 100_000_000;
pub const POLY_PX_SCALE:     u64 = 1_000_000;
pub const POLY_QTY_SCALE:    u64 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side { Buy, Sell }

#[derive(Clone, Copy, Debug)]
pub struct Trade {
    pub price_u: u64,
    pub qty_u:   u64,
    pub side:    Side,
    pub ts_us:   u64,
}

#[inline(always)]
pub fn parse_px(s: &str, scale: u64) -> u64 {
    let v: f64 = fast_float::parse(s).unwrap_or(0.0);
    (v * scale as f64 + 0.5) as u64
}
