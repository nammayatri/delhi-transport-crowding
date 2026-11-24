# Crowding model validation

Python checks that the bus crowding estimates produced by the Rust pipeline (`crowding/`) are grounded
in the data: what the ticket file contains, whether the estimates show the patterns a city's buses should
show, how they hold up from one period to the next, how much they depend on assumptions, and where they
are least reliable.

## Rerun

From the project root, after the pipeline has written its runs to `data/out/<run>/`
(`scripts/run_all_models.sh` writes all of them: `full`, `train`, `test`, the `sens_*` runs and the `ml_*` folds):

```sh
analysis/.venv/bin/python analysis/run_all.py
```

The whole run takes about five minutes. Each step can also be run on its own, e.g.
`analysis/.venv/bin/python analysis/src/stability.py`, but `summary.py` reads the JSON written by the
other steps, so run it last.

To recreate the environment:

```sh
python3 -m venv analysis/.venv
analysis/.venv/bin/pip install -r analysis/requirements.txt
# optional, for ARIMA and Chronos in the live-adjustment test (PyTorch; Chronos models download from
# Hugging Face on first run):
analysis/.venv/bin/pip install -r analysis/requirements-forecast.txt
```

## Steps and outputs

| script | question | writes |
| --- | --- | --- |
| `src/audit.py` | What is in the ticket file, how much of it matched the timetable, and do matched trip lengths agree with the fares? | `docs/results/audit.md`, `docs/figures/audit_*.png` |
| `src/patterns.py` | Do the estimates show known patterns: AM/PM peaks, weekday vs weekend, toward the centre in the morning and away in the evening, plausible busiest corridors and stops? | `docs/results/patterns.md`, `docs/figures/patterns_*.png` |
| `src/stability.py` | Does a model fitted on Oct 1-24 predict Oct 25-31, and does time of day beat a route-average baseline? | `docs/results/stability.md`, `docs/figures/stability_*.png` |
| `src/sensitivity.py` | How much do the crowding levels move under other assumptions (ridership, bus supply, capacity cap, alighting spread, trip-to-trip spread, shrinkage)? | `docs/results/sensitivity.md` |
| `src/coverage.py` | Where and when are estimates least reliable, and what does the confidence label miss? | `docs/results/coverage.md`, `docs/figures/coverage_map.png` |
| `src/ml_forecast.py` | Would gradient-boosted trees trained on the tickets forecast the last week better than the crowding model? | `docs/results/ml_forecast.md` |
| `src/nowcast_backtest.py` | Replaying October hour by hour: does the live adjustment forecast the next three hours better than the usual pattern, and which method is best? Adds the ARIMA and Chronos rows when `nowcast_arima.py` and `nowcast_chronos.py` have been run, and exports the trees for the service's opt-in live model. | `docs/results/nowcast_backtest.md`, `docs/figures/nowcast_*.png`, `data/out/full/live_trees.json` |
| `src/nowcast_arima.py` | Optional, not part of `run_all.py`: regression with ARIMA errors on the city total and on every route, refitted at every forecast time (about 10 minutes). Needs `requirements-forecast.txt`. | `data/out/nowcast/arima.npz` |
| `src/nowcast_chronos.py` | Optional, not part of `run_all.py`: forecasts from the pretrained Chronos models for the same cells (about 35 minutes). Needs `requirements-forecast.txt`. | `data/out/nowcast/chronos.npz` |
| `src/summary.py` | One-page digest of the headline numbers. | `docs/results/summary.md` |

Every step also writes its numbers to `docs/results/<step>.json`; `summary.md` is generated only from
those files, so the digest always matches the detailed results.

`src/common.py` holds the paths, DuckDB helpers (views over each run's Parquet files), the chart style,
Markdown table helpers and a NumPy port of the model's level rule (gamma-distributed load per bus,
median-bus level). The port reproduces the stored levels exactly and is used to re-derive levels for the
low and high ridership scenarios.

## Conventions

- "Load" is the model's expected number of riders on board per bus on a stop-to-stop segment in a
  30-minute bin (mid ridership scenario unless stated).
- Averages over cells are weighted by buses per day ("bus-weighted"); "bus-km" weights additionally by
  the straight-line segment length the model itself uses.
- Weekday = Monday to Friday except the Oct 2 and Oct 20 holidays, which the model treats as Sundays.
- Peak windows: 08:00-10:00 and 17:30-20:00 (bin start times).
- Inbound / outbound: a segment is inbound when its end stop is closer to Connaught Place
  (28.6315, 77.2167) than its start stop.
