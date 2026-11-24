#!/usr/bin/env bash
# Rebuild every model run used by the validation suite, then rerun the suite.
# Bus supply is calibrated on the live feed snapshots in data/raw/rt (scripts/rt_collect.sh);
# without them the runs fall back to the timetable as published.
set -euo pipefail
cd "$(dirname "$0")/../crowding"
RT=()
if [ -d ../data/raw/rt ] && [ -n "$(find ../data/raw/rt -name '*.pb*' -print -quit)" ]; then
  RT=(--rt ../data/raw/rt)
fi
P=(target/release/crowd-pipeline --gtfs ../data/raw/gtfs/delhi-bus --tickets ../data/raw/tickets/October-Delhi.csv "${RT[@]}")
"${P[@]}" --out ../data/out/full
"${P[@]}" --out ../data/out/train --date-to 2026-10-24
"${P[@]}" --out ../data/out/test --date-from 2026-10-25
"${P[@]}" --out ../data/out/sens_spread --set alight_spread=0.5
"${P[@]}" --out ../data/out/sens_cv --set trip_cv=0.5
"${P[@]}" --out ../data/out/sens_timetable --set supply_source=timetable --set fleet_calibration=false
"${P[@]}" --out ../data/out/sens_tickets --set supply_source=tickets
"${P[@]}" --out ../data/out/sens_nocap --set capacity_check_quantile=0.0
"${P[@]}" --out ../data/out/sens_shrink --set shrink_pseudo_trips=20.0
# Folds for the machine-learning comparison: features Oct 1-17 -> target Oct 18-24.
"${P[@]}" --out ../data/out/ml_feat --date-to 2026-10-17
"${P[@]}" --out ../data/out/ml_target --date-from 2026-10-18 --date-to 2026-10-24
cd ..
analysis/.venv/bin/python analysis/run_all.py
