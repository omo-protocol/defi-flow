#!/usr/bin/env python3
"""Plot backtest trajectory and PnL breakdown from JSON output."""

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

import matplotlib.pyplot as plt
import matplotlib.dates as mdates
import numpy as np


def load_data(path: str) -> dict:
    with open(path) as f:
        return json.load(f)


def ts_to_dates(timestamps: list[int]) -> list[datetime]:
    return [datetime.fromtimestamp(t, tz=timezone.utc) for t in timestamps]


def plot_trajectory(data: dict, out_dir: Path):
    hist = data["historical"]
    traj = hist["trajectory"]
    timestamps = [p["timestamp"] for p in traj]
    tvls = [p["tvl"] for p in traj]
    dates = ts_to_dates(timestamps)

    fig, ax = plt.subplots(figsize=(14, 6))

    # MC confidence bands
    mc = data.get("monte_carlo")
    if mc and mc["simulations"]:
        sims = mc["simulations"]
        n_ticks = len(tvls)
        # Collect TVL trajectories from all sims (aligned by tick index)
        sim_matrix = []
        for s in sims:
            st = s.get("trajectory", [])
            if len(st) >= n_ticks:
                sim_matrix.append([p["tvl"] for p in st[:n_ticks]])
        if sim_matrix:
            arr = np.array(sim_matrix)
            p5 = np.percentile(arr, 5, axis=0)
            p25 = np.percentile(arr, 25, axis=0)
            p75 = np.percentile(arr, 75, axis=0)
            p95 = np.percentile(arr, 95, axis=0)
            ax.fill_between(dates, p5, p95, alpha=0.12, color="steelblue", label="MC 5th-95th")
            ax.fill_between(dates, p25, p75, alpha=0.25, color="steelblue", label="MC 25th-75th")

    ax.plot(dates, tvls, color="#1a1a2e", linewidth=1.8, label="Historical")
    ax.axhline(y=tvls[0], color="gray", linestyle="--", linewidth=0.8, alpha=0.6)

    ax.set_title(hist["label"], fontsize=14, fontweight="bold")
    ax.set_xlabel("Date")
    ax.set_ylabel("Portfolio Value (USD)")
    ax.xaxis.set_major_formatter(mdates.DateFormatter("%b %Y"))
    ax.xaxis.set_major_locator(mdates.MonthLocator())
    fig.autofmt_xdate()
    ax.legend(loc="upper left")
    ax.grid(True, alpha=0.3)

    # Annotate final value
    final_tvl = tvls[-1]
    pnl_pct = (final_tvl / tvls[0] - 1) * 100
    ax.annotate(
        f"${final_tvl:,.0f} ({pnl_pct:+.1f}%)",
        xy=(dates[-1], final_tvl),
        xytext=(-80, 15),
        textcoords="offset points",
        fontsize=10,
        fontweight="bold",
        arrowprops=dict(arrowstyle="->", color="gray"),
    )

    # Stats box
    stats_text = (
        f"TWRR: {hist['twrr_pct']:+.2f}%\n"
        f"Ann.: {hist['annualized_pct']:+.2f}%\n"
        f"Max DD: {hist['max_drawdown_pct']:.2f}%\n"
        f"Sharpe: {hist['sharpe']:.3f}"
    )
    ax.text(
        0.98, 0.02, stats_text,
        transform=ax.transAxes, fontsize=9, verticalalignment="bottom",
        horizontalalignment="right", fontfamily="monospace",
        bbox=dict(boxstyle="round,pad=0.4", facecolor="white", edgecolor="gray", alpha=0.9),
    )

    plt.tight_layout()
    out_path = out_dir / "trajectory.png"
    fig.savefig(out_path, dpi=150)
    plt.close(fig)
    print(f"  Saved {out_path}")


