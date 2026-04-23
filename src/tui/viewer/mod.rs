mod fmt;
mod pane;
mod poly;

use crate::feed::View;
use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::prelude::*;
use std::io::Stdout;
use std::time::Duration;

pub async fn run(term: &mut Terminal<CrosstermBackend<Stdout>>, views: Vec<View>) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick   = tokio::time::interval(Duration::from_millis(33));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = tick.tick() => { term.draw(|f| draw(f, &views))?; }
            Some(Ok(Event::Key(k))) = events.next() => {
                if k.kind != KeyEventKind::Press { continue; }
                if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c')) { return Ok(()); }
                if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) { return Ok(()); }
            }
        }
    }
}

fn draw(f: &mut Frame, views: &[View]) {
    for (v, r) in views.iter().zip(split(f.area(), views.len()).iter()) {
        match v {
            View::Exchange(p) => pane::draw(f, *r, p),
            View::Poly(p)     => poly::draw(f, *r, p),
        }
    }
}

fn split(a: Rect, n: usize) -> Vec<Rect> {
    match n {
        1 => vec![a],
        2 => Layout::horizontal([Constraint::Percentage(50); 2]).split(a).to_vec(),
        3 => Layout::horizontal([Constraint::Ratio(1,3); 3]).split(a).to_vec(),
        4 => {
            let rows = Layout::vertical([Constraint::Percentage(50); 2]).split(a);
            let top  = Layout::horizontal([Constraint::Percentage(50); 2]).split(rows[0]);
            let bot  = Layout::horizontal([Constraint::Percentage(50); 2]).split(rows[1]);
            vec![top[0], top[1], bot[0], bot[1]]
        }
        _ => vec![a],
    }
}
