"""Chronos forecasts for the live-adjustment backtest (optional; needs requirements-forecast.txt).

Pretrained time-series models, used as they come (no training on Delhi data), on the same forecast
cells as nowcast_backtest.py: every route, every hour from 06:00 to 21:00 on Oct 15-31, the next three
hours, from the tickets up to the forecast time.

- Chronos-Bolt (small), per route: the route's hourly ticketed riders over the last two weeks, nothing else.
- Chronos-2, per route: the same history plus the usual pattern (the average of earlier days of each
  day type, as in the backtest) as a covariate known for the past and the next three hours.
- Chronos-2, city total: all routes added up, with the usual city total as covariate. The backtest
  turns it into a city ratio and applies it to each route's usual pattern.

Forecast = mean of the 0.1-0.9 quantiles. Writes data/out/nowcast/chronos.npz; rerun
nowcast_backtest.py afterwards. Takes about 40 minutes on a laptop (Apple GPU for Chronos-2).
"""

from __future__ import annotations

import sys
import time

import numpy as np
import torch
from chronos import BaseChronosPipeline, Chronos2Pipeline

from nowcast_backtest import CHRONOS, EVAL_DAYS, HZ, ND, ORIGINS, Cells, load_hourly

CTX = 14 * 24  # two weeks of hourly history
QS = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]
BOLT = "amazon/chronos-bolt-small"
C2 = "amazon/chronos-2"


def c2_mean(out: list[torch.Tensor], levels: list[float]) -> np.ndarray:
    idx = [min(range(len(levels)), key=lambda n: abs(levels[n] - q)) for q in QS]
    return np.stack([o[0, idx, :].float().mean(dim=0).cpu().numpy() for o in out])


def main() -> None:
    t0 = time.time()
    routes, dates, dts, X = load_hourly()
    c = Cells(X, dts)
    R = len(routes)
    Xf = X.reshape(R, ND * 24)
    dev2 = "mps" if torch.backends.mps.is_available() else "cpu"
    bolt = BaseChronosPipeline.from_pretrained(BOLT, device_map="cpu", torch_dtype=torch.float32)
    c2 = Chronos2Pipeline.from_pretrained(C2, device_map=dev2)
    levels = [float(x) for x in c2.quantiles]

    out_bolt = np.zeros((R, len(EVAL_DAYS), len(ORIGINS), HZ))
    out_c2 = np.zeros_like(out_bolt)
    city_inputs = []
    n_done, n_total = 0, len(EVAL_DAYS) * len(ORIGINS)
    for i, d in enumerate(EVAL_DAYS):
        # Usual pattern as of this day, laid over every past day by its own day type.
        U = np.stack([c.P[i][:, dts[e], :] for e in range(ND)], axis=1).reshape(R, ND * 24)
        A = c.A[:, i]
        for j, t in enumerate(ORIGINS):
            end = d * 24 + t
            ctx = Xf[:, end - CTX : end]
            use = np.where(A & (ctx.sum(axis=1) > 0))[0]
            # Chronos-Bolt, history only. (Its own "mean" output is the median, so average the quantiles.)
            qb, _ = bolt.predict_quantiles(
                torch.tensor(ctx[use], dtype=torch.float32), prediction_length=HZ, quantile_levels=QS
            )
            out_bolt[use, i, j] = np.clip(qb.float().mean(dim=-1).numpy(), 0, None)
            # Chronos-2 with the usual pattern as a covariate.
            inputs = [
                {
                    "target": ctx[r].astype(np.float32),
                    "past_covariates": {"usual": U[r, end - CTX : end].astype(np.float32)},
                    "future_covariates": {"usual": U[r, end : end + HZ].astype(np.float32)},
                }
                for r in use
            ]
            res = c2.predict(inputs, prediction_length=HZ, batch_size=512)
            out_c2[use, i, j] = np.clip(c2_mean(res, levels), 0, None)
            city_inputs.append(
                {
                    "target": Xf[A, end - CTX : end].sum(axis=0).astype(np.float32),
                    "past_covariates": {"usual": U[A, end - CTX : end].sum(axis=0).astype(np.float32)},
                    "future_covariates": {"usual": U[A, end : end + HZ].sum(axis=0).astype(np.float32)},
                }
            )
            n_done += 1
            el = time.time() - t0
            print(f"  {dates[d]} {t:02d}:00  {len(use)} routes  {n_done}/{n_total}  {el / 60:.1f} min", flush=True)
    res = c2.predict(city_inputs, prediction_length=HZ, batch_size=64)
    city = np.clip(c2_mean(res, levels), 0, None).reshape(len(EVAL_DAYS), len(ORIGINS), HZ)
    import chronos

    note = (
        f"{BOLT} and {C2}, chronos-forecasting {chronos.__version__}, two weeks of hourly history, "
        "forecast = mean of the 0.1-0.9 quantiles, no training on Delhi data"
    )
    CHRONOS.parent.mkdir(parents=True, exist_ok=True)
    np.savez_compressed(
        CHRONOS,
        routes=np.array(routes),
        origins=np.array(ORIGINS),
        eval_days=np.array(EVAL_DAYS),
        bolt=out_bolt,
        c2cov=out_c2,
        c2city=city,
        note=np.array(note),
    )
    print(f"wrote {CHRONOS} in {(time.time() - t0) / 60:.1f} min", file=sys.stderr)


if __name__ == "__main__":
    main()
