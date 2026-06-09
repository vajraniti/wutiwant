# orderbook

A limit order book **matching engine** written in Rust, with zero dependencies.

The engine ingests a stream of commands (place / cancel), matches orders by
**price-time priority**, and emits a stream of events (accepted, trade,
cancelled, rejected). A synthetic generator drives a realistic order flow
through the real book, and a small driver dumps the result to CSV for offline
analysis.

## Features

- Price-time priority matching (FIFO within a price level)
- Self-Trade Prevention with three modes: expire taker, expire maker, expire both
- Slab-based order pool with generation indices (use-after-free safe)
- Fixed-size price ladder with cached best price — O(1) level access
- Deterministic synthetic order-flow generator (reproducible from a seed)
- Zero external dependencies; custom SplitMix64 PRNG
- 45 unit tests covering all layers

## Quick start

Requires a Rust toolchain (`cargo`).

```powershell
# clone
git clone https://github.com/vajraniti/wutiwant.git
cd wutiwant

# run the simulation (writes trades.csv and book.csv)
cargo run --release

# run the test suite
cargo test
```

Simulation parameters (seed, number of steps, pool capacity) live in
`SimConfig::default()` in `src/sim.rs`. They are compiled into the binary, so
change them there and rebuild.

## Analysis (optional)

The Python client reads the CSV dumps and produces summary stats and plots.

```powershell
pip install pandas matplotlib
python analyze.py
```

Plots are written to `plots/` (price series, spread distribution, trade-size
distribution, depth at touch).

## Project layout

```
src/
  types.rs      domain types (Price, Qty, Side, OrderId, SubaccountId, SelfTradeMode)
  order.rs      the Order struct + intrusive list links
  ops.rs        input commands (Op::Place, Op::Cancel)
  events.rs     output events (Event) and reject reasons
  pool.rs       order pool (slab + generation) — order storage
  level.rs      a single price level (FIFO queue of orders)
  ladder.rs     the price ladder (array of levels) + best-price cache
  orderbook.rs  the core: apply(), buy/sell matching, STP
  generator.rs  synthetic command-flow generator
  rng.rs        deterministic PRNG (SplitMix64)
  sim.rs        driver: runs the flow and dumps CSV
  main.rs       entry point
analyze.py      Python client: stats + plots from the CSV dumps
```

## Notes

This is a computational core, not a standalone service: it has no network layer
and keeps all state in memory (as real matching engines do, for latency). It is
meant to be embedded into a host system (a gateway that speaks the wire protocol
and feeds it `Op`s). A persistence/event-log layer and a transport layer are the
natural next steps.

The order flow is synthetic — generated for correctness and load testing, not
sourced from a real market.
