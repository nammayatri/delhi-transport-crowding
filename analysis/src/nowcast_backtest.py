"""Does the live-ticket adjustment forecast the next hours better than the usual pattern?

The service scales each route's usual riding by how busy the city, and the route, have been in the last
hour (technical note 2.9). This replays October hour by hour and checks that rule against what happened,
next to route-only rules, gradient-boosted trees (also exported for the service's opt-in live model), and
(when `nowcast_arima.py` / `nowcast_chronos.py` have been run) ARIMA and the pretrained Chronos models.

Setup, with nothing from the future:
- Every route's ticketed riders per hour, Oct 1-31 (matched tickets, passengers counted).
- "Usual" for a day = average of the earlier days of the same day type (weekday / Saturday /
  Sunday and holidays), so the profile itself is never fitted on the day it forecasts.
- Forecasts are made at every hour from 06:00 to 21:00 on Oct 15-31 (17 days, two weeks of history
  each), for the next three hours. Tickets up to the forecast time are known; later ones are not.
- Settings are tuned on one half (Oct 15-24 or Oct 25-31) and scored on the other, so every score
  is out of sample. The gradient-boosted model is trained the same way.

Scores: Poisson deviance per route-hour (the right error for small counts), mean absolute error,
the city's total per hour, and the bias of the total. Lower deviance is better; "gain" is the drop
against the usual pattern alone.

Writes docs/results/nowcast_backtest.md and .json, docs/figures/nowcast_*.png.
"""

from __future__ import annotations

import itertools
import json
import time
from datetime import date, timedelta
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
import polars as pl
from sklearn.ensemble import HistGradientBoostingRegressor

from common import (
    BLUE,
    GREY,
    MUTED,
    ORANGE,
    AQUA,
    TEXT2,
    YELLOW,
    OUT,
    md_table,
    pct,
    save,
    save_json,
    setup_style,
    titles,
    write_md,
)

TICKETS = OUT / "full" / "assigned_tickets.parquet"
CHRONOS = OUT / "nowcast" / "chronos.npz"
ARIMA = OUT / "nowcast" / "arima.npz"
LIVE_TREES = OUT / "full" / "live_trees.json"

START = date(2026, 10, 1)
ND = 31
EVAL_FROM = 14  # Oct 15
EVAL_DAYS = list(range(EVAL_FROM, ND))
ORIGINS = list(range(6, 22))  # forecasts made at 06:00 ... 21:00
HZ = 3  # hours ahead
DAY_START = 4  # "since the day started" counts from 04:00
FLOOR = 0.1  # smallest forecast, in riders per route-hour (deviance needs a positive forecast)
DAYTYPES = ["weekday", "saturday", "sunday"]
# Half-open ranges of eval-day indices (into EVAL_DAYS): tune on one, score on the other.
FOLDS = {"Oct 15-24": (0, 10), "Oct 25-31": (10, 17)}
UNUSUAL = {
    "2026-10-18": "Sunday before Dussehra",
    "2026-10-19": "Monday, Dussehra eve",
    "2026-10-20": "Dussehra (holiday)",
}

# A route-only rule: the route's last hour, shrunk toward 1 with 6 pseudo-tickets.
CURRENT = {"window": 1, "prior": 6.0, "lo": 0.5, "hi": 2.5, "decay": 1.0, "anchor": "one"}

WINDOWS = [1, 2, 3, "day"]
PRIORS = [0.0, 2.0, 4.0, 6.0, 10.0, 15.0, 25.0, 40.0, 60.0, 100.0, 200.0, 400.0, 1000.0]
CLAMPS = [(0.5, 2.5), (0.67, 1.5), (0.25, 4.0)]
DECAYS = [1.0, 0.85, 0.7]


# --------------------------------------------------------------------------------------
# Data shared with nowcast_chronos.py
# --------------------------------------------------------------------------------------


