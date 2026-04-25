use anyhow::{Context, Result};
use axum::{extract::Query, response::IntoResponse, routing::get, Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, params_from_iter, Connection, ToSql};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::mpsc;

static WRITER_TX: OnceLock<mpsc::UnboundedSender<PolyTick>> = OnceLock::new();
static DB_PATH: OnceLock<Arc<String>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct PolyTick {
    pub ts_ms: i64,
    pub event_slug: String,
    pub asset: String,
    pub duration: String,
    pub ptb: f64,
    pub spot: f64,
    pub up_bid: Option<f64>,
    pub up_ask: Option<f64>,
    pub down_bid: Option<f64>,
    pub down_ask: Option<f64>,
    pub up_imbalance: Option<f64>,
    pub down_imbalance: Option<f64>,
}

#[derive(Debug, Serialize)]
struct ApiRow {
    ts_ms: i64,
    event_slug: String,
    asset: String,
    duration: String,
    ptb: f64,
    spot: f64,
    up_bid: Option<f64>,
    up_ask: Option<f64>,
    down_bid: Option<f64>,
    down_ask: Option<f64>,
    up_imbalance: Option<f64>,
    down_imbalance: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DataQuery {
    slug: Option<String>,
    date: Option<String>, // YYYY-MM-DD (UTC)
    start: Option<String>, // RFC3339
    end: Option<String>,   // RFC3339
    limit: Option<usize>,
}

pub fn init() -> Result<()> {
    let db_path = std::env::var("TRADER_DB_PATH").unwrap_or_else(|_| "trader.db".to_string());
    let api_addr = std::env::var("TRADER_API_ADDR").unwrap_or_else(|_| "0.0.0.0:8787".to_string());

    let db_path_arc = Arc::new(db_path.clone());
    let _ = DB_PATH.set(db_path_arc.clone());

    init_schema(&db_path)?;
    spawn_writer(db_path.clone())?;
    spawn_api_server(db_path_arc, api_addr);
    Ok(())
}

pub fn record_poly_tick(tick: PolyTick) {
    if let Some(tx) = WRITER_TX.get() {
        let _ = tx.send(tick);
    }
}

fn init_schema(db_path: &str) -> Result<()> {
    let conn = Connection::open(db_path).context("open sqlite")?;
    conn.execute_batch(
        r#"
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA temp_store=MEMORY;

CREATE TABLE IF NOT EXISTS poly_ticks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms INTEGER NOT NULL,
  event_slug TEXT NOT NULL,
  asset TEXT NOT NULL,
  duration TEXT NOT NULL,
  ptb REAL NOT NULL,
  spot REAL NOT NULL,
  up_bid REAL,
  up_ask REAL,
  down_bid REAL,
  down_ask REAL,
  up_imbalance REAL,
  down_imbalance REAL
);
CREATE INDEX IF NOT EXISTS idx_poly_ticks_slug_ts ON poly_ticks(event_slug, ts_ms);
CREATE INDEX IF NOT EXISTS idx_poly_ticks_asset_dur_ts ON poly_ticks(asset, duration, ts_ms);
"#,
    )?;
    Ok(())
}

