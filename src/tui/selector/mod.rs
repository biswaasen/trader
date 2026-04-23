mod draw;

use crate::feed::poly::{discover, Market};
use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use parking_lot::Mutex;
use ratatui::{prelude::*, widgets::ListState};
use std::sync::Arc;
use std::time::Duration;

const SYMBOLS: &[&str] = &[
    "BTCUSDT","ETHUSDT","SOLUSDT","BNBUSDT","XRPUSDT","DOGEUSDT","ADAUSDT","AVAXUSDT",
    "LINKUSDT","DOTUSDT","LTCUSDT","TRXUSDT","NEARUSDT","ATOMUSDT","ARBUSDT","OPUSDT",
    "SUIUSDT","TIAUSDT","INJUSDT","BCHUSDT","APTUSDT","HBARUSDT","TONUSDT","WIFUSDT",
    "PEPEUSDT","SHIBUSDT","RENDERUSDT","FETUSDT","BONKUSDT","UNIUSDT","AAVEUSDT","ETHBTC",
];

#[derive(Clone, Debug)]
pub enum Selection { Binance(String), Poly(Market) }

#[derive(Clone, Debug)]
pub enum Choice { Binance(String), Poly(Market) }

impl Choice {
    pub fn label(&self) -> String {
        match self {
            Choice::Binance(s) => format!("binance     {s}"),
            Choice::Poly(m)    => format!("polymarket  {} · {} {} · {}  (UP {:.0}¢ / DOWN {:.0}¢)",
                m.asset, m.asset, m.duration, draw::remaining(&m.end_date), m.up_price*100.0, m.down_price*100.0),
        }
    }
    pub fn search_text(&self) -> String {
        match self {
            Choice::Binance(s) => s.to_lowercase(),
            Choice::Poly(m)    => format!("{} {} {} {}", m.asset, m.duration, m.title, m.event_slug).to_lowercase(),
        }
    }
}

pub struct PickerState {
    pub choices:      Vec<Choice>,
    pub filtered:     Vec<usize>,
    pub selected:     Vec<usize>,
    pub cursor:       usize,
    pub filter:       String,
    pub poly_loading: bool,
    pub poly_error:   Option<String>,
    pub list_state:   ListState,
}

impl PickerState {
    fn new() -> Self {
        let choices: Vec<Choice> = SYMBOLS.iter().map(|s| Choice::Binance(s.to_string())).collect();
        let filtered = (0..choices.len()).collect();
        Self { choices, filtered, selected: vec![], cursor: 0, filter: String::new(),
               poly_loading: true, poly_error: None, list_state: Default::default() }
    }
    pub fn refilter(&mut self) {
        let n = self.filter.to_lowercase();
        self.filtered = self.choices.iter().enumerate()
            .filter(|(_,c)| n.is_empty() || c.search_text().contains(&n))
            .map(|(i,_)| i).collect();
        if self.cursor >= self.filtered.len() { self.cursor = self.filtered.len().saturating_sub(1); }
    }
    pub fn move_cursor(&mut self, d: i32) {
        if self.filtered.is_empty() { self.cursor = 0; return; }
        self.cursor = (self.cursor as i32 + d).clamp(0, self.filtered.len() as i32 - 1) as usize;
    }
    pub fn toggle(&mut self) {
        let Some(&idx) = self.filtered.get(self.cursor) else { return };
        if let Some(p) = self.selected.iter().position(|&i| i == idx) { self.selected.remove(p); }
        else if self.selected.len() < 4 { self.selected.push(idx); }
    }
    fn confirmed(&self) -> Vec<Selection> {
        let idxs = if self.selected.is_empty() {
            self.filtered.get(self.cursor).map(|&i| vec![i]).unwrap_or_default()
        } else { self.selected.clone() };
        idxs.iter().map(|&i| match &self.choices[i] {
            Choice::Binance(s) => Selection::Binance(s.clone()),
            Choice::Poly(m)    => Selection::Poly(m.clone()),
        }).collect()
    }
}

enum Action { None, Confirm, Quit }

fn handle_key(k: KeyEvent, s: &mut PickerState) -> Action {
    if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c')) { return Action::Quit; }
    match k.code {
        KeyCode::Esc        => Action::Quit,
        KeyCode::Enter      => Action::Confirm,
        KeyCode::Up         => { s.move_cursor(-1);  Action::None }
        KeyCode::Down       => { s.move_cursor(1);   Action::None }
        KeyCode::PageUp     => { s.move_cursor(-10); Action::None }
        KeyCode::PageDown   => { s.move_cursor(10);  Action::None }
        KeyCode::Home       => { s.cursor = 0; Action::None }
        KeyCode::End        => { s.cursor = s.filtered.len().saturating_sub(1); Action::None }
        KeyCode::Char(' ')  => { s.toggle(); Action::None }
        KeyCode::Char(c)    => { s.filter.push(c); s.refilter(); Action::None }
        KeyCode::Backspace  => { s.filter.pop(); s.refilter(); Action::None }
        _                   => Action::None,
    }
}

pub async fn run(term: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<Vec<Selection>> {
    let slot: Arc<Mutex<Option<Result<Vec<Market>>>>> = Arc::new(Mutex::new(None));
    { let s = slot.clone(); tokio::spawn(async move { *s.lock() = Some(discover().await); }); }

    let mut state  = PickerState::new();
    let mut events = EventStream::new();
    let mut tick   = tokio::time::interval(Duration::from_millis(100));

    loop {
        if state.poly_loading {
            if let Some(r) = slot.lock().take() {
                state.poly_loading = false;
                match r {
                    Ok(mkts) => { for m in mkts { state.choices.push(Choice::Poly(m)); } state.refilter(); }
                    Err(e)   => state.poly_error = Some(format!("{:#}", e)),
                }
            }
        }
        term.draw(|f| draw::draw(f, &mut state))?;
        tokio::select! {
            _ = tick.tick() => {}
            Some(Ok(Event::Key(k))) = events.next() => {
                if k.kind != KeyEventKind::Press { continue; }
                match handle_key(k, &mut state) {
                    Action::Quit    => return Ok(vec![]),
                    Action::Confirm => return Ok(state.confirmed()),
                    Action::None    => {}
                }
            }
        }
    }
}