def load_hourly():
    """Ticketed riders per route, day and hour: X[route, day, hour]."""
    import duckdb

    con = duckdb.connect()
    df = con.sql(
        f"""select route_id, date, daytype, (secs // 3600)::int h, sum(pax)::double pax
            from read_parquet('{TICKETS}') where secs < 86400 group by all"""
    ).pl()
    routes = sorted(df["route_id"].unique().to_list())
    ri = {r: i for i, r in enumerate(routes)}
    dates = [START + timedelta(days=i) for i in range(ND)]
    di = {d.isoformat(): i for i, d in enumerate(dates)}
    X = np.zeros((len(routes), ND, 24))
    X[
        df["route_id"].replace_strict(ri).to_numpy(),
        df["date"].replace_strict(di).to_numpy(),
        df["h"].to_numpy(),
    ] = df["pax"].to_numpy()
    dt_by_date = dict(df.select("date", "daytype").unique().iter_rows())
    dts = np.array([DAYTYPES.index(dt_by_date[d.isoformat()]) for d in dates])
    return routes, dates, dts, X


def profiles(X: np.ndarray, dts: np.ndarray) -> np.ndarray:
    """P[eval day, route, day type, hour]: average of the earlier days of each day type."""
    P = np.zeros((len(EVAL_DAYS), X.shape[0], len(DAYTYPES), 24))
    for i, d in enumerate(EVAL_DAYS):
        for t in range(len(DAYTYPES)):
            past = [e for e in range(d) if dts[e] == t]
            if past:
                P[i, :, t] = X[:, past].mean(axis=1)
    return P


def usual_today(P: np.ndarray, dts: np.ndarray) -> np.ndarray:
    """E[route, eval day, hour]: the usual pattern for each eval day's own day type."""
    return np.stack([P[i, :, dts[d]] for i, d in enumerate(EVAL_DAYS)], axis=1)


def active_routes(P: np.ndarray, dts: np.ndarray) -> np.ndarray:
    """A[route, eval day]: the route had tickets on earlier days of this day type."""
    return np.stack([P[i, :, dts[d]].sum(axis=1) > 0 for i, d in enumerate(EVAL_DAYS)], axis=1)


# --------------------------------------------------------------------------------------
# Forecast cells
# --------------------------------------------------------------------------------------


class Cells:
    """Arrays over [route, eval day, origin, horizon] (targets) and [route, eval day, origin] (windows)."""

    def __init__(self, X: np.ndarray, dts: np.ndarray):
        self.X = X
        self.dts = dts
        self.P = profiles(X, dts)
        self.E = usual_today(self.P, dts)  # [r, i, h]
        self.A = active_routes(self.P, dts)  # [r, i]
        Xe = X[:, EVAL_DAYS]  # [r, i, h]
        o = np.array(ORIGINS)
        hrs = o[:, None] + np.arange(HZ)[None, :]  # [j, k]
        self.y = Xe[:, :, hrs]  # [r, i, j, k]
        self.e = self.E[:, :, hrs]
        self.obs: dict[object, np.ndarray] = {}
        self.exp: dict[object, np.ndarray] = {}
        cx, ce = np.cumsum(Xe, axis=2), np.cumsum(self.E, axis=2)

        def win(c, a, b):  # sum over hours [a, b) along the last axis, vectorised over origins
            hi = c[:, :, b - 1]
            lo = np.where(a > 0, c[:, :, np.maximum(a - 1, 0)], 0.0)
            return hi - lo

        for w in WINDOWS:
            a = np.full_like(o, DAY_START) if w == "day" else o - w
            self.obs[w] = win(cx, a, o)
            self.exp[w] = win(ce, a, o)
        self.mask = np.broadcast_to(self.A[:, :, None, None], self.y.shape)
        self.day_of = np.broadcast_to(np.arange(len(EVAL_DAYS))[None, :, None, None], self.y.shape)

    def fold_mask(self, rng: tuple[int, int]) -> np.ndarray:
        return self.mask & (self.day_of >= rng[0]) & (self.day_of < rng[1])

    def city_ratio(self, w, lo, hi) -> np.ndarray:
        """Whole-city riding over the window against usual: [eval day, origin]."""
        O = (self.obs[w] * self.A[:, :, None]).sum(axis=0)
        Ex = (self.exp[w] * self.A[:, :, None]).sum(axis=0)
        return np.clip(np.where(Ex > 0, O / np.maximum(Ex, 1e-9), 1.0), lo, hi)


