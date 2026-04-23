use super::{Choice, PickerState};
use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

pub fn draw(f: &mut Frame, s: &mut PickerState) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
        Constraint::Length(3),
    ]).split(area);

    let poly_status = if s.poly_loading {
        Span::styled("loading Polymarket…", Style::default().fg(Color::Yellow))
    } else if let Some(err) = &s.poly_error {
        Span::styled(format!("poly err: {err}"), Style::default().fg(Color::Red))
    } else {
        Span::styled("Polymarket ready", Style::default().fg(Color::Green))
    };

    f.render_widget(Paragraph::new(Line::from(vec![
        Span::styled("  ORDERBOOK  ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled("pick up to 4 sources", Style::default().fg(Color::DarkGray)),
        Span::raw("   "),
        Span::styled(
            format!("{} selected", s.selected.len()),
            Style::default().fg(if s.selected.is_empty() { Color::DarkGray } else { Color::Green }),
        ),
        Span::raw("   "),
        poly_status,
    ])).block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(Color::DarkGray))),
    chunks[0]);

    let items: Vec<ListItem> = s.filtered.iter().map(|&i| {
        let c = &s.choices[i];
        let is_sel = s.selected.contains(&i);
        let marker = if is_sel { "● " } else { "○ " };
        let style = if is_sel { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) }
                    else      { Style::default().fg(Color::Gray) };
        let pfx_color = match c { Choice::Binance(_) => Color::Yellow, Choice::Poly(_) => Color::Magenta };
        let text = c.label();
        let (pfx, rest) = text.split_once("  ").unwrap_or((text.as_str(), ""));
        ListItem::new(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(format!("{:<10}", pfx), Style::default().fg(pfx_color).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {}", rest.trim_start()), style),
        ]))
    }).collect();

    s.list_state.select(Some(s.cursor));
    f.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" SOURCES ").border_style(Style::default().fg(Color::DarkGray)))
            .highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol("› "),
        chunks[1], &mut s.list_state,
    );

    f.render_widget(Paragraph::new(Line::from(vec![
        Span::styled(" /  ", Style::default().fg(Color::Cyan)),
        Span::raw(&s.filter),
        Span::styled("▌", Style::default().fg(Color::Cyan)),
    ])).block(Block::default().borders(Borders::ALL).title(" FILTER ").border_style(Style::default().fg(Color::DarkGray))),
    chunks[2]);

    f.render_widget(Paragraph::new(Line::from(vec![
        Span::styled(" ↑/↓ ", Style::default().fg(Color::Cyan)), Span::styled("cursor  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" Space ", Style::default().fg(Color::Cyan)), Span::styled("toggle  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" Enter ", Style::default().fg(Color::Cyan)), Span::styled("start   ", Style::default().fg(Color::DarkGray)),
        Span::styled(" Esc ",   Style::default().fg(Color::Cyan)), Span::styled("quit",    Style::default().fg(Color::DarkGray)),
    ])).wrap(Wrap { trim: true })
    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray))),
    chunks[3]);
}

pub fn remaining(iso: &str) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(iso) else { return "?".into() };
    let secs = (dt.with_timezone(&Utc) - Utc::now()).num_seconds();
    if secs <= 0 { return "expired".into(); }
    let (mm, ss) = (secs / 60, secs % 60);
    if mm >= 60 { format!("{}h{:02}m", mm/60, mm%60) } else { format!("{mm:>2}m{ss:02}s") }
}
