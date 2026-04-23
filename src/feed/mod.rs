pub mod binance;
pub mod poly;

use crate::market::OrderBook;
use parking_lot::RwLock;
use ratatui::style::Color;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub type BookHandle  = Arc<RwLock<OrderBook>>;
pub type StatsHandle = Arc<RwLock<Stats>>;

/// Single-book pane (used for Binance spot pairs)
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

/// Unified Polymarket pane — holds both UP and DOWN books together
#[derive(Clone)]
pub struct PolyPane {
    pub title:         String,
    pub asset:         String,
    pub duration:      String,
    /// ISO-8601 end timestamp (e.g. "2026-04-21T20:15:00Z")
    pub end_date:      String,
    /// Chainlink reference price at window open; 0.0 = not yet set
    pub price_to_beat: f64,
    pub up_book:       BookHandle,
    pub down_book:     BookHandle,
    pub up_stats:      StatsHandle,
    pub down_stats:    StatsHandle,
    /// Live Binance BTCUSDT mid — wired in by main if user opened that pair
    pub btc_book:      Option<BookHandle>,
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
