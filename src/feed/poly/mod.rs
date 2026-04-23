mod clob;
pub mod gamma;

pub use gamma::{discover, Market};

use crate::feed::{now_us, BookHandle, StatsHandle};
use crate::market::{POLY_PX_SCALE, POLY_QTY_SCALE};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const CLOB_WS: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

pub struct Feed {
    pub up_id:      String,
    pub down_id:    String,
    pub up_book:    BookHandle,
    pub down_book:  BookHandle,
    pub up_stats:   StatsHandle,
    pub down_stats: StatsHandle,
}

pub fn spawn(feed: Feed) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run(&feed).await {
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

async fn run(f: &Feed) -> Result<()> {
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
