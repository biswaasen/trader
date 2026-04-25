mod clob;
pub mod gamma;

pub use gamma::{discover, Market};

use crate::feed::{now_us, BookHandle, MarketHandle, SpotPriceHandle, StatsHandle};
use crate::market::{POLY_PX_SCALE, POLY_QTY_SCALE};
use crate::storage::{record_poly_tick, PolyTick};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const CLOB_WS:   &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const RTDS_WS:   &str = "wss://ws-live-data.polymarket.com";
const GAMMA_API: &str = "https://gamma-api.polymarket.com/events";
const PTB_API:   &str = "https://polymarket.com/api/crypto/crypto-price";

#[derive(Clone)]
pub struct Feed {
    pub market:           MarketHandle,
    pub up_book:          BookHandle,
    pub down_book:        BookHandle,
    pub up_stats:         StatsHandle,
    pub down_stats:       StatsHandle,
    /// Chainlink slash-format: "btc/usd", "eth/usd", etc.
    pub chainlink_symbol: String,
    pub spot_price:       SpotPriceHandle,
    pub price_to_beat:    SpotPriceHandle,
}

pub fn spawn(feed: Feed) {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("Mozilla/5.0 (compatible; trader/1.0)")
        .build().unwrap_or_default();

    // ── Chainlink live price via Polymarket RTDS ─────────────────────────────
    {
        let sym  = feed.chainlink_symbol.clone();
        let spot = feed.spot_price.clone();
        tokio::spawn(async move {
            loop {
                let _ = run_rtds(&sym, spot.clone()).await;
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
    }

    // ── Price-to-beat via Polymarket's own crypto-price API ──────────────────
    // GET https://polymarket.com/api/crypto/crypto-price?symbol=BTC&eventStartTime=...&variant=fifteen&endDate=...
    // Returns { "openPrice": 78257.57, ... } — the exact value Polymarket UI shows.
    {
        let h   = feed.price_to_beat.clone();
        let mkt = feed.market.clone();
        let c   = http.clone();
        tokio::spawn(async move {
            let mut last_slug = String::new();
            loop {
                let slug = mkt.read().event_slug.clone();
                if slug != last_slug { last_slug = slug.clone(); *h.write() = 0.0; }

                if *h.read() > 0.0 {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    continue;
                }
                if let Ok(p) = fetch_price_to_beat(&c, &slug).await {
                    *h.write() = p;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    // ── CLOB order-book WS + auto-advance ────────────────────────────────────
    let feed_clob = feed.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_clob(&feed_clob).await {
                let msg = format!("{:#}", e);
                for s in [&feed_clob.up_stats, &feed_clob.down_stats] {
                    let mut s = s.write();
                    s.connected = false; s.reconnects += 1;
                    s.last_error = Some(msg.clone());
                }
            }

            let expired = {
                let end = feed_clob.market.read().end_date.clone();
                secs_left_simple(&end) <= 0.0
            };
            if expired {
                for attempt in 0u32..5 {
                    if advance_market(&http, &feed_clob.market, &feed_clob.price_to_beat,
                                      &feed_clob.up_book, &feed_clob.down_book).await { break; }
                    let wait = 10 * (1 + attempt);
                    tokio::time::sleep(Duration::from_secs(wait as u64)).await;
                }
            }

            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    // ── 1Hz snapshot recorder (SQLite writer queue) ───────────────────────────
    {
        let market = feed.market.clone();
        let up_book = feed.up_book.clone();
        let down_book = feed.down_book.clone();
        let spot = feed.spot_price.clone();
        let ptb = feed.price_to_beat.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            tick.tick().await;
            loop {
                tick.tick().await;
                let ts_ms = chrono::Utc::now().timestamp_millis();

                let m = market.read().clone();
                let up = up_book.read();
                let down = down_book.read();
                let up_bid = up.best_bid().map(|(p, _)| p as f64 / up.px_scale as f64);
                let up_ask = up.best_ask().map(|(p, _)| p as f64 / up.px_scale as f64);
                let down_bid = down.best_bid().map(|(p, _)| p as f64 / down.px_scale as f64);
                let down_ask = down.best_ask().map(|(p, _)| p as f64 / down.px_scale as f64);
                let up_imb = up.imbalance_top(5);
                let down_imb = down.imbalance_top(5);
                drop(up);
                drop(down);

                record_poly_tick(PolyTick {
                    ts_ms,
                    event_slug: m.event_slug,
                    asset: m.asset,
                    duration: m.duration,
                    ptb: *ptb.read(),
                    spot: *spot.read(),
                    up_bid,
                    up_ask,
                    down_bid,
                    down_ask,
                    up_imbalance: up_imb,
                    down_imbalance: down_imb,
                });
            }
        });
    }
}

// ── price-to-beat: Polymarket's own internal endpoint ────────────────────────
// openPrice = the Chainlink oracle price recorded at window open. This is the
// same value shown on polymarket.com. Available immediately when window opens.

async fn fetch_price_to_beat(client: &reqwest::Client, slug: &str) -> Result<f64> {
    let start_secs = slug.rsplitn(2, '-').next()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| anyhow::anyhow!("bad slug"))?;

    let dur_label = slug.split('-').find(|p| matches!(*p, "5m" | "15m" | "1h")).unwrap_or("15m");
    let dur_secs  = match dur_label { "5m" => 300_i64, "1h" => 3_600, _ => 900 };
    let end_secs  = start_secs + dur_secs;

    let asset   = slug.split('-').next().unwrap_or("btc").to_uppercase();
    let variant = match dur_label { "5m" => "five", "1h" => "one_hour", _ => "fifteen" };

    let start_iso = secs_to_iso(start_secs);
    let end_iso   = secs_to_iso(end_secs);

    let url = format!("{PTB_API}?symbol={asset}&eventStartTime={start_iso}&variant={variant}&endDate={end_iso}");
    let v: serde_json::Value = client.get(&url).send().await?.json().await?;

    let p = v.get("openPrice").and_then(|x| x.as_f64()).unwrap_or(0.0);
    if p > 0.0 { Ok(p) } else { bail!("openPrice not available yet") }
}

fn secs_to_iso(ts: i64) -> String {
    use chrono::{DateTime, Utc};
    DateTime::from_timestamp(ts, 0)
        .map(|dt: DateTime<Utc>| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

// ── RTDS Chainlink live price (spot only) ────────────────────────────────────

async fn run_rtds(symbol: &str, handle: SpotPriceHandle) -> Result<()> {
    let (ws, _) = connect_async(RTDS_WS).await.context("rtds connect")?;
    let (mut tx, mut rx) = ws.split();

    let filters = format!(r#"{{\"symbol\":\"{symbol}\"}}"#);
    let sub = format!(
        r#"{{"action":"subscribe","subscriptions":[{{"topic":"crypto_prices_chainlink","type":"*","filters":"{filters}"}}]}}"#
    );
    tx.send(Message::Text(sub)).await.context("rtds subscribe")?;

    let mut ping = tokio::time::interval(Duration::from_secs(5));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await;

    loop {
        tokio::select! {
            _ = ping.tick() => { tx.send(Message::Text("PING".into())).await.ok(); }
            msg = rx.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if t.trim() == "PONG" { continue; }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                        if v.get("topic").and_then(|t| t.as_str()) != Some("crypto_prices_chainlink") { continue; }
                        // live update: payload.value | backfill snapshot: payload.data[last].value
                        let price = v.pointer("/payload/value").and_then(|x| x.as_f64())
                            .or_else(|| v.pointer("/payload/data")
                                .and_then(|d| d.as_array())
                                .and_then(|a| a.last())
                                .and_then(|e| e.get("value"))
                                .and_then(|x| x.as_f64()));
                        if let Some(p) = price { *handle.write() = p; }
                    }
                }
                Some(Ok(Message::Ping(p))) => { tx.send(Message::Pong(p)).await.ok(); }
                Some(Ok(Message::Close(_))) | None => bail!("rtds closed"),
                Some(Err(e)) => bail!("rtds err: {e}"),
                _ => {}
            }
        }
    }
}

// ── auto-advance ──────────────────────────────────────────────────────────────

struct MktSnap {
    title:      String,
    end_date:   String,
    event_slug: String,
    up_id:      String,
    down_id:    String,
}

async fn advance_market(
    client:    &reqwest::Client,
    market:    &MarketHandle,
    ptb:       &SpotPriceHandle,
    up_book:   &BookHandle,
    down_book: &BookHandle,
) -> bool {
    let (slug, duration) = { let m = market.read(); (m.event_slug.clone(), m.duration.clone()) };
    let window_secs = match duration.as_str() { "5m" => 300u64, "1h" => 3_600, _ => 900 };
    let next = next_slug(&slug, window_secs);

    if let Ok(Some(snap)) = fetch_snap(client, &next).await {
        { let mut m = market.write();
          m.title = snap.title; m.end_date = snap.end_date;
          m.event_slug = snap.event_slug; m.up_id = snap.up_id; m.down_id = snap.down_id; }
        *ptb.write() = 0.0; // triggers the PTB poller to fetch for new window
        for b in [up_book, down_book] { b.write().clear(); }
        return true;
    }
    false
}

async fn fetch_snap(client: &reqwest::Client, slug: &str) -> Result<Option<MktSnap>> {
    let url = format!("{GAMMA_API}?slug={slug}");
    let evs: Vec<serde_json::Value> = client.get(&url).send().await?.json().await?;
    let ev  = match evs.into_iter().find(|e| !e.get("closed").and_then(|v| v.as_bool()).unwrap_or(false)) {
        Some(e) => e,
        None    => return Ok(None),
    };
    let title = ev.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mkt   = match ev.get("markets").and_then(|v| v.as_array()).and_then(|a| a.first()) {
        Some(m) => m.clone(),
        None    => return Ok(None),
    };
    let ids: Vec<String> = mkt.get("clobTokenIds")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    if ids.len() < 2 { return Ok(None); }

    Ok(Some(MktSnap {
        title,
        end_date:   mkt.get("endDate").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        event_slug: slug.to_string(),
        up_id:      ids[0].clone(),
        down_id:    ids[1].clone(),
    }))
}

fn next_slug(current: &str, window_secs: u64) -> String {
    let mut parts = current.rsplitn(2, '-');
    if let (Some(ts_str), Some(base)) = (parts.next(), parts.next()) {
        if let Ok(ts) = ts_str.parse::<u64>() {
            return format!("{base}-{}", ts + window_secs);
        }
    }
    current.to_string()
}

fn secs_left_simple(end_date: &str) -> f64 {
    use chrono::{DateTime, Utc};
    DateTime::parse_from_rfc3339(end_date)
        .map(|dt| (dt.with_timezone(&Utc) - Utc::now()).num_milliseconds() as f64 / 1_000.0)
        .unwrap_or(-1.0)
}

// ── CLOB order-book WebSocket ─────────────────────────────────────────────────

async fn run_clob(f: &Feed) -> Result<()> {
    let (up_id, down_id) = { let m = f.market.read(); (m.up_id.clone(), m.down_id.clone()) };

    let (ws, _) = connect_async(CLOB_WS).await.context("poly ws")?;
    for s in [&f.up_stats, &f.down_stats] { let mut s = s.write(); s.connected = true; s.last_error = None; }

    for b in [&f.up_book, &f.down_book] {
        let mut b = b.write();
        b.clear(); b.px_scale = POLY_PX_SCALE; b.qty_scale = POLY_QTY_SCALE;
        b.seq = 0; b.updates = 0; b.last_update_us = now_us();
    }

    let (mut tx, mut rx) = ws.split();
    let sub = format!(r#"{{"type":"market","assets_ids":["{up_id}","{down_id}"],"custom_feature_enabled":true}}"#);
    tx.send(Message::Text(sub)).await.context("subscribe")?;

    let mut ping = tokio::time::interval(Duration::from_secs(10));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await;

    loop {
        tokio::select! {
            _ = ping.tick() => { tx.send(Message::Text("PING".into())).await.ok(); }
            msg = rx.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if t.trim() == "PONG" { continue; }
                    clob::handle(t.as_str(), f)?;
                }
                Some(Ok(Message::Ping(p))) => { tx.send(Message::Pong(p)).await.ok(); }
                Some(Ok(Message::Close(_))) | None => bail!("ws closed"),
                Some(Err(e)) => bail!("ws err: {e}"),
                _ => {}
            }
        }
    }
}
