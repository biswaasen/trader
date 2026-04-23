use super::fmt;
use crate::feed::{now_us, PolyPane};
use crate::market::{Side, POLY_PX_SCALE, POLY_QTY_SCALE};
use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
// ── public entry point ───────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, p: &PolyPane) {
    let (up_bids, up_asks, up_mid, up_imb, up_trade, up_conn, up_boot, up_msgs, up_lat, up_err) = snap_book(p, true);
    let (dn_bids, dn_asks, dn_mid, dn_imb, dn_trade, dn_conn, dn_boot, dn_msgs, dn_lat, _)     = snap_book(p, false);

    let btc_now = p.btc_book.as_ref().and_then(|b| b.read().mid_f64());
    let secs_left = secs_remaining(&p.end_date);
    let fair_up   = btc_now.and_then(|btc| fair_value(btc, p.price_to_beat, secs_left, &p.duration));

    let conn  = up_conn && dn_conn;
    let boot  = up_boot && dn_boot;
    let msgs  = up_msgs + dn_msgs;
    let lat   = (up_lat + dn_lat) * 0.5;
    let err   = up_err;

    let status_str = if !conn { "DISCONNECTED" } else if !boot { "syncing…" } else { "live" };
    let status_col = if conn && boot { Color::Green } else if conn { Color::Yellow } else { Color::Red };
    let countdown  = fmt_countdown(secs_left);
    let cd_col     = if secs_left < 60.0 { Color::Red } else if secs_left < 300.0 { Color::Yellow } else { Color::Cyan };

    let bottom = Line::from(vec![
        Span::raw(" "),
        Span::styled(format!("{} msgs", fmt::human(msgs)),          Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(format!("lat {:.0}ms", lat / 1_000.0),        Style::default().fg(fmt::latency_color(lat))),
        Span::raw("  "),
        Span::styled(status_str, Style::default().fg(status_col)),
        Span::raw(" "),
    ]);

    let block = Block::default().borders(Borders::ALL)
        .border_style(Style::default().fg(if conn { Color::Magenta } else { Color::DarkGray }))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("POLY · {} {}", p.asset, p.duration),
                         Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(&p.title, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
        ]))
        .title_top(Line::from(vec![
            Span::raw(" "),
            Span::styled(countdown, Style::default().fg(cd_col).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
        ]).right_aligned())
        .title_bottom(bottom);

    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(e) = &err {
        if up_bids.is_empty() && up_asks.is_empty() && dn_bids.is_empty() && dn_asks.is_empty() {
            f.render_widget(Paragraph::new(vec![
                Line::from(Span::styled("connection error:", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))),
                Line::from(Span::styled(e.clone(), Style::default().fg(Color::Yellow))),
                Line::from(Span::styled("retrying…", Style::default().fg(Color::DarkGray))),
            ]), inner);
            return;
        }
    }

    let mut lines: Vec<Line> = Vec::with_capacity(32);

    // ── reference price row ──────────────────────────────────────────────────
    if p.price_to_beat > 0.0 || btc_now.is_some() {
        let ref_s = if p.price_to_beat > 0.0 {
            format!("ref  ${}", fmt_btc(p.price_to_beat))
        } else { "ref  —".into() };
        let (now_s, delta_s, delta_col) = match btc_now {
            Some(n) if p.price_to_beat > 0.0 => {
                let d = n - p.price_to_beat;
                let pct = d / p.price_to_beat * 100.0;
                let sign = if d >= 0.0 { "+" } else { "" };
                let col = if d > 0.0 { Color::Green } else if d < 0.0 { Color::Red } else { Color::Gray };
                (format!("now  ${}", fmt_btc(n)), format!("Δ  {sign}${:.0}  ({sign}{pct:.2}%)", d.abs().copysign(d)), col)
            }
            Some(n) => (format!("now  ${}", fmt_btc(n)), String::new(), Color::Gray),
            None    => ("now  —".into(), String::new(), Color::Gray),
        };
        lines.push(Line::from(vec![
            Span::styled(ref_s, Style::default().fg(Color::DarkGray)),
            Span::raw("    "),
            Span::styled(now_s, Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled(delta_s, Style::default().fg(delta_col).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(""));
    }

    // ── UP / DOWN market rows ────────────────────────────────────────────────
    let up_mkt = up_mid.unwrap_or(0.0);
    let dn_mkt = dn_mid.unwrap_or(0.0);

    let draw_side = |label: &str, mkt: f64, fair: Option<f64>, is_up: bool| -> Line<'static> {
        let mkt_s = if mkt > 0.0 { format!("mkt  {:.1}¢", mkt * 100.0) } else { "mkt  —".into() };
        let (fair_s, edge_s, edge_col, signal) = match fair {
            Some(fv) => {
                let fv_s = format!("fair  {:.1}¢", fv * 100.0);
                let edge = if is_up { fv - mkt } else { (1.0 - fv) - mkt };
                let sign = if edge >= 0.0 { "+" } else { "" };
                let col  = if edge > 0.03 { Color::Green } else if edge < -0.03 { Color::Red } else { Color::Yellow };
                let sig  = if edge > 0.03 && is_up { " ▲ BUY" } else if edge < -0.03 && !is_up { " ▲ BUY" } else { "" };
                (fv_s, format!("edge  {sign}{:.1}¢", edge * 100.0), col, sig)
            }
            None => ("fair  —".into(), "edge  —".into(), Color::DarkGray, ""),
        };
        Line::from(vec![
            Span::styled(format!("{label:<5}"), Style::default().fg(if is_up { Color::Green } else { Color::Red }).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(mkt_s, Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled(fair_s, Style::default().fg(Color::Cyan)),
            Span::raw("    "),
            Span::styled(edge_s, Style::default().fg(edge_col)),
            Span::styled(signal, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ])
    };

    lines.push(draw_side("UP",   up_mkt, fair_up,            true));
    lines.push(draw_side("DOWN", dn_mkt, fair_up.map(|f| f), false));
    lines.push(Line::from(""));

    // ── side-by-side order books ─────────────────────────────────────────────
    let half = (inner.width as usize).saturating_sub(4) / 2;
    let levels = ((inner.height as usize).saturating_sub(lines.len() + 6) / 2).clamp(2, 6);

    let header = Line::from(vec![
        Span::styled(format!("{:<half$}", "── UP book"), Style::default().fg(Color::Green).add_modifier(Modifier::DIM)),
        Span::raw("  "),
        Span::styled("── DOWN book", Style::default().fg(Color::Red).add_modifier(Modifier::DIM)),
    ]);
    lines.push(header);

    let max_q = up_bids.iter().chain(up_asks.iter()).chain(dn_bids.iter()).chain(dn_asks.iter())
        .map(|(_,q)|*q).max().unwrap_or(1).max(1);
    let bar_w = (half.saturating_sub(20)).clamp(4, 16);

    for i in 0..levels {
        let up_ask = up_asks.get(levels - 1 - i);
        let dn_ask = dn_asks.get(levels - 1 - i);
        lines.push(book_line(up_ask, dn_ask, max_q, half, bar_w, Color::Red));
    }
    let imb_line = Line::from(vec![
        Span::styled(format!("{:<half$}", imb_str(up_imb)), Style::default().fg(fmt::imb_color(up_imb))),
        Span::raw("  "),
        Span::styled(imb_str(dn_imb), Style::default().fg(fmt::imb_color(dn_imb))),
    ]);
    for i in 0..levels {
        let up_bid = up_bids.get(i);
        let dn_bid = dn_bids.get(i);
        lines.push(book_line(up_bid, dn_bid, max_q, half, bar_w, Color::Green));
    }
    lines.push(imb_line);
    lines.push(Line::from(""));

    // ── last trades ──────────────────────────────────────────────────────────
    let trade_line = |label: &str, trade: Option<crate::market::Trade>, is_up: bool| -> Line<'static> {
        let col = if is_up { Color::Green } else { Color::Red };
        match trade {
            Some(t) => {
                let age_ms = now_us().saturating_sub(t.ts_us) / 1_000;
                let (ss, sc) = if matches!(t.side, Side::Buy) { ("BUY", Color::Green) } else { ("SELL", Color::Red) };
                Line::from(vec![
                    Span::styled(format!("{label:<5}"), Style::default().fg(col).add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
                    Span::styled(ss, Style::default().fg(sc).add_modifier(Modifier::BOLD)),
                    Span::raw("  "),
                    Span::styled(fmt::price(t.price_u as f64 / POLY_PX_SCALE as f64, POLY_PX_SCALE), Style::default().fg(Color::White)),
                    Span::styled(format!("  ×{}", fmt::qty(t.qty_u as f64 / POLY_QTY_SCALE as f64)), Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("  {age_ms}ms ago"), Style::default().fg(Color::DarkGray)),
                ])
            }
            None => Line::from(vec![
                Span::styled(format!("{label:<5}"), Style::default().fg(col)),
                Span::styled(" no trades yet", Style::default().fg(Color::DarkGray)),
            ]),
        }
    };
    lines.push(trade_line("UP",   up_trade, true));
    lines.push(trade_line("DOWN", dn_trade, false));

    f.render_widget(Paragraph::new(lines), inner);
}

// ── helpers ───────────────────────────────────────────────────────────────────

type SnapResult = (Vec<(u64,u64)>, Vec<(u64,u64)>, Option<f64>, Option<f64>,
                   Option<crate::market::Trade>, bool, bool, u64, f64, Option<String>);

fn snap_book(p: &PolyPane, up: bool) -> SnapResult {
    let (book, stats) = if up { (&p.up_book, &p.up_stats) } else { (&p.down_book, &p.down_stats) };
    let n = 6;
    let (bids, asks, mid, imb, trade) = {
        let b = book.read();
        (b.top_bids(n), b.top_asks(n), b.mid_f64(), b.imbalance_top(10), b.last_trade)
    };
    let (conn, boot, msgs, lat, err) = {
        let s = stats.read();
        (s.connected, s.bootstrapped, s.msgs, s.latency_ewma_us, s.last_error.clone())
    };
    (bids, asks, mid, imb, trade, conn, boot, msgs, lat, err)
}

fn book_line(left: Option<&(u64,u64)>, right: Option<&(u64,u64)>, max_q: u64, half: usize, bar_w: usize, col: Color) -> Line<'static> {
    let side_str = |opt: Option<&(u64,u64)>| -> String {
        match opt {
            None => format!("{:<width$}", "—", width = half),
            Some((p, q)) => {
                let px = fmt::price(*p as f64 / POLY_PX_SCALE as f64, POLY_PX_SCALE);
                let bar = "█".repeat((bar_w as f64 * (*q as f64 / max_q as f64).clamp(0.0, 1.0)) as usize);
                let qty = fmt::qty(*q as f64 / POLY_QTY_SCALE as f64);
                format!("{px:>8}  {bar:<bar_w$}  {qty:>7}")
            }
        }
    };
    Line::from(vec![
        Span::styled(format!("{:<half$}", side_str(left)),  Style::default().fg(col)),
        Span::raw("  "),
        Span::styled(side_str(right), Style::default().fg(col)),
    ])
}

fn imb_str(imb: Option<f64>) -> String {
    match imb {
        Some(v) => format!("imb  {v:+.2}"),
        None    => "imb  —".into(),
    }
}

/// Standard-normal CDF via Abramowitz & Stegun (max error ~1.5 × 10⁻⁷)
fn normal_cdf(x: f64) -> f64 {
    let a = [0.254829592_f64, -0.284496736, 1.421413741, -1.453152027, 1.061405429];
    let p = 0.3275911_f64;
    let sign = if x < 0.0 { -1.0_f64 } else { 1.0 };
    let xabs = x.abs();
    let t = 1.0 / (1.0 + p * xabs);
    let poly = t * (a[0] + t * (a[1] + t * (a[2] + t * (a[3] + t * a[4]))));
    let phi  = (-(xabs * xabs) / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt() * poly;
    0.5 * (1.0 + sign * (1.0 - 2.0 * phi))
}

/// P(UP wins) = Φ( Δ / σ_remaining )
/// σ_per_15m ≈ 0.3%, annualised from typical BTC short-term vol
fn fair_value(btc_now: f64, price_to_beat: f64, secs_left: f64, duration: &str) -> Option<f64> {
    if price_to_beat <= 0.0 || secs_left <= 0.0 { return None; }
    let window_secs = match duration { "5m" => 300.0, "15m" => 900.0, "1h" => 3600.0, _ => 900.0 };
    let sigma_window = 0.003 * (window_secs / 900.0_f64).sqrt();
    let frac_left    = (secs_left / window_secs).clamp(0.0, 1.0);
    let sigma        = sigma_window * frac_left.sqrt();
    if sigma < 1e-9 { return None; }
    let delta = (btc_now - price_to_beat) / price_to_beat;
    Some(normal_cdf(delta / sigma).clamp(0.01, 0.99))
}

fn secs_remaining(end_date: &str) -> f64 {
    DateTime::parse_from_rfc3339(end_date)
        .map(|dt| (dt.with_timezone(&Utc) - Utc::now()).num_milliseconds() as f64 / 1_000.0)
        .unwrap_or(0.0)
        .max(0.0)
}

fn fmt_countdown(secs: f64) -> String {
    if secs <= 0.0 { return "CLOSED".into(); }
    let s = secs as u64;
    if s < 60 { format!("{s}s") }
    else      { format!("{}:{:02}", s / 60, s % 60) }
}

fn fmt_btc(p: f64) -> String {
    let t = p as u64;
    let frac = ((p - t as f64) * 100.0).round() as u64;
    format!("{},{:03}.{:02}", t / 1_000, t % 1_000, frac)
}