def rule(c: Cells, window, prior, lo, hi, decay, anchor) -> np.ndarray:
    """Usual x live factor. The route's ratio is shrunk with `prior` pseudo-tickets toward 1
    (`anchor`="one") or toward the whole city's ratio ("city"); prior=inf uses the anchor alone.
    The factor fades with `decay` per hour ahead (1 = applied in full to all three hours)."""
    O, Ex = c.obs[window], c.exp[window]
    a = c.city_ratio(window, lo, hi)[None, :, :] if anchor == "city" else 1.0
    if np.isinf(prior):
        f = np.broadcast_to(a, O.shape).astype(float)
    else:
        den = Ex + prior
        f = np.where(den > 0, (O + prior * a) / np.maximum(den, 1e-9), a)
    f = np.clip(f, lo, hi)
    fk = np.exp(np.log(f)[..., None] * (decay ** np.arange(HZ))[None, None, None, :])
    return c.e * fk


def pdev(y: np.ndarray, mu: np.ndarray) -> np.ndarray:
    mu = np.maximum(mu, FLOOR)
    return 2.0 * (np.where(y > 0, y * np.log(np.maximum(y, 1e-12) / mu), 0.0) - (y - mu))


def mean_dev(c: Cells, pred: np.ndarray, m: np.ndarray) -> float:
    return float(pdev(c.y[m], pred[m]).mean())


# --------------------------------------------------------------------------------------
# Tuning (on one half, scored on the other)
# --------------------------------------------------------------------------------------


def grid(anchor: str) -> list[dict]:
    priors = [np.inf] if anchor == "city-only" else PRIORS
    out = []
    for w, k, (lo, hi), dcy in itertools.product(WINDOWS, priors, CLAMPS, DECAYS):
        out.append({"window": w, "prior": k, "lo": lo, "hi": hi, "decay": dcy, "anchor": "city" if anchor != "one" else "one"})
    return out


def tune(c: Cells, anchor: str, m: np.ndarray) -> tuple[dict, float]:
    best, best_d = None, np.inf
    for p in grid(anchor):
        d = mean_dev(c, rule(c, **p), m)
        if d < best_d:
            best, best_d = p, d
    return best, best_d


def cross_rule(c: Cells, anchor: str) -> tuple[np.ndarray, dict]:
    """Out-of-sample forecasts: settings tuned on the other half; also the settings tuned on all days."""
    pred = np.zeros_like(c.y)
    chosen = {}
    names = list(FOLDS)
    for a, b in [(0, 1), (1, 0)]:
        p, _ = tune(c, anchor, c.fold_mask(FOLDS[names[a]]))
        chosen[f"tuned on {names[a]}"] = p
        m = c.fold_mask(FOLDS[names[b]])
        pred[m] = rule(c, **p)[m]
    p_all, _ = tune(c, anchor, c.mask)
    chosen["tuned on all days"] = p_all
    return pred, chosen


# --------------------------------------------------------------------------------------
# Gradient-boosted trees on today's tickets so far
# --------------------------------------------------------------------------------------

GBM_FEATURES = [
    "log_usual",  # usual riders in the target hour
    "ahead",  # hours ahead (1-3)
    "hour",  # forecast time
    "daytype",
    "log_route_day",  # route's usual riders per day
    "r1",  # route: log((tickets + 1) / (usual + 1)) over the last hour
    "r2",
    "rday",  # ... since 04:00
    "log_exp1",  # evidence behind r1 / rday
    "log_expday",
    "c1",  # whole city, last hour and since 04:00
    "cday",
]
# Without today's tickets: shows how much of the gain is the trees correcting the usual pattern itself.
GBM_STATIC = ["log_usual", "ahead", "hour", "daytype", "log_route_day"]