fn spawn_writer(db_path: String) -> Result<()> {
    if WRITER_TX.get().is_some() {
        return Ok(());
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<PolyTick>();
    let _ = WRITER_TX.set(tx);

    std::thread::Builder::new()
        .name("sqlite-writer".to_string())
        .spawn(move || {
            let conn = match Connection::open(&db_path) {
                Ok(c) => c,
                Err(_) => return,
            };

            loop {
                let first = match rx.blocking_recv() {
                    Some(v) => v,
                    None => break,
                };

                let mut batch = Vec::with_capacity(512);
                batch.push(first);

                for _ in 0..511 {
                    match rx.try_recv() {
                        Ok(v) => batch.push(v),
                        Err(_) => break,
                    }
                }

                if let Ok(txn) = conn.unchecked_transaction() {
                    if let Ok(mut stmt) = txn.prepare(
                        r#"
INSERT INTO poly_ticks (
  ts_ms,event_slug,asset,duration,ptb,spot,up_bid,up_ask,down_bid,down_ask,up_imbalance,down_imbalance
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
"#,
                    ) {
                        for t in &batch {
                            let _ = stmt.execute(params![
                                t.ts_ms,
                                t.event_slug,
                                t.asset,
                                t.duration,
                                t.ptb,
                                t.spot,
                                t.up_bid,
                                t.up_ask,
                                t.down_bid,
                                t.down_ask,
                                t.up_imbalance,
                                t.down_imbalance
                            ]);
                        }
                    }
                    let _ = txn.commit();
                }

                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .context("spawn sqlite writer")?;
    Ok(())
}

fn spawn_api_server(db_path: Arc<String>, addr: String) {
    tokio::spawn(async move {
        let app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route("/data", get(data_handler))
            .with_state(db_path);

        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(v) => v,
            Err(_) => return,
        };
        let _ = axum::serve(listener, app).await;
    });
}

async fn data_handler(
    axum::extract::State(db_path): axum::extract::State<Arc<String>>,
    Query(q): Query<DataQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(2000).min(50_000);

    let mut sql = String::from(
        "SELECT ts_ms,event_slug,asset,duration,ptb,spot,up_bid,up_ask,down_bid,down_ask,up_imbalance,down_imbalance FROM poly_ticks WHERE 1=1",
    );

    let mut args: Vec<Box<dyn ToSql>> = Vec::new();

    if let Some(slug) = q.slug {
        sql.push_str(" AND event_slug = ?");
        args.push(Box::new(slug));
    }

    if let Some(date) = q.date {
        if let Ok(d) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
            let start_dt = d.and_hms_opt(0, 0, 0).map(|v| DateTime::<Utc>::from_naive_utc_and_offset(v, Utc));
            let end_dt = d.and_hms_opt(23, 59, 59).map(|v| DateTime::<Utc>::from_naive_utc_and_offset(v, Utc));
            if let (Some(s), Some(e)) = (start_dt, end_dt) {
                sql.push_str(" AND ts_ms BETWEEN ? AND ?");
                args.push(Box::new(s.timestamp_millis()));
                args.push(Box::new(e.timestamp_millis()));
            }
        }
    } else {
        if let Some(start) = q.start.and_then(|v| DateTime::parse_from_rfc3339(&v).ok()) {
            sql.push_str(" AND ts_ms >= ?");
            args.push(Box::new(start.timestamp_millis()));
        }
        if let Some(end) = q.end.and_then(|v| DateTime::parse_from_rfc3339(&v).ok()) {
            sql.push_str(" AND ts_ms <= ?");
            args.push(Box::new(end.timestamp_millis()));
        }
    }

    sql.push_str(" ORDER BY ts_ms ASC LIMIT ?");
    args.push(Box::new(limit as i64));

    let conn = match Connection::open(db_path.as_str()) {
        Ok(c) => c,
        Err(e) => return Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    };

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => return Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    };

    let rows_iter = match stmt.query_map(params_from_iter(args.iter().map(|b| &**b)), |r| {
        Ok(ApiRow {
            ts_ms: r.get(0)?,
            event_slug: r.get(1)?,
            asset: r.get(2)?,
            duration: r.get(3)?,
            ptb: r.get(4)?,
            spot: r.get(5)?,
            up_bid: r.get(6)?,
            up_ask: r.get(7)?,
            down_bid: r.get(8)?,
            down_ask: r.get(9)?,
            up_imbalance: r.get(10)?,
            down_imbalance: r.get(11)?,
        })
    }) {
        Ok(v) => v,
        Err(e) => return Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    };

    let mut out = Vec::new();
    for row in rows_iter.flatten() {
        out.push(row);
    }
    Json(serde_json::json!({ "ok": true, "count": out.len(), "rows": out }))
}
