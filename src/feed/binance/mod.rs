mod wire;

use crate::feed::{now_us, BookHandle, StatsHandle};
use crate::market::{parse_px, BINANCE_PX_SCALE, BINANCE_QTY_SCALE};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::collections::VecDeque;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use wire::{fetch_snapshot, parse_event, DepthEvent};

const WS: &str = "wss://stream.binance.com:9443/ws";

pub fn spawn(symbol: String, book: BookHandle, stats: StatsHandle) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run(&symbol, &book, &stats).await {
                let mut s = stats.write();
                s.connected = false; s.bootstrapped = false;
                s.reconnects += 1; s.last_error = Some(format!("{:#}", e));
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

async fn run(symbol: &str, book: &BookHandle, stats: &StatsHandle) -> Result<()> {
    let (ws, _) = connect_async(format!("{WS}/{}@depth", symbol.to_lowercase()).as_str())
        .await.context("ws connect")?;
    { let mut s = stats.write(); s.connected = true; s.last_error = None; }

    let (mut tx, mut rx) = ws.split();
    let mut buf: VecDeque<DepthEvent> = VecDeque::with_capacity(512);
    let sym_uc   = symbol.to_uppercase();
    let snap_fut = fetch_snapshot(&sym_uc);
    tokio::pin!(snap_fut);

    let snap = loop {
        tokio::select! {
            biased;
            got = &mut snap_fut => { break got.context("snapshot")?; }
            msg = rx.next() => match msg {
                Some(Ok(Message::Text(t)))    => { if let Some(ev) = parse_event(t) { buf.push_back(ev); } }
                Some(Ok(Message::Ping(p)))    => { tx.send(Message::Pong(p)).await.ok(); }
                Some(Ok(Message::Close(_))) | None => bail!("ws closed during bootstrap"),
                Some(Err(e))                  => bail!("ws err: {e}"),
                _ => {}
            }
        }
    };

    {
        let mut b = book.write();
        b.clear(); b.px_scale = BINANCE_PX_SCALE; b.qty_scale = BINANCE_QTY_SCALE;
        for [p,q] in &snap.bids { b.apply_bid(parse_px(p, BINANCE_PX_SCALE), parse_px(q, BINANCE_QTY_SCALE)); }
        for [p,q] in &snap.asks { b.apply_ask(parse_px(p, BINANCE_PX_SCALE), parse_px(q, BINANCE_QTY_SCALE)); }
        b.seq = snap.last_update_id; b.updates = 0; b.last_update_us = now_us();
    }

    let mut seq = snap.last_update_id;
    while let Some(f) = buf.front() {
        if f.u <= seq { buf.pop_front(); continue; }
        if !(f.first_u <= seq + 1 && f.u >= seq + 1) { bail!("bad bootstrap event"); }
        break;
    }
    for ev in buf.drain(..) { apply(book, &ev, &mut seq)?; }
    stats.write().bootstrapped = true;

    loop {
        match rx.next().await {
            Some(Ok(Message::Text(t))) => {
                let recv = now_us();
                let Some(ev) = parse_event(t) else { continue };
                let exch_us = ev.event_time_ms * 1_000;
                apply(book, &ev, &mut seq)?;
                let mut s = stats.write();
                s.msgs += 1; s.last_msg_us = recv; s.last_exchange_us = exch_us;
                let lat = recv.saturating_sub(exch_us) as f64;
                s.latency_ewma_us = if s.latency_ewma_us == 0.0 { lat } else { s.latency_ewma_us * 0.9 + lat * 0.1 };
            }
            Some(Ok(Message::Ping(p))) => { tx.send(Message::Pong(p)).await.ok(); }
            Some(Ok(Message::Close(_))) | None => bail!("ws closed"),
            Some(Err(e)) => bail!("ws err: {e}"),
            _ => {}
        }
    }
}

#[inline]
fn apply(book: &BookHandle, ev: &DepthEvent, seq: &mut u64) -> Result<()> {
    if ev.u <= *seq { return Ok(()); }
    if ev.first_u != *seq + 1 { bail!("seq gap: expected {}, got U={} u={}", *seq+1, ev.first_u, ev.u); }
    let mut b = book.write();
    for [p,q] in &ev.bids { b.apply_bid(parse_px(p, BINANCE_PX_SCALE), parse_px(q, BINANCE_QTY_SCALE)); }
    for [p,q] in &ev.asks { b.apply_ask(parse_px(p, BINANCE_PX_SCALE), parse_px(q, BINANCE_QTY_SCALE)); }
    b.seq = ev.u; b.updates += 1; b.last_update_us = now_us(); *seq = ev.u;
    Ok(())
}
