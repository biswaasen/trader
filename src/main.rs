mod feed;
mod market;
mod tui;

use anyhow::Result;
use crossterm::{execute, terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen}};
use feed::{binance, poly, Pane, PolyPane, Stats, View};
use market::{OrderBook, BINANCE_PX_SCALE, BINANCE_QTY_SCALE, POLY_PX_SCALE, POLY_QTY_SCALE};
use parking_lot::RwLock;
use ratatui::{prelude::*, Terminal};
use std::{io::stdout, sync::Arc};
use tui::selector::Selection;

fn main() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4).enable_all().thread_name("ob-worker").build()?
        .block_on(run())
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
        default_hook(info);
    }));

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, crossterm::cursor::Hide)?;
    let mut term = Terminal::new(CrosstermBackend::new(out))?;
    term.clear()?;

    let selections = match arg_flag(&args, "--symbol") {
        Some(s) => vec![Selection::Binance(s)],
        None    => tui::selector::run(&mut term).await?,
    };

    if selections.is_empty() { return teardown(&mut term); }

    let views = build_views(selections);
    let res   = tui::viewer::run(&mut term, views).await;
    teardown(&mut term)?;
    res
}

fn teardown(term: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    let _ = term.show_cursor();
    let _ = disable_raw_mode();
    let _ = execute!(term.backend_mut(), LeaveAlternateScreen, crossterm::cursor::Show);
    Ok(())
}

fn arg_flag(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn build_views(selections: Vec<Selection>) -> Vec<View> {
    let mut views: Vec<View> = Vec::new();
    // Track the BTCUSDT book handle so poly panes can read live BTC price
    let mut btc_book: Option<feed::BookHandle> = None;

    for sel in selections {
        match sel {
            Selection::Binance(sym) => {
                let book  = Arc::new(RwLock::new(OrderBook::new(BINANCE_PX_SCALE, BINANCE_QTY_SCALE)));
                let stats = Arc::new(RwLock::new(Stats::default()));
                binance::spawn(sym.clone(), book.clone(), stats.clone());
                if sym.to_uppercase() == "BTCUSDT" {
                    btc_book = Some(book.clone());
                }
                views.push(View::Exchange(Pane {
                    title:    format!("BINANCE · {sym}"),
                    subtitle: "raw @depth  ·  integer ticks".into(),
                    color:    Color::Yellow,
                    book, stats, group: None,
                }));
            }
            Selection::Poly(m) => {
                let up_book    = Arc::new(RwLock::new(OrderBook::new(POLY_PX_SCALE, POLY_QTY_SCALE)));
                let down_book  = Arc::new(RwLock::new(OrderBook::new(POLY_PX_SCALE, POLY_QTY_SCALE)));
                let up_stats   = Arc::new(RwLock::new(Stats::default()));
                let down_stats = Arc::new(RwLock::new(Stats::default()));
                poly::spawn(poly::Feed {
                    up_id:    m.up_token_id.clone(), down_id:   m.down_token_id.clone(),
                    up_book:  up_book.clone(),       down_book: down_book.clone(),
                    up_stats: up_stats.clone(),      down_stats: down_stats.clone(),
                });
                views.push(View::Poly(PolyPane {
                    title:         m.title,
                    asset:         m.asset,
                    duration:      m.duration,
                    end_date:      m.end_date,
                    price_to_beat: m.price_to_beat,
                    up_book, down_book, up_stats, down_stats,
                    btc_book: None, // wired below after all selections are processed
                }));
            }
        }
    }

    // Wire the BTCUSDT book into every poly pane so they can show live BTC price
    if let Some(btc) = &btc_book {
        for v in &mut views {
            if let View::Poly(p) = v { p.btc_book = Some(btc.clone()); }
        }
    }

    views.truncate(4);
    views
}