def gbm_frame(c: Cells, m: np.ndarray, feats: list[str]) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    R, D, J, K = c.y.shape
    shape = c.y.shape

    def b3(a):  # [r, i, j] -> [r, i, j, k]
        return np.broadcast_to(a[..., None], shape)

    city1 = np.log(c.city_ratio(1, 0.05, 20.0))
    cityd = np.log(c.city_ratio("day", 0.05, 20.0))
    dtype_i = np.array([c.dts[d] for d in EVAL_DAYS])
    route_day = c.E.sum(axis=2)  # [r, i]
    cols = {
        "log_usual": np.log(c.e + 0.1),
        "ahead": np.broadcast_to(np.arange(1, K + 1)[None, None, None, :], shape),
        "hour": np.broadcast_to(np.array(ORIGINS)[None, None, :, None], shape),
        "daytype": np.broadcast_to(dtype_i[None, :, None, None], shape),
        "log_route_day": np.broadcast_to(np.log(route_day + 0.1)[:, :, None, None], shape),
        "r1": b3(np.log((c.obs[1] + 1) / (c.exp[1] + 1))),
        "r2": b3(np.log((c.obs[2] + 1) / (c.exp[2] + 1))),
        "rday": b3(np.log((c.obs["day"] + 1) / (c.exp["day"] + 1))),
        "log_exp1": b3(np.log(c.exp[1] + 0.1)),
        "log_expday": b3(np.log(c.exp["day"] + 0.1)),
        "c1": np.broadcast_to(city1[None, :, :, None], shape),
        "cday": np.broadcast_to(cityd[None, :, :, None], shape),
    }
    Xf = np.column_stack([cols[f][m].astype(np.float32) for f in feats])
    base = np.maximum(c.e[m], FLOOR)
    return Xf, c.y[m], base


def fit_gbm(Xt: np.ndarray, yt: np.ndarray, bt: np.ndarray) -> HistGradientBoostingRegressor:
    """Poisson trees for actual / usual, weighted by usual (an offset model). Day type is a plain
    number (0 weekday, 1 Saturday, 2 Sunday/holiday) so the service can evaluate the trees."""
    model = HistGradientBoostingRegressor(
        loss="poisson",
        learning_rate=0.05,
        max_iter=400,
        max_leaf_nodes=31,
        min_samples_leaf=200,
        l2_regularization=1.0,
        early_stopping=True,
        validation_fraction=0.15,
        random_state=0,
    )
    return model.fit(Xt, yt / bt, sample_weight=bt)


def cross_gbm(c: Cells, feats: list[str]) -> np.ndarray:
    pred = np.zeros_like(c.y)
    names = list(FOLDS)
    for a, b in [(0, 1), (1, 0)]:
        mt, ms = c.fold_mask(FOLDS[names[a]]), c.fold_mask(FOLDS[names[b]])
        model = fit_gbm(*gbm_frame(c, mt, feats))
        Xs, _, bs = gbm_frame(c, ms, feats)
        pred[ms] = bs * model.predict(Xs)
    return pred


