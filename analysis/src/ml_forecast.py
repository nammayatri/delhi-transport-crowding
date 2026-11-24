"""Can a simple machine-learning model forecast crowding better than the crowding model?

Gradient-boosted trees (scikit-learn HistGradientBoostingRegressor, Poisson loss) trained only on the
October tickets, scored on the same week-to-week check as stability.md.

Unit: weekday cell = route pattern x stop-to-stop segment x 30-minute bin.
Target: ticketed riders per bus pass in a later window (kernel-smoothed counts, no shrinkage). Bus passes
are the exposure: Poisson deviance weighted by them, so a cell with many buses counts more.

Temporal backtest, no leakage:
- fit:      features from Oct 1-17 (run `ml_feat`)  -> target Oct 18-24 (run `ml_target`)
- evaluate: features from Oct 1-24 (run `train`)    -> target Oct 25-31 (run `test`)
The crowding model is scored on the same cells: its `train` estimate against the same target.

Writes docs/results/ml_forecast.md and docs/results/ml_forecast.json.
"""

from __future__ import annotations

import time

import numpy as np
import polars as pl
from sklearn.ensemble import HistGradientBoostingRegressor
from sklearn.inspection import permutation_importance
from sklearn.metrics import mean_poisson_deviance

from common import (
    CP_LAT,
    CP_LON,
    LEVELS,
    cohen_kappa,
    connect,
    level_probs,
    load_json,
    md_table,
    pct,
    q,
    save_json,
    spearman,
    write_md,
)

MIN_TEST_TICKETS = 10.0
CRUSH = {"12m-ac": 85.0, "12m-nonac": 90.0, "9m-ev": 55.0}
CLASS_CODE = {"12m-ac": 0, "12m-nonac": 1, "9m-ev": 2}

# Features computed from the feature window only (tickets) or known in advance (timetable, map).
WINDOW_FEATURES = [
    "rate",  # this cell: ticketed riders per bus pass
    "log_tix",  # evidence behind it
    "rate_1h",  # same stretch, +/- 1 hour
    "rate_2h",  # same stretch, +/- 2 hours
    "seg_rate",  # same stretch, all day
    "log_seg_tix",
    "route_bin_rate",  # whole route, same half hour
    "route_rate",  # whole route, all day
    "log_route_tix",
    "city_bin_rate",  # whole city, same half hour
]
STATIC_FEATURES = [
    "bin_minute",
    "seq",
    "frac",  # position along the route
    "km_from_start",
    "route_km",
    "log_buses",  # bus passes per day (timetable x live feed)
    "bus_class",
    "inbound",  # heading toward Connaught Place
    "km_to_cp",
    "bearing_sin",
    "bearing_cos",
]
MODEL_FEATURES = ["model_rate"]  # the crowding model's own estimate (stacking)


def window_table(con, run: str, nd: int) -> None:
    con.sql(
        f"""
        create or replace table f_{run} as
        with b as (select pattern, seq, bin_minute, buses_per_day, tickets, ticketed_rate
                   from seg_{run} where daytype = 'weekday'),
        k as (select t.*, 0.25 * coalesce(p.buses_per_day, 0) + 0.5 * t.buses_per_day
                          + 0.25 * coalesce(n.buses_per_day, 0) ks
              from b t
              left join b p on p.pattern = t.pattern and p.seq = t.seq and p.bin_minute = t.bin_minute - 30
              left join b n on n.pattern = t.pattern and n.seq = t.seq and n.bin_minute = t.bin_minute + 30),
        e as (select *, {nd} * ks expo from k)
        select pattern, seq, bin_minute, tickets tix, expo,
               tickets / nullif(expo, 0) rate,
               ticketed_rate model_rate,
               sum(tickets) over w1 / nullif(sum(expo) over w1, 0) rate_1h,
               sum(tickets) over w2 / nullif(sum(expo) over w2, 0) rate_2h,
               sum(tickets) over (partition by pattern, seq)
                 / nullif(sum(expo) over (partition by pattern, seq), 0) seg_rate,
               sum(tickets) over (partition by pattern, seq) seg_tix,
               sum(tickets) over (partition by pattern, bin_minute)
                 / nullif(sum(expo) over (partition by pattern, bin_minute), 0) route_bin_rate,
               sum(tickets) over (partition by pattern) / nullif(sum(expo) over (partition by pattern), 0) route_rate,
               sum(tickets) over (partition by pattern) route_tix,
               sum(tickets) over (partition by bin_minute)
                 / nullif(sum(expo) over (partition by bin_minute), 0) city_bin_rate
        from e
        window w1 as (partition by pattern, seq order by bin_minute range between 60 preceding and 60 following),
               w2 as (partition by pattern, seq order by bin_minute range between 120 preceding and 120 following)
        """
    )


