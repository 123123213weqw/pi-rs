#!/usr/bin/env python3
"""Generate the README hero comparison chart (pi vs pi-rs). → assets/comparison.png"""
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import os

DARK = "#0d1117"
FG = "#e6edf3"
MUTED = "#8b949e"
PI_C = "#8b949e"
RS_C = "#4cc38a"

rows = [
    # (label, pi display, rs display, pi value, rs value, unit)
    ("Peak RSS (real tasks)", "161–201 MB", "8–63 MB", 181, 35.5, "MB  (lower is better)"),
    ("Install size", "197 MB", "2.5 MB", 197, 2.5, "MB  (lower is better)"),
    ("Cold start", "910 ms", "2 ms", 910, 2, "ms  (lower is better)"),
    ("Concurrent agents in 16 GB", "~88", "~455", 88, 455, "sessions  (higher is better)"),
]

fig, ax = plt.subplots(figsize=(10, 5.2), dpi=160)
fig.patch.set_facecolor(DARK)
ax.set_facecolor(DARK)

y = list(range(len(rows)))[::-1]
bar_h = 0.32

for i, (label, pi_s, rs_s, pi_v, rs_v, unit) in enumerate(rows):
    yy = y[i]
    # log-ish scale: use value directly; rss 181 vs 35.5 fine; cold 910 vs 2 → rs invisible.
    scale = max(pi_v, rs_v)
    pw = max(pi_v / scale, 0.04)
    rw = max(rs_v / scale, 0.04)
    ax.barh(yy + bar_h / 2 + 0.02, pw, height=bar_h, color=PI_C, alpha=0.85)
    ax.barh(yy - bar_h / 2 - 0.02, rw, height=bar_h, color=RS_C)
    ax.text(pw + 0.012, yy + bar_h / 2 + 0.02, f"pi  {pi_s}", va="center", fontsize=10,
            color=FG, family="monospace")
    ax.text(rw + 0.012, yy - bar_h / 2 - 0.02, f"pi-rs  {rs_s}", va="center", fontsize=10,
            color=RS_C, family="monospace", fontweight="bold")
    ax.text(-0.015, yy, label, va="center", ha="right", fontsize=11.5, color=FG)
    ax.text(1.0, yy + 0.52, unit, va="center", ha="right", fontsize=8, color=MUTED)

ax.set_xlim(0, 1.12)
ax.set_ylim(-0.7, len(rows) + 0.9)
ax.axis("off")

ty = len(rows) - 0.2
ax.text(0.02, ty + 0.35, "pi-rs", fontsize=20, color=RS_C, fontweight="bold", family="monospace")
ax.text(0.16, ty + 0.35, "— same behavior, less baggage", fontsize=13, color=FG, family="monospace")
ax.text(0.02, ty, "behavioral clone of pi 0.74.2:  85/85 behavior atoms identical  ·  arena 102/104 vs 102/104 (same failures)",
        fontsize=9, color=MUTED, family="monospace")

out = os.path.join(os.path.dirname(__file__), "comparison.png")
fig.savefig(out, bbox_inches="tight", facecolor=DARK, pad_inches=0.25)
print(f"saved {out}")