def export_trees(c: Cells, path: Path) -> dict:
    """Train the trees on every replayed day and write them for the service's opt-in live model
    (`live.model = "trees"`). The file carries check rows the service re-scores on load."""
    Xt, yt, bt = gbm_frame(c, c.mask, GBM_FEATURES)
    model = fit_gbm(Xt, yt, bt)
    trees = []
    for (t,) in model._predictors:
        n = t.nodes
        leaf = n["is_leaf"].astype(bool)
        trees.append(
            {
                "feature": np.where(leaf, -1, n["feature_idx"]).astype(int).tolist(),
                "threshold": n["num_threshold"].astype(float).tolist(),
                "left": n["left"].astype(int).tolist(),
                "right": n["right"].astype(int).tolist(),
                "missing_left": n["missing_go_to_left"].astype(bool).tolist(),
                "value": n["value"].astype(float).tolist(),
            }
        )
    rng = np.random.default_rng(0)
    idx = rng.choice(len(Xt), size=200, replace=False)
    Xc = Xt[idx].astype(np.float64)
    checks = [{"x": x.tolist(), "factor": float(f)} for x, f in zip(Xc, model.predict(Xc))]
    out = {
        "about": "Gradient-boosted trees (scikit-learn, Poisson loss): ticketed riders in an hour ahead / usual, "
        "from today's tickets so far. Trained on the Oct 15-31 replay (analysis/src/nowcast_backtest.py).",
        "features": GBM_FEATURES,
        "baseline": float(np.asarray(model._baseline_prediction).ravel()[0]),
        "trees": trees,
        "checks": checks,
        "trained_on_cells": int(len(Xt)),
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(out))
    return {"path": str(path.relative_to(OUT.parent.parent)), "trees": len(trees), "cells": int(len(Xt))}


# --------------------------------------------------------------------------------------
# Scoring
# --------------------------------------------------------------------------------------


def scores(c: Cells, pred: np.ndarray, m: np.ndarray, base_dev: float | None) -> dict:
    y, f = c.y[m], np.maximum(pred[m], FLOOR)
    d = float(pdev(y, f).mean())
    # City total per (day, origin, hour ahead), active routes only.
    yc = np.where(m, c.y, 0).sum(axis=0)
    fc = np.where(m, np.maximum(pred, FLOOR), 0).sum(axis=0)
    has = np.where(m, 1, 0).sum(axis=0) > 0
    ape = np.abs(fc[has] - yc[has]) / np.maximum(yc[has], 1.0)
    return {
        "deviance": d,
        "gain": None if base_dev is None else 1 - d / base_dev,
        "mae": float(np.abs(f - y).mean()),
        "city_ape": float(ape.mean()),
        "bias": float(f.sum() / max(y.sum(), 1e-9)),
        "rows": int(m.sum()),
    }


def describe(p: dict) -> str:
    w = "since 04:00" if p["window"] == "day" else f"last {p['window']} h"
    if np.isinf(p["prior"]):
        k = "city ratio only"
    else:
        k = f"{p['prior']:g} pseudo-tickets toward {'1' if p['anchor'] == 'one' else 'the city ratio'}"
    fade = "full for 3 h" if p["decay"] == 1.0 else f"fades x{p['decay']:g} per hour"
    return f"{w}; {k}; clamp {p['lo']:g}-{p['hi']:g}; {fade}"


