use crate::market::POLY_PX_SCALE;
use ratatui::style::Color;

pub fn price(p: f64, scale: u64) -> String {
    if scale == POLY_PX_SCALE { return format!("{:.2}¢", p * 100.0); }
    if p >= 1_000.0 { format!("{},{:07.2}", (p / 1000.0) as u64, p % 1000.0) }
    else if p >= 1.0 { format!("{:.4}", p) }
    else             { format!("{:.6}", p) }
}

pub fn qty(q: f64) -> String {
    if q >= 1000.0     { format!("{:.1}k", q / 1000.0) }
    else if q >= 1.0   { format!("{:.3}", q) }
    else if q >= 0.001 { format!("{:.4}", q) }
    else               { format!("{:.6}", q) }
}

pub fn human(n: u64) -> String {
    if n >= 1_000_000_000 { format!("{:.1}B", n as f64 / 1e9) }
    else if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1e6) }
    else if n >= 10_000    { format!("{:.1}k", n as f64 / 1e3) }
    else                   { format!("{n}") }
}

pub fn latency_color(us: f64) -> Color {
    let ms = us / 1_000.0;
    if ms < 100.0 { Color::Green } else if ms < 300.0 { Color::Yellow } else { Color::Red }
}

pub fn imb_color(i: Option<f64>) -> Color {
    match i {
        Some(v) if v >  0.15 => Color::Green,
        Some(v) if v < -0.15 => Color::Red,
        _                    => Color::Gray,
    }
}
