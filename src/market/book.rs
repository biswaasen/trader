use std::collections::BTreeMap;
use super::parse::Trade;

pub struct OrderBook {
    pub bids:           BTreeMap<u64, u64>,
    pub asks:           BTreeMap<u64, u64>,
    pub px_scale:       u64,
    pub qty_scale:      u64,
    pub seq:            u64,
    pub updates:        u64,
    pub last_update_us: u64,
    pub last_trade:     Option<Trade>,
}

impl OrderBook {
    pub fn new(px_scale: u64, qty_scale: u64) -> Self {
        Self { bids: BTreeMap::new(), asks: BTreeMap::new(), px_scale, qty_scale,
               seq: 0, updates: 0, last_update_us: 0, last_trade: None }
    }

    #[inline(always)] pub fn clear(&mut self) { self.bids.clear(); self.asks.clear(); }

    #[inline(always)]
    pub fn apply_bid(&mut self, px: u64, qty: u64) {
        if qty == 0 { self.bids.remove(&px); } else { self.bids.insert(px, qty); }
    }
    #[inline(always)]
    pub fn apply_ask(&mut self, px: u64, qty: u64) {
        if qty == 0 { self.asks.remove(&px); } else { self.asks.insert(px, qty); }
    }

    #[inline] pub fn best_bid(&self) -> Option<(u64, u64)> { self.bids.iter().next_back().map(|(p,q)|(*p,*q)) }
    #[inline] pub fn best_ask(&self) -> Option<(u64, u64)> { self.asks.iter().next().map(|(p,q)|(*p,*q)) }

    pub fn mid_f64(&self) -> Option<f64> {
        let (b,_) = self.best_bid()?; let (a,_) = self.best_ask()?;
        Some(((b + a) as f64 * 0.5) / self.px_scale as f64)
    }
    pub fn spread_bps(&self) -> Option<f64> {
        let (b,_) = self.best_bid()?; let (a,_) = self.best_ask()?;
        if a <= b { return None; }
        Some((a - b) as f64 / ((a + b) as f64 * 0.5) * 10_000.0)
    }
    pub fn imbalance_top(&self, n: usize) -> Option<f64> {
        let b: u64 = self.bids.iter().rev().take(n).map(|(_,q)|*q).sum();
        let a: u64 = self.asks.iter().take(n).map(|(_,q)|*q).sum();
        let s = b + a;
        if s == 0 { None } else { Some((b as f64 - a as f64) / s as f64) }
    }

    pub fn top_bids(&self, n: usize) -> Vec<(u64,u64)> { self.bids.iter().rev().take(n).map(|(p,q)|(*p,*q)).collect() }
    pub fn top_asks(&self, n: usize) -> Vec<(u64,u64)> { self.asks.iter().take(n).map(|(p,q)|(*p,*q)).collect() }
}