def main() -> None:
    t0 = time.time()
    routes, dates, dts, X = load_hourly()
    c = Cells(X, dts)
    m_all = c.mask
    print(f"  {len(routes)} routes, {int(m_all.sum()):,} forecast cells ({time.time() - t0:.1f}s)")

    preds: dict[str, np.ndarray] = {}
    settings: dict[str, dict] = {}
    preds["Usual pattern only"] = c.e.copy()
    preds["Route-only rule (last hour, 6 pseudo-tickets)"] = rule(c, **CURRENT)
    for name, anchor in [
        ("Route-only rule, tuned", "one"),
        ("City ratio only, tuned", "city-only"),
        ("City ratio nudged by the route (service default)", "city"),
    ]:
        t1 = time.time()
        preds[name], settings[name] = cross_rule(c, anchor)
        print(f"  {name}: {time.time() - t1:.1f}s")
    t1 = time.time()
    preds["Gradient-boosted trees, no live tickets"] = cross_gbm(c, GBM_STATIC)
    preds["Gradient-boosted trees on today's tickets"] = cross_gbm(c, GBM_FEATURES)
    print(f"  gbm: {time.time() - t1:.1f}s")

    Ec = (c.e * c.A[:, :, None, None]).sum(axis=0)  # usual city total per forecast cell

    def city_times_usual(city: np.ndarray) -> np.ndarray:
        return c.e * np.where(Ec > 0, city / np.maximum(Ec, 1e-9), 1.0)[None]

    def matches(z) -> bool:
        return list(z["routes"]) == routes and list(z["origins"]) == ORIGINS and list(z["eval_days"]) == EVAL_DAYS

    arima_note = None
    if ARIMA.exists():
        z = np.load(ARIMA, allow_pickle=False)
        if matches(z):
            preds["ARIMA on the city total x usual pattern"] = city_times_usual(z["city"])
            per = c.e.copy()
            per[z["top"]] = z["per_route"]
            preds["ARIMA per route, usual pattern as regressor"] = per
            arima_note = str(z["note"])
            ar1 = z["city_ar1"]
        else:
            print("  arima.npz does not match these routes / days; rerun nowcast_arima.py")

    chronos_note = None
    if CHRONOS.exists():
        z = np.load(CHRONOS, allow_pickle=False)
        if matches(z):
            preds["Chronos-Bolt per route (pretrained, no profile)"] = z["bolt"]
            preds["Chronos-2 per route, usual pattern as covariate"] = z["c2cov"]
            preds["Chronos-2 on the city total x usual pattern"] = city_times_usual(z["c2city"])
            chronos_note = str(z["note"])
        else:
            print("  chronos.npz does not match these routes / days; rerun nowcast_chronos.py")

    base = mean_dev(c, preds["Usual pattern only"], m_all)
    rows = []
    res: dict = {"methods": {}, "settings": {}, "cells": int(m_all.sum())}
    for name, p in preds.items():
        s = scores(c, p, m_all, base)
        busy = m_all & (c.e >= 10)
        sb = scores(c, p, busy, mean_dev(c, preds["Usual pattern only"], busy))
        unusual_idx = [EVAL_DAYS.index(dates.index(date.fromisoformat(d))) for d in UNUSUAL]
        mu = m_all & np.isin(c.day_of, unusual_idx)
        su = scores(c, p, mu, mean_dev(c, preds["Usual pattern only"], mu))
        mn = m_all & ~np.isin(c.day_of, unusual_idx)
        sn = scores(c, p, mn, mean_dev(c, preds["Usual pattern only"], mn))
        by_h = []
        for k in range(HZ):
            mk = m_all.copy()
            mk[..., [x for x in range(HZ) if x != k]] = False
            by_h.append(scores(c, p, mk, mean_dev(c, preds["Usual pattern only"], mk))["gain"])
        by_day = []
        for i in range(len(EVAL_DAYS)):
            mi = m_all & (c.day_of == i)
            by_day.append(scores(c, p, mi, mean_dev(c, preds["Usual pattern only"], mi))["gain"])
        res["methods"][name] = {"all": s, "busy": sb, "unusual_days": su, "other_days": sn, "gain_by_hours_ahead": by_h, "gain_by_day": by_day}
        rows.append(
            {
                "Method": name,
                "Deviance": s["deviance"],
                "Gain": pct(s["gain"]),
                "Gain, busy routes": pct(sb["gain"]),
                "Gain, Oct 18-20": pct(su["gain"]),
                "Gain, other days": pct(sn["gain"]),
                "MAE": s["mae"],
                "City total error": pct(s["city_ape"]),
                "Total / actual": s["bias"],
            }
        )
    for name, ch in settings.items():
        res["settings"][name] = {k: {**v, "prior": None if np.isinf(v["prior"]) else v["prior"], "text": describe(v)} for k, v in ch.items()}
    res["eval_days"] = [dates[d].isoformat() for d in EVAL_DAYS]
    res["chronos"] = chronos_note
    res["arima"] = arima_note
    if arima_note:
        res["arima_city_ar1_median"] = float(np.nanmedian(ar1))

    # City ratio on the unusual days, for the write-up.
    cr = c.city_ratio(1, 0.0, 100.0)
    res["city_ratio_last_hour"] = {dates[d].isoformat(): [float(x) for x in cr[i]] for i, d in enumerate(EVAL_DAYS)}

    res["live_trees"] = export_trees(c, LIVE_TREES)
    print(f"  exported the live trees to {LIVE_TREES}")
    figs = figures(c, dates, preds, res)
    write(c, dates, rows, res, figs)
    save_json("nowcast_backtest", res)
    print(f"  done in {time.time() - t0:.1f}s")


