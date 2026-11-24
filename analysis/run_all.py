"""Run every validation step in order and write docs/results/*.md and docs/figures/*.png.

Usage (from the project root):
    analysis/.venv/bin/python analysis/run_all.py
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "src"))

import audit  # noqa: E402
import coverage  # noqa: E402
import ml_forecast  # noqa: E402
import patterns  # noqa: E402
import sensitivity  # noqa: E402
import stability  # noqa: E402
import summary  # noqa: E402
from common import OUT, RES, RUNS, TICKETS_CSV, ensure_dirs  # noqa: E402

STEPS = [
    ("audit", audit.main),
    ("patterns", patterns.main),
    ("stability", stability.main),
    ("sensitivity", sensitivity.main),
    ("coverage", coverage.main),
    ("ml_forecast", ml_forecast.main),
    ("summary", summary.main),
]


def check_inputs() -> None:
    missing = [str(TICKETS_CSV)] if not TICKETS_CSV.exists() else []
    for run in RUNS:
        for f in ["segment_profiles.parquet", "stop_activity.parquet", "assigned_tickets.parquet", "recon_trips.parquet", "route_summary.csv", "meta.json", "match_report.json"]:
            if not (OUT / run / f).exists():
                missing.append(str(OUT / run / f))
    if missing:
        sys.exit("missing inputs (run the pipeline first):\n  " + "\n  ".join(missing))


def main() -> None:
    check_inputs()
    ensure_dirs()
    t_all = time.time()
    for name, fn in STEPS:
        t0 = time.time()
        print(f"[{name}] running")
        fn()
        print(f"[{name}] done in {time.time() - t0:.1f}s")
    print(f"all steps done in {time.time() - t_all:.1f}s; results in {RES}")


if __name__ == "__main__":
    main()
