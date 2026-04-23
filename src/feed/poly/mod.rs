mod clob;
pub mod gamma;

pub use gamma::{discover, Market};

use crate::feed::{now_us, BookHandle, MarketHandle, SpotPriceHandle, StatsHandle};
use crate::market::{POLY_PX_SCALE, POLY_QTY_SCALE};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const CLOB_WS:   &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const RTDS_WS:   &str = "wss://ws-live-data.polymarket.com";
const GAMMA_API: &str = "https://gamma-api.polymarket.com/events";

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
    let http = reqwest::Client::builder().timeout(Duration::from_secs(6))
        .build().unwrap_or_default();

    // ── Chainlink live price via Polymarket RTDS ─────────────────────────────
    {
        let sym = feed.chainlink_symbol.clone();
        let h   = feed.spot_price.clone();
        tokio::spawn(async move {
            loop {
                let _ = run_rtds(&sym, h.clone()).await;
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
    }

    // ── price-to-beat poller (polls until set, then watches for slug changes) ─
    {
        let h    = feed.price_to_beat.clone();
        let mkt  = feed.market.clone();
        let c    = http.clone();
        tokio::spawn(async move {
            let mut last_slug = String::new();
            loop {
                let slug = mkt.read().event_slug.clone();
                // If slug changed (market advanced), force a re-fetch
                if slug != last_slug { last_slug = slug.clone(); *h.write() = 0.0; }

                if *h.read() > 0.0 {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
                if let Ok(p) = fetch_price_to_beat(&c, &slug).await {
                    if p > 0.0 { *h.write() = p; }
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });
    }

    // ── CLOB order-book WS + auto-advance ────────────────────────────────────
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_clob(&feed).await {
                let msg = format!("{:#}", e);
                for s in [&feed.up_stats, &feed.down_stats] {
                    let mut s = s.write();
                    s.connected = false; s.reconnects += 1;
                    s.last_error = Some(msg.clone());
                }
            }

            // After disconnect, check if the window expired → try to roll forward
            let expired = {
                let end = feed.market.read().end_date.clone();
                secs_left_simple(&end) <= 0.0
            };
            if expired {
                // Retry fetching next window up to 5 times (new window may not exist yet)
                for attempt in 0u32..5 {
                    if advance_market(&http, &feed.market, &feed.price_to_beat,
                                      &feed.up_book, &feed.down_book).await { break; }
                    let wait = 10 * (1 + attempt);
                    tokio::time::sleep(Duration::from_secs(wait as u64)).await;
                }
            }

            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

// ── RTDS Chainlink live price ─────────────────────────────────────────────────

async fn run_rtds(symbol: &str, handle: SpotPriceHandle) -> Result<()> {
    let (ws, _) = connect_async(RTDS_WS).await.context("rtds connect")?;
    let (mut tx, mut rx) = ws.split();
    let filters = format!(r#"{{\"symbol\":\"{symbol}\"}}"#);
    let sub = format!(
        r#"{{"action":"subscribe","subscriptions":[{{"topic":"crypto_prices_chainlink","type":"*","filters":"{filters}"}}]}}"#
    );
    tx.send(Message::Text(sub)).await.context("rtds subscribe")?;

    while let Some(msg) = rx.next().await {
        match msg? {
            Message::Text(t) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                    if v.get("topic").and_then(|t| t.as_str()) == Some("crypto_prices_chainlink") {
                        if let Some(p) = v.pointer("/payload/value").and_then(|v| v.as_f64()) {
                            *handle.write() = p;
                        }
                    }
                }
            }
            Message::Ping(p) => { tx.send(Message::Pong(p)).await.ok(); }
            Message::Close(_) => bail!("rtds closed"),
            _ => {}
        }
    }
    bail!("rtds ended")
}

// ── price-to-beat fetch (handles both object and string-encoded eventMetadata) ─

async fn fetch_price_to_beat(client: &reqwest::Client, slug: &str) -> Result<f64> {
    let url = format!("{GAMMA_API}?slug={slug}");
    let evs: Vec<serde_json::Value> = client.get(&url).send().await?.json().await?;
    let ev  = evs.first().ok_or_else(|| anyhow::anyhow!("no event"))?;
    Ok(extract_ptb(ev))
}

fn extract_ptb(ev: &serde_json::Value) -> f64 {
    // Try as nested object: eventMetadata.priceToBeat
    if let Some(p) = ev.pointer("/eventMetadata/priceToBeat").and_then(|v| v.as_f64()) {
        if p > 0.0 { return p; }
    }
    // Try as stringified JSON: eventMetadata = "{\"priceToBeat\":78192.52}"
    if let Some(s) = ev.get("eventMetadata").and_then(|v| v.as_str()) {
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(s) {
            if let Some(p) = meta.get("priceToBeat").and_then(|v| v.as_f64()) {
                if p > 0.0 { return p; }
            }
        }
    }
    // Also try inside each market's eventMetadata
    if let Some(markets) = ev.get("markets").and_then(|v| v.as_array()) {
        for m in markets {
            if let Some(p) = m.pointer("/eventMetadata/priceToBeat").and_then(|v| v.as_f64()) {
                if p > 0.0 { return p; }
            }
        }
    }
    0.0
}

// ── auto-advance: fetch next window and hot-swap market state ─────────────────

struct MktSnap {
    title:         String,
    end_date:      String,
    event_slug:    String,
    up_id:         String,
    down_id:       String,
    price_to_beat: f64,
}

async fn advance_market(
    client:    &reqwest::Client,
    market:    &MarketHandle,
    ptb:       &SpotPriceHandle,
    up_book:   &BookHandle,
    down_book: &BookHandle,
) -> bool {
    let (slug, duration) = { let m = market.read(); (m.event_slug.clone(), m.duration.clone()) };
    let window_secs = dur_secs(&duration);
    let next = next_slug(&slug, window_secs);

    match fetch_snap(client, &next).await {
        Ok(Some(snap)) => {
            { let mut m = market.write();
              m.title = snap.title; m.end_date = snap.end_date;
              m.event_slug = snap.event_slug; m.up_id = snap.up_id; m.down_id = snap.down_id; }
            *ptb.write() = snap.price_to_beat;
            for b in [up_book, down_book] { b.write().clear(); }
            true
        }
        _ => false,
    }
}

async fn fetch_snap(client: &reqwest::Client, slug: &str) -> Result<Option<MktSnap>> {
    let url = format!("{GAMMA_API}?slug={slug}");
    let evs: Vec<serde_json::Value> = client.get(&url).send().await?.json().await?;
    let ev  = match evs.into_iter().find(|e| !e.get("closed").and_then(|v| v.as_bool()).unwrap_or(false)) {
        Some(e) => e,
        None    => return Ok(None),
    };
    let title = ev.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let ptb   = extract_ptb(&ev);

    let mkt = match ev.get("markets").and_then(|v| v.as_array()).and_then(|a| a.first()) {
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
        end_date:      mkt.get("endDate").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        event_slug:    slug.to_string(),
        up_id:         ids[0].clone(),
        down_id:       ids[1].clone(),
        price_to_beat: ptb,
    }))
}

fn next_slug(current: &str, window_secs: u64) -> String {
    // "btc-updown-15m-1745621400" → base="btc-updown-15m", ts=1745621400
    let mut parts = current.rsplitn(2, '-');
    if let (Some(ts_str), Some(base)) = (parts.next(), parts.next()) {
        if let Ok(ts) = ts_str.parse::<u64>() {
            return format!("{base}-{}", ts + window_secs);
        }
    }
    current.to_string()
}

fn dur_secs(label: &str) -> u64 {
    match label { "5m" => 300, "15m" => 900, "1h" => 3_600, _ => 900 }
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
