use super::Feed;
use crate::feed::now_us;
use crate::market::{parse_px, Side, Trade, POLY_PX_SCALE, POLY_QTY_SCALE};
use anyhow::Result;
use serde::Deserialize;
use std::sync::atomic::{AtomicU32, Ordering};

// ── debug capture ─────────────────────────────────────────────────────────────
// Writes the first 60 raw CLOB frames to /tmp/poly_debug.log so we can
// inspect the exact field names Polymarket sends. Remove once confirmed.
static DEBUG_N: AtomicU32 = AtomicU32::new(0);

fn debug_log(raw: &str) {
    let n = DEBUG_N.fetch_add(1, Ordering::Relaxed);
    if n >= 60 { return; }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/poly_debug.log") {
        let _ = writeln!(f, "=== msg {n} ===\n{raw}\n");
    }
}

// ── public entry point ────────────────────────────────────────────────────────

pub fn handle(text: &str, f: &Feed) -> Result<()> {
    debug_log(text);
    let first = text.bytes().find(|c| !c.is_ascii_whitespace()).unwrap_or(0);
    if first == b'[' {
        if let Ok(evs) = serde_json::from_str::<Vec<Ev>>(text) {
            for ev in evs { dispatch(ev, f); }
        } else {
            debug_log(&format!("PARSE_FAIL_ARRAY: {text}"));
        }
    } else if first == b'{' {
        if let Ok(ev) = serde_json::from_str::<Ev>(text) {
            dispatch(ev, f);
        } else {
            debug_log(&format!("PARSE_FAIL_OBJ: {text}"));
        }
    }
    Ok(())
}

fn dispatch(ev: Ev, f: &Feed) {
    let (book, stats) = if ev.asset_id.as_deref() == Some(f.up_id.as_str()) {
        (&f.up_book, &f.up_stats)
    } else if ev.asset_id.as_deref() == Some(f.down_id.as_str()) {
        (&f.down_book, &f.down_stats)
    } else { return };

    match ev.event_type.as_deref().unwrap_or("") {
        "book" => {
            let mut b = book.write();
            b.bids.clear(); b.asks.clear();
            // Polymarket may send "buys"/"sells" or "bids"/"asks" — aliases handle both
            for o in ev.buys.iter().flatten()  { b.apply_bid(parse_px(&o.price.str(), POLY_PX_SCALE), parse_px(&o.size.str(), POLY_QTY_SCALE)); }
            for o in ev.sells.iter().flatten() { b.apply_ask(parse_px(&o.price.str(), POLY_PX_SCALE), parse_px(&o.size.str(), POLY_QTY_SCALE)); }
            b.updates += 1; b.last_update_us = now_us();
        }
        "price_change" => {
            let mut b = book.write();
            for c in ev.changes.iter().flatten() {
                let (px, qty) = (parse_px(&c.price.str(), POLY_PX_SCALE), parse_px(&c.size.str(), POLY_QTY_SCALE));
                match c.side.as_deref().unwrap_or("") {
                    "BUY"  => b.apply_bid(px, qty),
                    "SELL" => b.apply_ask(px, qty),
                    _ => {}
                }
            }
            b.updates += 1; b.last_update_us = now_us();
        }
        "last_trade_price" => {
            if let (Some(p), Some(q)) = (&ev.price, &ev.size) {
                book.write().last_trade = Some(Trade {
                    price_u: parse_px(&p.str(), POLY_PX_SCALE),
                    qty_u:   parse_px(&q.str(), POLY_QTY_SCALE),
                    side:    if ev.side.as_deref() == Some("SELL") { Side::Sell } else { Side::Buy },
                    ts_us:   ev.timestamp.as_ref()
                                .map(|n| (n.f64() * 1_000.0) as u64)
                                .unwrap_or_else(now_us),
                });
            }
        }
        _ => {}
    }

    let recv = now_us();
    let mut s = stats.write();
    s.msgs += 1; s.last_msg_us = recv; s.bootstrapped = true;
    if let Some(t) = &ev.timestamp {
        let exch = (t.f64() * 1_000.0) as u64;
        s.last_exchange_us = exch;
        let lat = recv.saturating_sub(exch) as f64;
        s.latency_ewma_us = if s.latency_ewma_us == 0.0 { lat } else { s.latency_ewma_us * 0.9 + lat * 0.1 };
    }
}

/// Accepts both quoted string ("0.89") and bare number (0.89)
#[derive(Deserialize)]
#[serde(untagged)]
enum N { S(String), F(f64) }

impl N {
    fn str(&self) -> String {
        match self { N::S(s) => s.clone(), N::F(f) => format!("{f}") }
    }
    fn f64(&self) -> f64 {
        match self { N::F(f) => *f, N::S(s) => fast_float::parse::<f64, _>(s).unwrap_or(0.0) }
    }
}

#[derive(Deserialize)]
struct Ev {
    event_type: Option<String>,
    asset_id:   Option<String>,
    /// "buys" (Polymarket CLOB book snapshot) OR "bids" (alternate field name)
    #[serde(alias = "bids",  default)] buys:    Option<Vec<Lvl>>,
    /// "sells" (Polymarket CLOB book snapshot) OR "asks" (alternate field name)
    #[serde(alias = "asks",  default)] sells:   Option<Vec<Lvl>>,
    #[serde(default)]                  changes: Option<Vec<Chg>>,
    #[serde(default)]                  price:   Option<N>,
    #[serde(default)]                  size:    Option<N>,
    #[serde(default)]                  side:    Option<String>,
    #[serde(default)]                  timestamp: Option<N>,
}
#[derive(Deserialize)] struct Lvl { price: N, size: N }
#[derive(Deserialize)] struct Chg { price: N, size: N, side: Option<String> }