def plot_pnl_breakdown(data: dict, out_dir: Path):
    hist = data["historical"]

    categories = ["Funding", "Lending", "Rewards", "Premium", "LP Fees", "Swap Costs"]
    values = [
        hist["funding_pnl"],
        hist["lending_interest"],
        hist["rewards_pnl"],
        hist["premium_pnl"],
        hist["lp_fees"],
        -hist["swap_costs"],  # costs are positive, flip for net view
    ]

    colors = ["#2ecc71" if v >= 0 else "#e74c3c" for v in values]

    fig, ax = plt.subplots(figsize=(10, 5))
    bars = ax.bar(categories, values, color=colors, edgecolor="white", linewidth=0.5)

    for bar, val in zip(bars, values):
        y_pos = bar.get_height()
        ax.text(
            bar.get_x() + bar.get_width() / 2,
            y_pos + (5 if val >= 0 else -15),
            f"${val:+,.2f}",
            ha="center", va="bottom" if val >= 0 else "top",
            fontsize=10, fontweight="bold",
        )

    ax.axhline(y=0, color="gray", linewidth=0.8)
    ax.set_title(f"{hist['label']} - PnL Breakdown", fontsize=14, fontweight="bold")
    ax.set_ylabel("USD")
    ax.grid(True, axis="y", alpha=0.3)

    # Net PnL annotation
    ax.text(
        0.98, 0.95,
        f"Net PnL: ${hist['net_pnl']:+,.2f}",
        transform=ax.transAxes, fontsize=12, fontweight="bold",
        ha="right", va="top",
        bbox=dict(boxstyle="round,pad=0.4", facecolor="lightyellow", edgecolor="gray"),
    )

    plt.tight_layout()
    out_path = out_dir / "pnl_breakdown.png"
    fig.savefig(out_path, dpi=150)
    plt.close(fig)
    print(f"  Saved {out_path}")


def plot_mc_distribution(data: dict, out_dir: Path):
    mc = data.get("monte_carlo")
    if not mc or not mc["simulations"]:
        return

    sims = mc["simulations"]
    hist = data["historical"]

    fig, axes = plt.subplots(1, 3, figsize=(16, 5))

    # TWRR distribution
    twrrs = [s["twrr_pct"] for s in sims]
    axes[0].hist(twrrs, bins=40, color="steelblue", edgecolor="white", alpha=0.8)
    axes[0].axvline(hist["twrr_pct"], color="red", linewidth=2, label=f"Historical: {hist['twrr_pct']:+.1f}%")
    axes[0].set_title("TWRR Distribution")
    axes[0].set_xlabel("TWRR (%)")
    axes[0].legend()

    # Max Drawdown distribution
    dds = [s["max_drawdown_pct"] for s in sims]
    axes[1].hist(dds, bins=40, color="coral", edgecolor="white", alpha=0.8)
    axes[1].axvline(hist["max_drawdown_pct"], color="red", linewidth=2, label=f"Historical: {hist['max_drawdown_pct']:.2f}%")
    axes[1].set_title("Max Drawdown Distribution")
    axes[1].set_xlabel("Max Drawdown (%)")
    axes[1].legend()

    # Sharpe distribution
    sharpes = [s["sharpe"] for s in sims]
    axes[2].hist(sharpes, bins=40, color="mediumseagreen", edgecolor="white", alpha=0.8)
    axes[2].axvline(hist["sharpe"], color="red", linewidth=2, label=f"Historical: {hist['sharpe']:.3f}")
    axes[2].set_title("Sharpe Ratio Distribution")
    axes[2].set_xlabel("Sharpe")
    axes[2].legend()

    for ax in axes:
        ax.grid(True, alpha=0.3)

    fig.suptitle(f"{hist['label']} - Monte Carlo ({mc['n_simulations']} sims)", fontsize=13, fontweight="bold")
    plt.tight_layout()
    out_path = out_dir / "mc_distributions.png"
    fig.savefig(out_path, dpi=150)
    plt.close(fig)
    print(f"  Saved {out_path}")


def main():
    if len(sys.argv) < 2:
        print("Usage: plot_backtest.py <backtest_output.json> [output_dir]")
        sys.exit(1)

    json_path = sys.argv[1]
    out_dir = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(json_path).parent

    data = load_data(json_path)
    plot_trajectory(data, out_dir)
    plot_pnl_breakdown(data, out_dir)
    plot_mc_distribution(data, out_dir)
    print("  Done.")


if __name__ == "__main__":
    main()
