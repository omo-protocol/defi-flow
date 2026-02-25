#!/usr/bin/env python3
"""Plot venue allocations from engine tick CSV output.

Usage:
    # Generate tick CSV from the engine:
    defi-flow backtest strategies/delta_neutral.json --tick-csv /tmp/v1_ticks.csv
    # Then plot:
    python scripts/plot_allocations.py /tmp/v1_ticks.csv --spot data/delta-neutral/hyperliquid_spot_eth.csv --perp data/delta-neutral/hyperliquid_eth.csv -o backtest_allocations.png
"""

import argparse
import csv
from datetime import datetime, timezone
from pathlib import Path

import matplotlib.pyplot as plt
import matplotlib.dates as mdates
import numpy as np


def load_csv(path):
    rows = []
    with open(path) as f:
        reader = csv.DictReader(f)
        for row in reader:
            rows.append(row)
    return rows


def ts_to_dt(ts):
    return datetime.fromtimestamp(int(ts), tz=timezone.utc)


def plot(tick_csv, spot_csv, perp_csv, output, title=None):
    ticks = load_csv(tick_csv)
    if not ticks:
        print("No tick data found")
        return

    # Parse tick data
    timestamps = [ts_to_dt(r["timestamp"]) for r in ticks]
    tvl = [float(r["tvl"]) for r in ticks]

    # Identify venue columns (everything except timestamp, tvl)
    venue_cols = [k for k in ticks[0].keys() if k not in ("timestamp", "tvl")]

    # Group venues by type for stacking
    venue_data = {}
    for col in venue_cols:
        venue_data[col] = [float(r[col]) for r in ticks]

    # Time range from tick data
    tick_ts = [int(r["timestamp"]) for r in ticks]
    ts_min, ts_max = min(tick_ts), max(tick_ts)

    # Load spot/perp prices (filter to tick time range)
    spot_prices, spot_timestamps = [], []
    perp_prices, perp_timestamps = [], []
    if spot_csv:
        for r in load_csv(spot_csv):
            t = int(r["timestamp"])
            if ts_min <= t <= ts_max:
                spot_timestamps.append(ts_to_dt(t))
                spot_prices.append(float(r["price"]))
    if perp_csv:
        for r in load_csv(perp_csv):
            t = int(r["timestamp"])
            if ts_min <= t <= ts_max:
                perp_timestamps.append(ts_to_dt(t))
                perp_prices.append(float(r["mark_price"]))

    # Color palette
    colors = {
        "buy_eth": "#4CAF50",
        "lend_eth": "#4CAF50",
        "short_eth": "#FF9800",
        "lend_idle": "#9C27B0",
        "lend_usdc": "#9C27B0",
    }
    labels = {
        "buy_eth": "Spot ETH",
        "lend_eth": "ETH Lending",
        "short_eth": "Short Perp",
        "lend_idle": "USDC Lending",
        "lend_usdc": "USDC Lending",
    }
    # Stacking order: spot/eth_lend first, then perp, then lending
    order = ["buy_eth", "lend_eth", "short_eth", "lend_idle", "lend_usdc"]
    venue_cols_ordered = [c for c in order if c in venue_cols]
    # Add any remaining columns
    for c in venue_cols:
        if c not in venue_cols_ordered:
            venue_cols_ordered.append(c)

    # Auto-detect title
    if title is None:
        has_lend_eth = "lend_eth" in venue_cols
        has_buy_eth = "buy_eth" in venue_cols
        if has_lend_eth:
            title = "v2: ETH Lending + Short Perp + USDC Lending  (group-aware rebalance)"
        elif has_buy_eth:
            title = "v1: Spot ETH + Short Perp + USDC Lending  (group-aware rebalance)"
        else:
            title = "Backtest Allocations"

    fig, axes = plt.subplots(
        3, 1, figsize=(14, 10), sharex=True,
        gridspec_kw={"height_ratios": [1, 1.2, 0.8]}
    )
    fig.suptitle(title, fontsize=14, fontweight="bold", y=0.98)

    # ── Panel 1: Prices ──────────────────────────────────────────────
    ax1 = axes[0]
    if spot_prices:
        ax1.plot(spot_timestamps, spot_prices, label="Spot", color="#2196F3", linewidth=1.2)
    if perp_prices:
        ax1.plot(perp_timestamps, perp_prices,
                 label="Perp Mark", color="#FF5722", linewidth=1, alpha=0.7)
    ax1.set_ylabel("ETH/USD")
    ax1.legend(loc="upper left", fontsize=9)
    ax1.grid(True, alpha=0.3)

    # ── Panel 2: Venue values (stacked) ──────────────────────────────
    ax2 = axes[1]
    bottom = np.zeros(len(timestamps))
    for col in venue_cols_ordered:
        vals = np.array(venue_data[col])
        c = colors.get(col, "#607D8B")
        lb = labels.get(col, col)
        ax2.fill_between(timestamps, bottom, bottom + vals, alpha=0.7, label=lb, color=c)
        bottom += vals
    ax2.plot(timestamps, tvl, color="black", linewidth=1.5, linestyle="--", label="TVL")
    ax2.set_ylabel("USD")
    ax2.legend(loc="upper left", fontsize=9)
    ax2.grid(True, alpha=0.3)

    # Annotate final TVL
    final_tvl = tvl[-1]
    initial_tvl = tvl[0]
    pnl_pct = (final_tvl / initial_tvl - 1) * 100 if initial_tvl > 0 else 0
    ax2.annotate(
        f"${final_tvl:,.0f} ({pnl_pct:+.1f}%)",
        xy=(timestamps[-1], final_tvl), xytext=(-100, 15), textcoords="offset points",
        fontsize=10, fontweight="bold",
        arrowprops=dict(arrowstyle="->", color="gray"),
    )

    # ── Panel 3: Allocation % ────────────────────────────────────────
    ax3 = axes[2]
    tvl_arr = np.maximum(np.array(tvl), 1.0)
    bottom = np.zeros(len(timestamps))
    for col in venue_cols_ordered:
        vals = np.array(venue_data[col])
        pcts = vals / tvl_arr * 100
        c = colors.get(col, "#607D8B")
        lb = labels.get(col, col)
        ax3.fill_between(timestamps, bottom, bottom + pcts, alpha=0.7, label=lb, color=c)
        bottom += pcts
    ax3.axhline(50, color="white", linewidth=0.5, alpha=0.5, linestyle="--")
    ax3.set_ylabel("%")
    ax3.set_ylim(0, 105)
    ax3.legend(loc="upper left", fontsize=9)
    ax3.grid(True, alpha=0.3)

    ax3.xaxis.set_major_formatter(mdates.DateFormatter("%b '%y"))
    ax3.xaxis.set_major_locator(mdates.MonthLocator())
    plt.xticks(rotation=30)
    plt.tight_layout()
    plt.savefig(output, dpi=150, bbox_inches="tight")
    print(f"Saved: {output}")
    plt.close()


def main():
    parser = argparse.ArgumentParser(description="Plot backtest venue allocations")
    parser.add_argument("tick_csv", help="Path to engine tick CSV output")
    parser.add_argument("--spot", help="Path to spot price CSV")
    parser.add_argument("--perp", help="Path to perp price CSV")
    parser.add_argument("-o", "--output", default="backtest_allocations.png")
    parser.add_argument("--title", help="Override chart title")
    args = parser.parse_args()

    plot(args.tick_csv, args.spot, args.perp, args.output, args.title)


if __name__ == "__main__":
    main()
