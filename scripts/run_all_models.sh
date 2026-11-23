#!/usr/bin/env bash
# Rebuild every model run used by the validation suite, then rerun the suite.
set -euo pipefail
cd "$(dirname "$0")/../crowding"
P="target/release/crowd-pipeline --gtfs ../data/raw/gtfs/delhi-bus --tickets ../data/raw/tickets/October-Delhi.csv"
$P --out ../data/out/full
$P --out ../data/out/train --date-to 2026-10-24
$P --out ../data/out/test --date-from 2026-10-25
$P --out ../data/out/sens_spread --set alight_spread=0.5
$P --out ../data/out/sens_cv --set trip_cv=0.5
$P --out ../data/out/sens_noobs --set supply_source=timetable
$P --out ../data/out/sens_nocap --set capacity_check_quantile=0.0
$P --out ../data/out/sens_shrink --set shrink_pseudo_trips=20.0
cd ..
analysis/.venv/bin/python analysis/run_all.py
