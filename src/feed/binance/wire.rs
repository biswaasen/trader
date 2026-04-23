use anyhow::Result;
use serde::Deserialize;
use std::time::Duration;

const REST: &str = "https://api.binance.com/api/v3/depth";

#[derive(Deserialize)]
pub struct DepthEvent {
    #[serde(rename = "E")] pub event_time_ms: u64,
    #[serde(rename = "U")] pub first_u:       u64,
    #[serde(rename = "u")] pub u:             u64,
    #[serde(rename = "b")] pub bids:          Vec<[String; 2]>,
    #[serde(rename = "a")] pub asks:          Vec<[String; 2]>,
}

#[derive(Deserialize)]
pub struct Snapshot {
    #[serde(rename = "lastUpdateId")] pub last_update_id: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

#[inline]
pub fn parse_event(text: String) -> Option<DepthEvent> {
    simd_json::serde::from_slice::<DepthEvent>(&mut text.into_bytes()).ok()
}

pub async fn fetch_snapshot(symbol: &str) -> Result<Snapshot> {
    let url = format!("{REST}?symbol={}&limit=5000", symbol);
    let bytes = reqwest::Client::builder()
        .timeout(Duration::from_secs(8)).build()?
        .get(url.as_str()).send().await?.error_for_status()?.bytes().await?;
    Ok(simd_json::serde::from_slice(&mut bytes.to_vec())?)
}