# --------------------------------------------------------------------------------------
# Figures and write-up
# --------------------------------------------------------------------------------------

SHORT = {
    "Route-only rule (last hour, 6 pseudo-tickets)": "Route-only rule",
    "Route-only rule, tuned": "Route-only rule, tuned",
    "City ratio only, tuned": "City ratio only",
    "City ratio nudged by the route (service default)": "Service default",
    "Gradient-boosted trees, no live tickets": "Trees, no live tickets",
    "Gradient-boosted trees on today's tickets": "Gradient-boosted trees",
    "Chronos-Bolt per route (pretrained, no profile)": "Chronos-Bolt per route",
    "Chronos-2 per route, usual pattern as covariate": "Chronos-2 per route",
    "Chronos-2 on the city total x usual pattern": "Chronos-2 city total",
    "ARIMA on the city total x usual pattern": "ARIMA city total",
    "ARIMA per route, usual pattern as regressor": "ARIMA per route",
}


def figures(c: Cells, dates, preds, res) -> dict:
    setup_style()
    out = {}
    show = [
        ("Route-only rule (last hour, 6 pseudo-tickets)", GREY),
        ("City ratio nudged by the route (service default)", BLUE),
        ("Gradient-boosted trees on today's tickets", ORANGE),
        ("Chronos-2 per route, usual pattern as covariate", AQUA),
        ("ARIMA on the city total x usual pattern", YELLOW),
    ]
    show = [(n, col) for n, col in show if n in preds]
    fig, ax = plt.subplots(figsize=(10, 4.2))
    xs = np.arange(len(EVAL_DAYS))
    wbar = 0.8 / len(show)
    for n, (name, col) in enumerate(show):
        g = [100 * (v or 0) for v in res["methods"][name]["gain_by_day"]]
        ax.bar(xs + (n - (len(show) - 1) / 2) * wbar, g, width=wbar, color=col, label=SHORT[name])
    ax.axhline(0, color=MUTED, lw=0.8)
    ax.set_xticks(xs)
    ax.set_xticklabels([dates[d].strftime("%d\n%a") for d in EVAL_DAYS], fontsize=8)
    ax.set_ylabel("error drop vs usual pattern (%)")
    titles(
        ax,
        "Live tickets help most when a day is unlike the usual",
        "Drop in Poisson deviance of route-hour forecasts 1-3 h ahead, against the usual pattern alone; out of sample",
    )
    ax.legend(ncol=len(show), loc="upper right")
    out["days"] = save(fig, "nowcast_days")

    # Oct 19: city total per hour, actual vs forecasts made one hour earlier.
    i = EVAL_DAYS.index(dates.index(date(2026, 10, 19)))
    hrs = np.array(ORIGINS)
    A = c.A[:, i]
    fig, ax = plt.subplots(figsize=(10, 4.2))
    ax.plot(hrs, c.y[A, i, :, 0].sum(axis=0), color=TEXT2, lw=2.4, label="Actual tickets")
    ax.plot(hrs, c.e[A, i, :, 0].sum(axis=0), color=GREY, ls="--", label="Usual for a weekday")
    for name, col in show:
        ax.plot(hrs, np.maximum(preds[name][A, i, :, 0], FLOOR).sum(axis=0), color=col, label=SHORT[name] + ", 1 h earlier")
    ax.set_xticks(hrs)
    ax.set_xticklabels([f"{h:02d}:00" for h in hrs], fontsize=8)
    ax.set_ylabel("ticketed riders per hour, all routes")
    lo = c.y[A, i, :, 0].sum(axis=0) / np.maximum(c.e[A, i, :, 0].sum(axis=0), 1)
    titles(
        ax,
        f"Dussehra eve: riding ran {100 * (1 - lo.max()):.0f}-{100 * (1 - lo.min()):.0f}% below a usual weekday",
        "Mon 19 Oct, tickets in the hour starting at each time, against forecasts made at its start",
    )
    ax.legend(ncol=2, loc="upper right")
    out["oct19"] = save(fig, "nowcast_oct19")
    return out


