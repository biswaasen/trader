mod clob;
pub mod gamma;

pub use gamma::{discover, Market};

use crate::feed::{now_us, BookHandle, SpotPriceHandle, StatsHandle};
use crate::market::{POLY_PX_SCALE, POLY_QTY_SCALE};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const CLOB_WS:  &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
/// Polymarket's Real-Time Data Socket — streams the exact Chainlink prices
/// used for market resolution (no auth required)
const RTDS_WS:  &str = "wss://ws-live-data.polymarket.com";
const GAMMA_API: &str = "https://gamma-api.polymarket.com/events";

pub struct Feed {
    pub up_id:           String,
    pub down_id:         String,
    pub up_book:         BookHandle,
    pub down_book:       BookHandle,
    pub up_stats:        StatsHandle,
    pub down_stats:      StatsHandle,
    /// Chainlink slash-format symbol, e.g. "btc/usd", "eth/usd"
    pub chainlink_symbol: String,
    pub spot_price:      SpotPriceHandle,
    /// Gamma event slug — polled until Chainlink posts priceToBeat
    pub event_slug:      String,
    pub price_to_beat:   SpotPriceHandle,
}

pub fn spawn(feed: Feed) {
    // ── Chainlink live price via Polymarket RTDS WebSocket ───────────────────
    {
        let sym = feed.chainlink_symbol.clone();
        let h   = feed.spot_price.clone();
        tokio::spawn(async move {
            loop {
                if let Err(_e) = run_rtds(&sym, h.clone()).await {
                    // reconnect after a short pause
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
    }

    // ── price-to-beat poller (Gamma API, every 15 s until non-zero) ─────────
    {
        let slug = feed.event_slug.clone();
        let h    = feed.price_to_beat.clone();
        let http = reqwest::Client::builder().timeout(Duration::from_secs(5))
            .build().unwrap_or_default();
        tokio::spawn(async move {
            loop {
                if *h.read() > 0.0 {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    continue;
                }
                if let Ok(p) = fetch_price_to_beat(&http, &slug).await {
                    if p > 0.0 { *h.write() = p; }
                }
                tokio::time::sleep(Duration::from_secs(15)).await;
            }
        });
    }

    // ── CLOB order-book WebSocket ────────────────────────────────────────────
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_clob(&feed).await {
                let msg = format!("{:#}", e);
                for s in [&feed.up_stats, &feed.down_stats] {
                    let mut s = s.write();
                    s.connected = false; s.reconnects += 1; s.last_error = Some(msg.clone());
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

/// Connect to Polymarket RTDS and stream Chainlink price updates into `handle`.
async fn run_rtds(symbol: &str, handle: SpotPriceHandle) -> Result<()> {
    let (ws, _) = connect_async(RTDS_WS).await.context("rtds connect")?;
    let (mut tx, mut rx) = ws.split();

    // Build subscription — filters field is a JSON-encoded string
    let filters = format!(r#"{{\"symbol\":\"{symbol}\"}}"#);
    let sub = format!(
        r#"{{"action":"subscribe","subscriptions":[{{"topic":"crypto_prices_chainlink","type":"*","filters":"{filters}"}}]}}"#
    );
    tx.send(Message::Text(sub)).await.context("rtds subscribe")?;

    while let Some(msg) = rx.next().await {
        match msg? {
            Message::Text(t) => {
                // {"topic":"crypto_prices_chainlink","type":"update","payload":{"symbol":"btc/usd","value":78215.67,...}}
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                    let is_price = v.get("topic")
                        .and_then(|t| t.as_str()) == Some("crypto_prices_chainlink");
                    if is_price {
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
    bail!("rtds stream ended")
}

async fn fetch_price_to_beat(client: &reqwest::Client, slug: &str) -> Result<f64> {
    let url  = format!("{GAMMA_API}?slug={slug}");
    let evs: Vec<serde_json::Value> = client.get(&url).send().await?.json().await?;
    let ptb = evs.first()
        .and_then(|e| e.get("eventMetadata"))
        .and_then(|m| m.get("priceToBeat"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    Ok(ptb)
}

async fn run_clob(f: &Feed) -> Result<()> {
    let (ws, _) = connect_async(CLOB_WS).await.context("poly ws")?;
    for s in [&f.up_stats, &f.down_stats] { let mut s = s.write(); s.connected = true; s.last_error = None; }

    for b in [&f.up_book, &f.down_book] {
        let mut b = b.write();
        b.clear(); b.px_scale = POLY_PX_SCALE; b.qty_scale = POLY_QTY_SCALE;
        b.seq = 0; b.updates = 0; b.last_update_us = now_us();
    }

    let (mut tx, mut rx) = ws.split();
    let sub = format!(r#"{{"type":"market","assets_ids":["{}","{}"],"custom_feature_enabled":true}}"#, f.up_id, f.down_id);
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
