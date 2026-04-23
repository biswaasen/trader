use anyhow::Result;
use futures_util::future::join_all;
use serde::Deserialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const GAMMA: &str = "https://gamma-api.polymarket.com/events";

// Short-window markets are created minutes before each window and closed
// immediately after. The generic active=true&closed=false list misses them.
// Computing slugs from the clock is the only reliable discovery method.
const PAIRS: &[(&str, &str, u64)] = &[
    ("btc", "BTC", 300),
    ("btc", "BTC", 900),
    ("btc", "BTC", 3_600),
    ("eth", "ETH", 300),
    ("eth", "ETH", 900),
    ("eth", "ETH", 3_600),
    ("sol", "SOL", 900),
    ("xrp", "XRP", 900),
];

#[derive(Clone, Debug)]
pub struct Market {
    pub event_slug:    String,
    pub title:         String,
    pub up_token_id:   String,
    pub down_token_id: String,
    pub up_price:      f64,
    pub down_price:    f64,
    pub end_date:      String,
    pub duration:      String,
    pub asset:         String,
}

pub async fn discover() -> Result<Vec<Market>> {
    let now    = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let client = reqwest::Client::builder().timeout(Duration::from_secs(8)).build()?;

    // current window + previous (fallback if current not yet created)
    let mut slug_jobs: Vec<(String, String, String)> = Vec::new();
    for &(a, au, sec) in PAIRS {
        let dur  = sec_to_label(sec);
        let cur  = (now / sec) * sec;
        let prev = cur.saturating_sub(sec);
        for ts in [cur, prev] {
            slug_jobs.push((au.into(), dur.into(), format!("{a}-updown-{dur}-{ts}")));
        }
    }

    let futs: Vec<_> = slug_jobs.into_iter().map(|(asset, dur, slug)| {
        let c = client.clone();
        async move {
            let url  = format!("{GAMMA}?slug={slug}");
            let evs: Vec<GammaEvent> = c.get(&url).send().await.ok()?.json().await.ok()?;
            let ev   = evs.into_iter().find(|e| !e.closed)?;
            let mkt  = ev.markets.into_iter().next()?;
            let ids  = json_vec(mkt.clob_token_ids.as_deref())?;
            if ids.len() < 2 { return None; }
            let prices = json_vec(mkt.outcome_prices.as_deref()).unwrap_or_default();
            let pf = |s: Option<&String>| s.and_then(|v| fast_float::parse::<f64,_>(v).ok()).unwrap_or(0.5);
            Some(Market {
                event_slug:    slug,
                title:         ev.title,
                up_token_id:   ids[0].clone(),
                down_token_id: ids[1].clone(),
                up_price:      pf(prices.first()),
                down_price:    pf(prices.get(1)),
                end_date:      mkt.end_date.unwrap_or_default(),
                duration:      dur,
                asset,
            })
        }
    }).collect();

    let mut out: Vec<Market> = join_all(futs).await.into_iter().flatten().collect();
    out.sort_by(|a, b| a.end_date.cmp(&b.end_date));
    out.dedup_by(|a, b| a.event_slug == b.event_slug);
    Ok(out)
}

fn sec_to_label(s: u64) -> &'static str {
    match s { 300 => "5m", 900 => "15m", 3_600 => "1h", 14_400 => "4h", _ => "24h" }
}

fn json_vec(s: Option<&str>) -> Option<Vec<String>> { serde_json::from_str(s?).ok() }

#[derive(Deserialize)]
struct GammaEvent {
    title:   String,
    #[serde(default)] closed:  bool,
    #[serde(default)] markets: Vec<GammaMarket>,
}
#[derive(Deserialize)]
struct GammaMarket {
    #[serde(rename = "clobTokenIds")]  clob_token_ids: Option<String>,
    #[serde(rename = "outcomePrices")] outcome_prices: Option<String>,
    #[serde(rename = "endDate")]       end_date:       Option<String>,
}
