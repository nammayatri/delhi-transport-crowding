"""ARIMA forecasts for the live-adjustment backtest.

Regression with ARIMA errors (statsmodels SARIMAX), on the same forecast cells as nowcast_backtest.py:
every hour from 06:00 to 21:00 on Oct 15-31, the next three hours, from two weeks of hourly history up to
the forecast time, refitted at every forecast time.

    log(1 + riders) = c + b * log(1 + usual) + e,   e ~ ARMA

"Usual" is the backtest's usual pattern (average of earlier days of each day type), known for the past
and the next three hours, so the ARMA part models how far today runs from usual and how long that lasts.

- City total: all routes added up; the order is chosen by AIC from AR(1), AR(2) and ARMA(1,1). The
  backtest turns the forecast into a city ratio and applies it to each route's usual pattern.
- Per route: AR(1) errors, every route. Most routes see 2-3 tickets an hour, often none, which ARIMA
  (continuous values, constant variance) does not really suit; a route whose model cannot be fitted, or
  with no tickets in the two weeks, gets its usual pattern.

Writes data/out/nowcast/arima.npz; nowcast_backtest.py reads it. About 10 minutes on a laptop (8 processes).
"""

from __future__ import annotations

import time
import warnings
from multiprocessing import Pool

import numpy as np
from statsmodels.tsa.statespace.sarimax import SARIMAX

from nowcast_backtest import ARIMA, EVAL_DAYS, EVAL_FROM, HZ, ND, ORIGINS, Cells, load_hourly

CTX = 14 * 24
CITY_ORDERS = [(1, 0, 0), (2, 0, 0), (1, 0, 1)]


def fit_forecast(y: np.ndarray, u_past: np.ndarray, u_next: np.ndarray, orders) -> tuple[np.ndarray, float, tuple]:
    """Best-AIC model of log1p(y) on log1p(usual); returns the 3-hour forecast, AR(1) coefficient, order."""
    best = None
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        for order in orders:
            try:
                r = SARIMAX(np.log1p(y), exog=np.log1p(u_past), order=order, trend="c").fit(disp=False, maxiter=200)
            except Exception:  # noqa: BLE001 - a failed fit falls back to the next order
                continue
            if best is None or r.aic < best[0].aic:
                best = (r, order)
    if best is None:
        return u_next.copy(), np.nan, (0, 0, 0)
    r, order = best
    f = np.expm1(r.forecast(steps=HZ, exog=np.log1p(u_next)[:, None]))
    names = list(r.model.param_names)
    a1 = float(np.asarray(r.params)[names.index("ar.L1")]) if "ar.L1" in names else np.nan
    return np.clip(f, 0, None), a1, order


def usual_series(c: Cells, dts: np.ndarray, i: int) -> np.ndarray:
    """Usual pattern as of eval day i, laid over every day by its own day type: [route, ND * 24]."""
    R = c.X.shape[0]
    return np.stack([c.P[i][:, dts[e], :] for e in range(ND)], axis=1).reshape(R, ND * 24)


def route_job(args) -> np.ndarray:
    Xr, Us = args  # Xr: [ND*24], Us: [eval day, ND*24]
    out = np.zeros((len(EVAL_DAYS), len(ORIGINS), HZ))
    for i, d in enumerate(EVAL_DAYS):
        for j, t in enumerate(ORIGINS):
            end = d * 24 + t
            y, up, un = Xr[end - CTX : end], Us[i, end - CTX : end], Us[i, end : end + HZ]
            if y.sum() > 0 and up.std() > 0:
                out[i, j], _, _ = fit_forecast(y, up, un, [(1, 0, 0)])
            else:
                out[i, j] = un
    return out


def main() -> None:
    t0 = time.time()
    routes, dates, dts, X = load_hourly()
    c = Cells(X, dts)
    R = len(routes)
    Xf = X.reshape(R, ND * 24)
    U = np.stack([usual_series(c, dts, i) for i in range(len(EVAL_DAYS))])  # [i, r, ND*24]

    # City total.
    city = np.zeros((len(EVAL_DAYS), len(ORIGINS), HZ))
    ar1 = np.full((len(EVAL_DAYS), len(ORIGINS)), np.nan)
    orders = []
    for i, d in enumerate(EVAL_DAYS):
        A = c.A[:, i]
        y_all, u_all = Xf[A].sum(axis=0), U[i][A].sum(axis=0)
        for j, t in enumerate(ORIGINS):
            end = d * 24 + t
            city[i, j], ar1[i, j], o = fit_forecast(y_all[end - CTX : end], u_all[end - CTX : end], u_all[end : end + HZ], CITY_ORDERS)
            orders.append(str(o))
    print(f"  city: {time.time() - t0:.0f}s, AR(1) median {np.nanmedian(ar1):.2f}")

    # Every route that had tickets before the test period.
    top = np.where(X[:, :EVAL_FROM].sum(axis=(1, 2)) > 0)[0]
    jobs = [(Xf[r], U[:, r, :]) for r in top]
    with Pool(8) as pool:
        per_route = np.stack(pool.map(route_job, jobs, chunksize=8))
    print(f"  routes: {time.time() - t0:.0f}s")

    import statsmodels

    vals, counts = np.unique(orders, return_counts=True)
    note = (
        f"statsmodels {statsmodels.__version__} SARIMAX, log(1 + riders) on log(1 + usual) with ARMA errors, "
        f"two weeks of hourly history, refitted at every forecast time; city orders chosen: "
        + ", ".join(f"{v} x{n}" for v, n in zip(vals, counts))
    )
    ARIMA.parent.mkdir(parents=True, exist_ok=True)
    np.savez_compressed(
        ARIMA,
        routes=np.array(routes),
        origins=np.array(ORIGINS),
        eval_days=np.array(EVAL_DAYS),
        city=city,
        city_ar1=ar1,
        top=top,
        per_route=per_route,
        note=np.array(note),
    )
    print(f"  wrote {ARIMA} in {time.time() - t0:.0f}s")


if __name__ == "__main__":
    main()