def write(c: Cells, dates, rows, res, figs) -> None:
    table = md_table(
        pl.DataFrame(rows),
        fmt={"Deviance": "{:.3f}", "MAE": "{:.2f}", "Total / actual": "{:.3f}"},
    )
    hz = pl.DataFrame(
        [
            {"Method": n, **{f"{k + 1} h ahead": pct(g) for k, g in enumerate(v["gain_by_hours_ahead"])}}
            for n, v in res["methods"].items()
            if n != "Usual pattern only"
        ]
    )
    sett = []
    for name, ch in res["settings"].items():
        for fold, p in ch.items():
            sett.append({"Method": name, "Tuned on": fold, "Settings": p["text"]})
    cr = res["city_ratio_last_hour"]
    ur = [
        {"Day": f"{d} ({UNUSUAL[d]})", "City tickets vs usual, 08:00-09:00": f"{cr[d][ORIGINS.index(9)]:.2f}", "17:00-18:00": f"{cr[d][ORIGINS.index(18)]:.2f}"}
        for d in UNUSUAL
    ]
    chronos = (
        f"Chronos rows come from `nowcast_chronos.py` ({res['chronos']})."
        if res["chronos"]
        else "Chronos was not run (install `analysis/requirements-forecast.txt`, run `analysis/src/nowcast_chronos.py`)."
    )
    chronos += (
        f" ARIMA rows come from `nowcast_arima.py` ({res['arima']})."
        if res["arima"]
        else " ARIMA was not run (`analysis/src/nowcast_arima.py`)."
    )
    text = f"""
# Live adjustment: does it forecast the next hours better?

The service scales each route's usual riding by how busy the city, and the route, have been in the last
hour (technical note 2.9). This replays October hour by hour and scores that rule and the alternatives
against what happened.

**Setup.** Every route's ticketed riders per hour. "Usual" for a day is the average of the earlier days of
its day type, so it is never fitted on the day it forecasts. At every hour from 06:00 to 21:00 on Oct 15-31
(17 days), each method forecasts each route's riders for the next three hours from the tickets so far:
{res['cells']:,} route-hour forecasts. Settings are tuned on Oct 15-24 and scored on Oct 25-31, and the
other way round, so every score is out of sample. {chronos}

**Scores.** Poisson deviance per route-hour (lower is better; the right error for small counts). Gain is
its drop against the usual pattern alone. MAE is in riders per route-hour. "City total error" is the
average error of the city's total for each forecast hour; "Total / actual" is the bias of the total.
Busy routes are route-hours where at least 10 ticketed riders are usual. Oct 18-20 are the days around
Dussehra.

{table}

![Gain by day]({figs['days']})

![Dussehra eve]({figs['oct19']})

## Gain by hours ahead
{md_table(hz)}

## Settings chosen
{md_table(pl.DataFrame(sett))}

## The days around Dussehra
{md_table(pl.DataFrame(ur))}

## Reading the results
See the technical note, section 5, for the interpretation and how the statistical rule compares with the models.
"""
    write_md("nowcast_backtest", text)


if __name__ == "__main__":
    main()