def target_table(con, run: str, nd: int) -> None:
    con.sql(
        f"""
        create or replace table t_{run} as
        with b as (select * from seg_{run} where daytype = 'weekday'),
        k as (select t.*, 0.25 * coalesce(p.buses_per_day, 0) + 0.5 * t.buses_per_day
                          + 0.25 * coalesce(n.buses_per_day, 0) ks
              from b t
              left join b p on p.pattern = t.pattern and p.seq = t.seq and p.bin_minute = t.bin_minute - 30
              left join b n on n.pattern = t.pattern and n.seq = t.seq and n.bin_minute = t.bin_minute + 30)
        select k.pattern, k.seq, k.bin_minute, k.route_id, k.bus_class, k.seats, k.comfortable,
               k.km_from_start, k.buses_per_day,
               k.tickets t_tix, {nd} * k.ks t_expo, k.tickets / nullif({nd} * k.ks, 0) t_rate,
               k.load t_load, k.level t_level,
               max(k.seq) over (partition by k.pattern) n_seq,
               max(k.km_from_start) over (partition by k.pattern) route_km,
               fs.lat f_lat, fs.lon f_lon, ts.lat t_lat, ts.lon t_lon
        from k
        join gtfs_stops fs on fs.stop_id = k.from_stop_id
        join gtfs_stops ts on ts.stop_id = k.to_stop_id
        where k.ks > 0
        """
    )


def frame(con, feat_run: str, target_run: str, adj_run: str, pred_run: str | None) -> pl.DataFrame:
    extra = ""
    if pred_run:
        extra = f", m.load model_load, m.tickets model_tix"
    df = q(
        con,
        f"""
        select t.*, f.tix, f.rate, f.model_rate, f.rate_1h, f.rate_2h, f.seg_rate, f.seg_tix,
               f.route_bin_rate, f.route_rate, f.route_tix, f.city_bin_rate,
               coalesce(r.expansion_adjustment, 1.0) adj_target,
               coalesce(ra.expansion_adjustment, 1.0) adj_pred
               {extra}
        from t_{target_run} t
        left join f_{feat_run} f using (pattern, seq, bin_minute)
        left join routes_{target_run} r on r.route_id = t.route_id
        left join routes_{adj_run} ra on ra.route_id = t.route_id
        {f"left join (select pattern, seq, bin_minute, load, tickets from seg_{pred_run} where daytype = 'weekday') m using (pattern, seq, bin_minute)" if pred_run else ""}
        """,
    )
    d_from = np.hypot((df["f_lat"] - CP_LAT).to_numpy(), ((df["f_lon"] - CP_LON) * np.cos(np.radians(CP_LAT))).to_numpy())
    d_to = np.hypot((df["t_lat"] - CP_LAT).to_numpy(), ((df["t_lon"] - CP_LON) * np.cos(np.radians(CP_LAT))).to_numpy())
    ang = np.arctan2((df["f_lat"] - CP_LAT).to_numpy(), ((df["f_lon"] - CP_LON) * np.cos(np.radians(CP_LAT))).to_numpy())
    return df.with_columns(
        pl.Series("inbound", (d_to < d_from).astype(float)),
        pl.Series("km_to_cp", d_from * 111.32),
        pl.Series("bearing_sin", np.sin(ang)),
        pl.Series("bearing_cos", np.cos(ang)),
        (pl.col("seq") / pl.col("n_seq").clip(lower_bound=1)).alias("frac"),
        pl.col("buses_per_day").log1p().alias("log_buses"),
        pl.col("tix").fill_null(0).log1p().alias("log_tix"),
        pl.col("seg_tix").fill_null(0).log1p().alias("log_seg_tix"),
        pl.col("route_tix").fill_null(0).log1p().alias("log_route_tix"),
        pl.col("bus_class").replace_strict(CLASS_CODE, default=0).cast(pl.Float64).alias("bus_class"),
    )


