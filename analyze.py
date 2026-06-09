#!/usr/bin/env python3
"""Analysis of matching-engine output (book.csv + trades.csv).

Run:
    cargo run --release        # generates trades.csv and book.csv
    python3 analyze.py         # computes stats and saves plots to plots/

Dependencies: pandas, matplotlib.
"""

from pathlib import Path

import matplotlib

matplotlib.use("Agg")  # no display — write plots to files
import matplotlib.pyplot as plt
import pandas as pd

PLOTS = Path("plots")


def load() -> tuple[pd.DataFrame, pd.DataFrame]:
    book = pd.read_csv("book.csv")
    trades = pd.read_csv("trades.csv")
    return book, trades


def summary(book: pd.DataFrame, trades: pd.DataFrame) -> None:
    print("=== book.csv ===")
    print(f"rows: {len(book):,}")
    print(f"two-sided book (has mid): {book['mid'].notna().sum():,}")
    print(book[["spread", "mid", "bid_qty", "ask_qty"]].describe().round(2))

    print("\n=== trades.csv ===")
    print(f"trades: {len(trades):,}")
    print(f"total volume: {trades['qty'].sum():,}")
    by_side = trades["aggressor"].value_counts()
    print("by aggressor:")
    print(by_side)
    print("\ntrade size (qty):")
    print(trades["qty"].describe().round(2))


def plot(book: pd.DataFrame, trades: pd.DataFrame) -> None:
    PLOTS.mkdir(exist_ok=True)

    # 1. Price series: mid + best bid/ask over time.
    fig, ax = plt.subplots(figsize=(12, 4))
    ax.plot(book["seq"], book["mid"], lw=0.6, label="mid")
    ax.plot(book["seq"], book["best_bid"], lw=0.4, alpha=0.6, label="best bid")
    ax.plot(book["seq"], book["best_ask"], lw=0.4, alpha=0.6, label="best ask")
    ax.set(xlabel="event", ylabel="price (ticks)", title="Price series")
    ax.legend()
    fig.tight_layout()
    fig.savefig(PLOTS / "price.png", dpi=110)
    plt.close(fig)

    # 2. Spread distribution (log Y axis — heavy tail).
    fig, ax = plt.subplots(figsize=(8, 4))
    book["spread"].dropna().plot.hist(bins=80, ax=ax, log=True)
    ax.set(xlabel="spread (ticks)", title="Spread distribution")
    fig.tight_layout()
    fig.savefig(PLOTS / "spread.png", dpi=110)
    plt.close(fig)

    # 3. Trade size distribution (heavy tail).
    fig, ax = plt.subplots(figsize=(8, 4))
    trades["qty"].plot.hist(bins=80, ax=ax, log=True)
    ax.set(xlabel="trade size (qty)", title="Trade size distribution")
    fig.tight_layout()
    fig.savefig(PLOTS / "trade_size.png", dpi=110)
    plt.close(fig)

    # 4. Depth at touch over time (bid vs ask).
    fig, ax = plt.subplots(figsize=(12, 4))
    ax.plot(book["seq"], book["bid_qty"], lw=0.5, label="bid qty")
    ax.plot(book["seq"], book["ask_qty"], lw=0.5, label="ask qty")
    ax.set(xlabel="event", ylabel="volume at touch", title="Depth at best price")
    ax.legend()
    fig.tight_layout()
    fig.savefig(PLOTS / "depth.png", dpi=110)
    plt.close(fig)

    print(f"\nplots saved to {PLOTS}/")


def main() -> None:
    book, trades = load()
    summary(book, trades)
    plot(book, trades)


if __name__ == "__main__":
    main()