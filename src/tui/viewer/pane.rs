use super::fmt;
use crate::feed::{now_us, Pane};
use crate::market::Side;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn draw(f: &mut Frame, area: Rect, pane: &Pane) {
    let (bids, asks, mid, spread, imb, seq, upd, last_trade, px_sc, qty_sc) = {
        let b = pane.book.read();
        let n = ((area.height.saturating_sub(5) / 2) as usize).max(3).min(24);
        (b.top_bids(n), b.top_asks(n), b.mid_f64(), b.spread_bps(), b.imbalance_top(10),
         b.seq, b.updates, b.last_trade, b.px_scale, b.qty_scale)
    };
    let (conn, msgs, lat_us, last_us, boot, recon, err) = {
        let s = pane.stats.read();
        (s.connected, s.msgs, s.latency_ewma_us, s.last_msg_us, s.bootstrapped, s.reconnects, s.last_error.clone())
    };

    let status_str = if !conn { "DISCONNECTED" } else if !boot { "syncing…" } else { "live" };
    let status_col = if conn && boot { Color::Green } else if conn { Color::Yellow } else { Color::Red };
    let mut bottom = vec![
        Span::raw(" "),
        Span::styled(format!("{} msgs", fmt::human(msgs)), Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(format!("seq {}", fmt::human(seq)), Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(format!("lat {:.0}ms", lat_us / 1_000.0), Style::default().fg(fmt::latency_color(lat_us))),
        Span::raw("  "),
        Span::styled(format!("upd {}", fmt::human(upd)), Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(status_str, Style::default().fg(status_col)),
    ];
    if recon > 0 { bottom.push(Span::styled(format!("  ↻{recon}"), Style::default().fg(Color::Yellow))); }
    bottom.push(Span::raw(" "));

    let block = Block::default().borders(Borders::ALL)
        .border_style(Style::default().fg(if conn { pane.color } else { Color::DarkGray }))
        .title(Line::from(vec![Span::raw(" "), Span::styled(&pane.title, Style::default().fg(pane.color).add_modifier(Modifier::BOLD)), Span::raw(" ")]))
        .title_bottom(Line::from(bottom));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let body = if inner.height > 8 && !pane.subtitle.is_empty() {
        let v = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).split(inner);
        f.render_widget(Paragraph::new(Line::from(Span::styled(&pane.subtitle, Style::default().fg(Color::DarkGray)))), v[0]);
        v[1]
    } else { inner };

    if let Some(e) = &err {
        if bids.is_empty() && asks.is_empty() {
            f.render_widget(Paragraph::new(vec![
                Line::from(Span::styled("connection error:", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from(Span::styled(e.clone(), Style::default().fg(Color::Yellow))),
                Line::from(""),
                Line::from(Span::styled("retrying…", Style::default().fg(Color::DarkGray))),
            ]), body);
            return;
        }
    }

    let w = body.width as usize;
    let (pw, sw) = (12usize, 12usize);
    let bw = w.saturating_sub(pw + sw + 4);
    let max_q = bids.iter().map(|(_,q)|*q).chain(asks.iter().map(|(_,q)|*q)).max().unwrap_or(1).max(1);

    let mut lines: Vec<Line> = Vec::with_capacity(bids.len() + asks.len() + 4);
    lines.push(Line::from(vec![
        Span::styled(format!("{:>pw$}", "price"), Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(format!("{:>sw$}", "size"), Style::default().fg(Color::DarkGray)),
        Span::raw("  depth"),
    ]));

    for (p,q) in asks.iter().rev() { lines.push(level_line(*p, *q, max_q, bw, pw, sw, Color::Red,   px_sc, qty_sc)); }

    let mid_s    = mid.map(|m| fmt::price(m, px_sc)).unwrap_or_else(|| "—".into());
    let spread_s = spread.map(|s| format!("{s:.2} bp")).unwrap_or_else(|| "—".into());
    let imb_s    = imb.map(|i| format!("{i:+.2}")).unwrap_or_else(|| "—".into());
    lines.push(Line::from(vec![
        Span::styled("  mid ",     Style::default().fg(Color::DarkGray)),
        Span::styled(mid_s,        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled(" · spread ", Style::default().fg(Color::DarkGray)),
        Span::styled(spread_s,     Style::default().fg(Color::Cyan)),
        Span::styled(" · imb ",    Style::default().fg(Color::DarkGray)),
        Span::styled(imb_s,        Style::default().fg(fmt::imb_color(imb))),
    ]));

    for (p,q) in bids.iter() { lines.push(level_line(*p, *q, max_q, bw, pw, sw, Color::Green, px_sc, qty_sc)); }

    lines.push(Line::from(""));
    if let Some(t) = last_trade {
        let age_ms = now_us().saturating_sub(t.ts_us) / 1_000;
        let (ss, sc) = if matches!(t.side, Side::Buy) { ("BUY", Color::Green) } else { ("SELL", Color::Red) };
        lines.push(Line::from(vec![
            Span::styled("  last ", Style::default().fg(Color::DarkGray)),
            Span::styled(ss, Style::default().fg(sc).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(fmt::price(t.price_u as f64 / px_sc as f64, px_sc), Style::default().fg(Color::White)),
            Span::styled(format!("  ×{}", fmt::qty(t.qty_u as f64 / qty_sc as f64)), Style::default().fg(Color::DarkGray)),
            Span::styled(format!("  ({age_ms} ms ago)"), Style::default().fg(Color::DarkGray)),
        ]));
    } else if !bids.is_empty() {
        let age = if last_us == 0 { 0 } else { now_us().saturating_sub(last_us) / 1_000 };
        lines.push(Line::from(Span::styled(format!("  last update {age} ms ago"), Style::default().fg(Color::DarkGray))));
    }

    f.render_widget(Paragraph::new(lines), body);
}

fn level_line(px: u64, qty: u64, max_q: u64, bw: usize, pw: usize, sw: usize, color: Color, px_sc: u64, qty_sc: u64) -> Line<'static> {
    let bar = "█".repeat((bw as f64 * (qty as f64 / max_q as f64).clamp(0.0, 1.0)) as usize);
    Line::from(vec![
        Span::styled(format!("{:>pw$}", fmt::price(px as f64 / px_sc as f64, px_sc)), Style::default().fg(color)),
        Span::raw("  "),
        Span::styled(format!("{:>sw$}", fmt::qty(qty as f64 / qty_sc as f64)), Style::default().fg(Color::Gray)),
        Span::raw("  "),
        Span::styled(bar, Style::default().fg(color).add_modifier(Modifier::DIM)),
    ])
}