def matrix(df: pl.DataFrame, cols: list[str]) -> np.ndarray:
    return df.select([pl.col(c).cast(pl.Float64) for c in cols]).to_numpy()


def base_rate(df: pl.DataFrame) -> np.ndarray:
    """The crowding model's estimate, falling back to the route and city rates where it has none."""
    b = df.select(
        pl.coalesce(["model_rate", "route_bin_rate", "city_bin_rate"]).fill_null(0.0)
    ).to_series().to_numpy()
    return np.clip(b, 1e-4, None)


def fit(df: pl.DataFrame, cols: list[str], offset: np.ndarray | None = None) -> HistGradientBoostingRegressor:
    m = HistGradientBoostingRegressor(
        loss="poisson",
        learning_rate=0.05,
        max_iter=600,
        max_leaf_nodes=31,
        min_samples_leaf=200,
        l2_regularization=1.0,
        early_stopping=True,
        validation_fraction=0.1,
        n_iter_no_change=30,
        random_state=0,
        categorical_features=[cols.index("bus_class")] if "bus_class" in cols else None,
    )
    y, w = df["t_rate"].to_numpy(), df["t_expo"].to_numpy()
    if offset is not None:
        # Poisson with an offset: fit target / offset with weight x offset, so the trees learn a
        # multiplicative correction to the offset (deviance is linear in the scale).
        y, w = y / offset, w * offset
    m.fit(matrix(df, cols), y, sample_weight=w)
    return m


def to_load(rate: np.ndarray, e: float, adj: np.ndarray, crush: np.ndarray) -> np.ndarray:
    return np.minimum(np.nan_to_num(rate, nan=0.0) * e * adj, crush)


def levels(load, tickets, seats, comfortable, trip_cv) -> np.ndarray:
    _, _, _, lv = level_probs(load, tickets, seats, comfortable, trip_cv)
    return np.asarray(lv)


def score(name: str, ev: pl.DataFrame, pred_rate: np.ndarray, pred_load: np.ndarray, pred_tix, target, trip_cv) -> dict:
    """Metrics on all evaluation cells (deviance) and on cells with >= 10 test tickets."""
    y, w = ev["t_rate"].to_numpy(), ev["t_expo"].to_numpy()
    pr = np.clip(np.nan_to_num(pred_rate, nan=0.0), 1e-6, None)
    dev_all = mean_poisson_deviance(y, pr, sample_weight=w)
    h = (ev["t_tix"] >= MIN_TEST_TICKETS).to_numpy()
    # Level-matched: scale the forecast so its total matches the target week, to separate how well
    # a model spreads riders over cells from how well it guessed the week's overall level.
    k = (y * w).sum() / max((pr * w).sum(), 1e-12)
    seats, comf = ev["seats"].to_numpy()[h], ev["comfortable"].to_numpy()[h]
    lv_pred = levels(pred_load[h], np.nan_to_num(pred_tix[h], nan=1.0), seats, comf, trip_cv)
    cm = np.zeros((3, 3))
    np.add.at(cm, (lv_pred, target["raw_level"][h]), 1)
    return {
        "model": name,
        "Spearman vs raw test": spearman(pred_load[h], target["raw"][h]),
        "Spearman vs test model": spearman(pred_load[h], target["model"][h]),
        "mean abs error (riders/bus)": float(np.mean(np.abs(pred_load[h] - target["raw"][h]))),
        "level agreement": float(np.trace(cm) / cm.sum()),
        "kappa": cohen_kappa(cm),
        "Poisson deviance, all cells": float(dev_all),
        "Poisson deviance, >= 10 tickets": float(mean_poisson_deviance(y[h], pr[h], sample_weight=w[h])),
        "level-matched deviance, all cells": float(mean_poisson_deviance(y, pr * k, sample_weight=w)),
        "level-matched deviance, >= 10 tickets": float(mean_poisson_deviance(y[h], pr[h] * k, sample_weight=w[h])),
        "forecast / actual week total": float(1 / k),
    }


