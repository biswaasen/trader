# orderbook

A terminal tool for watching real-time order books — Binance spot pairs and Polymarket binary markets, side by side.

## what it does

- streams the Binance raw depth feed (not the 100ms aggregated one — every single order add/cancel in real time)
- streams Polymarket CLOB (the YES and NO books for up/down binary markets like BTC 15m, ETH 1h, etc.)
- shows a live ladder: asks above, bids below, with mid price, spread, and order book imbalance
- supports up to 4 panes at once — mix Binance pairs and Polymarket markets however you want

## build

you need Rust installed. if you don't have it yet:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
```

then build:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

takes about 60 seconds the first time. binary ends up at `./target/release/orderbook`.

## run

```bash
./target/release/orderbook
```

opens a picker. type to filter (e.g. `btc`, `eth`, `updown`). space to select, enter to launch. polymarket markets load in the background — the badge turns green when ready.

or skip the picker and go straight to a symbol:

```bash
./target/release/orderbook --symbol BTCUSDT
```

## keys

in the picker: `↑/↓` to move, any letter to filter, `space` to select, `enter` to start, `esc` to quit.

in the viewer: `q` or `esc` to quit.

## source layout

```
src/
├── main.rs
├── market/         order book data structure + price parsing
├── feed/
│   ├── binance/    raw @depth websocket + REST snapshot bootstrap
│   └── poly/       Gamma API discovery + CLOB websocket
└── tui/
    ├── selector/   fuzzy source picker
    └── viewer/     order book renderer (30fps)
```
