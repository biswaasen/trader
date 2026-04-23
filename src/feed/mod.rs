pub mod binance;
pub mod poly;

use crate::market::OrderBook;
use parking_lot::RwLock;
use ratatui::style::Color;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub type BookHandle       = Arc<RwLock<OrderBook>>;
pub type StatsHandle      = Arc<RwLock<Stats>>;
/// Shared live f64 price — used for both spot price and price-to-beat
pub type SpotPriceHandle  = Arc<RwLock<f64>>;
/// Live mutable market metadata — swapped atomically on auto-advance
pub type MarketHandle     = Arc<RwLock<MarketState>>;

/// All fields that change when we roll to the next window
#[derive(Clone)]
pub struct MarketState {
    pub title:      String,
    pub asset:      String,
    pub duration:   String,
    pub end_date:   String,
    pub event_slug: String,
    pub up_id:      String,
    pub down_id:    String,
}

/// Single-book pane (Binance spot pairs)
#[derive(Clone)]
pub struct Pane {
    pub title:    String,
    pub subtitle: String,
    pub color:    Color,
    pub book:     BookHandle,
    pub stats:    StatsHandle,
    #[allow(dead_code)]
    pub group:    Option<String>,
}

/// Unified Polymarket pane — both UP and DOWN, auto-advances each window
#[derive(Clone)]
pub struct PolyPane {
    pub market:        MarketHandle,
    pub price_to_beat: SpotPriceHandle,
    pub spot_price:    SpotPriceHandle,
    pub up_book:       BookHandle,
    pub down_book:     BookHandle,
    pub up_stats:      StatsHandle,
    pub down_stats:    StatsHandle,
}

/// What the viewer renders — one column in the layout grid
#[derive(Clone)]
pub enum View {
    Exchange(Pane),
    Poly(PolyPane),
}

pub struct Stats {
    pub connected:        bool,
    pub bootstrapped:     bool,
    pub reconnects:       u32,
    pub last_error:       Option<String>,
    pub msgs:             u64,
    pub last_msg_us:      u64,
    pub last_exchange_us: u64,
    pub latency_ewma_us:  f64,
}

impl Default for Stats {
    fn default() -> Self {
        Self { connected: false, bootstrapped: false, reconnects: 0, last_error: None,
               msgs: 0, last_msg_us: 0, last_exchange_us: 0, latency_ewma_us: 0.0 }
    }
}

#[inline(always)]
pub fn now_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}