def main() -> dict:
    t0 = time.time()
    runs = ["ml_feat", "ml_target", "train", "test"]
    con = connect(runs)
    nd = {r: load_json(r, "meta.json")["meta"]["n_days"][0] for r in runs}
    exp = {r: load_json(r, "meta.json")["meta"]["expansion"]["mid"] for r in runs}
    trip_cv = load_json("train", "meta.json")["config"]["trip_cv"]
    for r in ["ml_feat", "train"]:
        window_table(con, r, nd[r])
    for r in ["ml_target", "test"]:
        target_table(con, r, nd[r])

    fit_df = frame(con, "ml_feat", "ml_target", "ml_feat", None).filter(pl.col("t_expo") > 0)
    ev = frame(con, "train", "test", "train", "train").filter(pl.col("t_expo") > 0)
    # Ticketed riding against the crowding model's estimate, in the fitting and evaluation target weeks.
    def ratio(df):
        b = base_rate(df)
        return float((df["t_rate"].to_numpy() * df["t_expo"].to_numpy()).sum() / (b * df["t_expo"].to_numpy()).sum())
    shift = {"fit": ratio(fit_df), "eval": ratio(ev)}
    crush = ev["bus_class"].cast(pl.Int64).replace_strict({v: CRUSH[k] for k, v in CLASS_CODE.items()}).to_numpy()
    adj_p, adj_t = ev["adj_pred"].to_numpy(), ev["adj_target"].to_numpy()
    raw = np.nan_to_num(ev["t_rate"].to_numpy()) * exp["test"] * adj_t
    target = {
        "raw": raw,
        "model": ev["t_load"].fill_null(0).to_numpy(),
        "raw_level": levels(raw, ev["t_tix"].to_numpy(), ev["seats"].to_numpy(), ev["comfortable"].to_numpy(), trip_cv),
    }
    tix_pred = ev["model_tix"].to_numpy()

    # Tickets only: trees predict the rate directly. On top of the crowding model: trees learn a
    # correction factor to the model's own estimate (Poisson offset).
    variants = {
        "ML, tickets only": (WINDOW_FEATURES + STATIC_FEATURES, False),
        "ML on top of the crowding model": (WINDOW_FEATURES + STATIC_FEATURES + MODEL_FEATURES, True),
    }
    rows, fitted = [], {}
    # The crowding model and two simple baselines, from the same feature window.
    model_rate = ev["model_rate"].to_numpy()
    rows.append(score("crowding model", ev, model_rate, ev["model_load"].fill_null(0).to_numpy(), tix_pred, target, trip_cv))
    for col, name in [("seg_rate", "stretch all-day average"), ("route_rate", "route all-day average")]:
        r_ = ev[col].to_numpy()
        rows.append(score(name, ev, r_, to_load(r_, exp["train"], adj_p, crush), np.full(ev.height, 1e9), target, trip_cv))
    for name, (cols, use_offset) in variants.items():
        t1 = time.time()
        m = fit(fit_df, cols, base_rate(fit_df) if use_offset else None)
        fitted[name] = (m, cols)
        pr = m.predict(matrix(ev, cols)) * (base_rate(ev) if use_offset else 1.0)
        row = score(name, ev, pr, to_load(pr, exp["train"], adj_p, crush), tix_pred, target, trip_cv)
        row["trees"] = int(m.n_iter_)
        row["fit seconds"] = round(time.time() - t1, 1)
        rows.append(row)

    # What the tickets-only model leans on: permutation importance on a sample of evaluation cells.
    m, cols = fitted["ML, tickets only"]
    rng = np.random.default_rng(0)
    idx = rng.choice(ev.height, size=min(100_000, ev.height), replace=False)
    pi = permutation_importance(
        m,
        matrix(ev, cols)[idx],
        ev["t_rate"].to_numpy()[idx],
        sample_weight=ev["t_expo"].to_numpy()[idx],
        scoring="neg_mean_poisson_deviance",
        n_repeats=3,
        random_state=0,
    )
    imp = sorted(zip(cols, pi.importances_mean), key=lambda x: -x[1])

    base = rows[0]
    res = {
        "fit_cells": fit_df.height,
        "eval_cells": ev.height,
        "headline_cells": int((ev["t_tix"] >= MIN_TEST_TICKETS).sum()),
        "rows": rows,
        "importance": [{"feature": f, "deviance increase when shuffled": float(v)} for f, v in imp],
        "seconds": round(time.time() - t0, 1),
        "target_over_model": shift,
    }
    save_json("ml_forecast", res)

    tab = pl.DataFrame([{k: v for k, v in r.items() if k not in ("trees", "fit seconds")} for r in rows])
    dev_gain = {r["model"]: 1 - r["Poisson deviance, all cells"] / base["Poisson deviance, all cells"] for r in rows}
    best = max(rows, key=lambda r: r["Spearman vs raw test"])
    tickets_only = next(r for r in rows if r["model"] == "ML, tickets only")
    stacked = next(r for r in rows if r["model"] == "ML on top of the crowding model")
    md = f"""
# A simple machine-learning forecast, against the crowding model

**Question.** Trained only on the October tickets, can a gradient-boosted model forecast the last week better
than the crowding model (pooled running count with shrinkage)?

**Setup.** Weekday cells (route pattern x stretch x 30-minute bin). The target is ticketed riders per bus pass,
kernel-smoothed but not shrunk, with bus passes as the exposure. scikit-learn `HistGradientBoostingRegressor`
with Poisson loss, trained on features from Oct 1-17 to predict Oct 18-24 ({fit_df.height:,} cells), then given
Oct 1-24 to predict Oct 25-31 ({ev.height:,} cells). Nothing from the evaluation week is used for fitting. The crowding
model is scored on the same cells, from its Oct 1-24 run. Loads use the same expansion, route capacity
adjustment and crush cap for every model; levels use the model's own rule.

Features, all from the feature window or known in advance: the cell's ticketed riders per bus and the tickets
behind it; the same stretch over +/- 1 and 2 hours and all day; the route at the same half hour and all day; the
city at the same half hour; half hour, position and km along the route, route length, bus passes, bus class,
direction (toward Connaught Place or away), distance and bearing from Connaught Place. The second ML model also
gets the crowding model's own estimate and learns a correction factor to it (a Poisson offset), so it starts from
the crowding model rather than from scratch.

{md_table(tab, {
    "Spearman vs raw test": "{:.3f}", "Spearman vs test model": "{:.3f}", "mean abs error (riders/bus)": "{:.2f}",
    "level agreement": pct, "kappa": "{:.3f}", "Poisson deviance, all cells": "{:.4f}", "Poisson deviance, >= 10 tickets": "{:.4f}",
    "level-matched deviance, all cells": "{:.4f}", "level-matched deviance, >= 10 tickets": "{:.4f}", "forecast / actual week total": "{:.3f}",
})}

Spearman, error and level agreement are on the {res['headline_cells']:,} cells with at least {MIN_TEST_TICKETS:g} test tickets
(as in `stability.md`); "raw test" is the observed last-week value, "test model" the crowding model fitted on the
last week alone. Poisson deviance (lower is better) covers every cell, weighted by bus passes, so it also
measures the sparse cells.

## Reading

- Poisson deviance on all cells, relative to the crowding model: {', '.join(f"{k} {pct(v)}" for k, v in dev_gain.items() if k != base['model'])}
  (positive = better forecast).
- Ranking against the raw last week: the tickets-only ML model scores Spearman {tickets_only['Spearman vs raw test']:.3f} and
  {pct(tickets_only['level agreement'])} level agreement; the crowding model {base['Spearman vs raw test']:.3f} and
  {pct(base['level agreement'])}; ML on top of the crowding model {stacked['Spearman vs raw test']:.3f} and
  {pct(stacked['level agreement'])}. Best on this measure: {best['model']}.
- **The weeks differ in level, and one month cannot teach that.** Ticketed riding in the fitting target week
  (Oct 18-24, Dussehra week) was {shift['fit']:.2f}x the crowding model's estimate; in the evaluation week (Oct 25-31)
  it was {shift['eval']:.2f}x. The ML models learn the first week's level and carry it into the second, so they
  under-forecast the last week (forecast / actual: tickets-only
  {tickets_only['forecast / actual week total']:.3f}, crowding model {base['forecast / actual week total']:.3f}).
  The level-matched columns rescale each forecast to the week's actual total: they show how well a model spreads
  riders over stretches and hours, apart from the overall level. In use, the live-ticket factor (technical note
  §2.9) supplies that level.
- What the tickets-only model leans on (increase in deviance when the feature is shuffled):
  {'; '.join(f"{f} {v:.4f}" for f, v in imp[:6])}.
- Both approaches learn from the same tickets, so neither can fix what the tickets cannot show: the city-wide
  expansion from ticketed riders to all riders, app use that differs by route, and buses that carry no
  tagged ticket. The ML model forecasts ticketed riders per bus; the step to crowding (expansion, bus class,
  capacity) is the same for both.
"""
    write_md("ml_forecast", md)
    return res


if __name__ == "__main__":
    main()
