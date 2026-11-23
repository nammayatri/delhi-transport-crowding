"""Week-to-week stability: does a model fitted on Oct 1-24 predict Oct 25-31?

Compares the `train` run (Oct 1-24, 15 weekdays) with the `test` run (Oct 25-31, 5 weekdays) on
weekday cells (route pattern x segment x 30-minute bin) present in both. The test run is a model
estimate too (only 5 weekdays, shrunk within the test week); a raw test target (pooled ticket count
per bus pass, no shrinkage) is reported as a robustness check.

Writes docs/results/stability.md, docs/results/stability.json and docs/figures/stability_*.png.
"""

from __future__ import annotations

import matplotlib.pyplot as plt
import numpy as np
import polars as pl

from common import (
    BLUE,
    GREY,
    LEVELS,
    ORANGE,
    SEQ_RAMP,
    TEXT,
    TEXT2,
    cohen_kappa,
    connect,
    level_names,
    level_probs,
    load_json,
    md_table,
    num,
    pct,
    q,
    save,
    save_json,
    scalar,
    setup_style,
    spearman,
    titles,
    write_md,
)

MIN_TEST_TICKETS = 10.0
TICKET_BINS = [0, 1, 3, 10, 20, 40, 100, float("inf")]


def bin_label(lo: float, hi: float) -> str:
    if hi == float("inf"):
        return f"{lo:g}+"
    return f"{lo:g}-{hi:g}"


def level_metrics(pred: np.ndarray, obs: np.ndarray, w: np.ndarray | None = None) -> dict:
    """Agreement, kappa, and how often a non-seats prediction holds."""
    idx = {l: i for i, l in enumerate(LEVELS)}
    pi = np.array([idx[x] for x in pred])
    oi = np.array([idx[x] for x in obs])
    w = np.ones(len(pi)) if w is None else np.asarray(w, dtype=float)
    cm = np.zeros((3, 3))
    np.add.at(cm, (pi, oi), w)
    tot = cm.sum()
    ns_pred = cm[1:, :].sum()
    ns_obs = cm[:, 1:].sum()
    return {
        "n": int(len(pi)),
        "agreement": float(np.trace(cm) / tot) if tot else float("nan"),
        "kappa": cohen_kappa(cm),
        "nonseats_precision": float(cm[1:, 1:].sum() / ns_pred) if ns_pred else float("nan"),
        "nonseats_recall": float(cm[1:, 1:].sum() / ns_obs) if ns_obs else float("nan"),
        "cm": cm,
    }


def main() -> dict:
    setup_style()
    con = connect(["train", "test"])
    meta_tr = load_json("train", "meta.json")
    meta_te = load_json("test", "meta.json")
    nd_te = meta_te["meta"]["n_days"][0]
    e_te = meta_te["meta"]["expansion"]["mid"]
    trip_cv = meta_tr["config"]["trip_cv"]
    res: dict = {"min_test_tickets": MIN_TEST_TICKETS, "test_weekdays": nd_te, "train_weekdays": meta_tr["meta"]["n_days"][0]}

    # Raw test target: kernel-smoothed ticketed crossings per kernel-smoothed bus pass, times the
    # test run's expansion and the route's capacity adjustment (no shrinkage toward priors).
    con.sql(
        f"""
        create table te as
        with b as (select * from seg_test where daytype = 'weekday')
        select t.*,
               0.25 * coalesce(p.buses_per_day, 0) + 0.5 * t.buses_per_day + 0.25 * coalesce(n.buses_per_day, 0) as ks
        from b t
        left join b p on p.pattern = t.pattern and p.seq = t.seq and p.bin_minute = t.bin_minute - 30
        left join b n on n.pattern = t.pattern and n.seq = t.seq and n.bin_minute = t.bin_minute + 30
        """
    )
    con.sql(
        f"""
        create table j as
        select a.route_id, a.route_name, a.pattern, a.seq, a.bin_minute,
               a.load tr_load, b.load te_load, a.level tr_level, b.level te_level,
               a.tickets tr_tix, b.tickets te_tix, a.confidence tr_conf, b.confidence te_conf,
               a.buses_per_day tr_buses, b.buses_per_day te_buses, a.seats, a.comfortable,
               a.from_stop_id = b.from_stop_id same_stop,
               b.tickets / ({nd_te} * b.ks) * {e_te} * coalesce(r.expansion_adjustment, 1.0) te_raw
        from seg_train a
        join te b using (route_id, pattern, seq, daytype, bin_minute)
        left join routes_test r on r.route_id = b.route_id
        where a.daytype = 'weekday'
        """
    )
    n_tr = scalar(con, "select count(*) from seg_train where daytype = 'weekday'")
    n_te = scalar(con, "select count(*) from seg_test where daytype = 'weekday'")
    n_j = scalar(con, "select count(*) from j")
    same = scalar(con, "select avg(same_stop::int) from j")
    res.update({"train_cells": n_tr, "test_cells": n_te, "common_cells": n_j, "same_stop_share": same})

    # Baselines from the train run: route all-day average, segment all-day average.
    con.sql(
        """
        create table base as
        select j.*, r.route_avg, s.seg_avg
        from j
        join (select pattern, sum(load * buses_per_day) / sum(buses_per_day) route_avg
              from seg_train where daytype = 'weekday' group by 1) r using (pattern)
        join (select pattern, seq, sum(load * buses_per_day) / sum(buses_per_day) seg_avg
              from seg_train where daytype = 'weekday' group by 1, 2) s using (pattern, seq)
        """
    )
    df = q(con, "select * from base")
    # Baseline levels: same level rule, CV of trip-to-trip spread only (the baseline pools a large number of tickets).
    for col in ["route_avg", "seg_avg"]:
        _, _, _, lv = level_probs(df[col].to_numpy(), np.full(df.height, 1e9), df["seats"].to_numpy(), df["comfortable"].to_numpy(), trip_cv)
        df = df.with_columns(pl.Series(f"{col}_level", level_names(lv)))
    # Level of the raw test target, using the test tickets for the CV as the model does.
    _, _, _, lv = level_probs(df["te_raw"].fill_nan(0).fill_null(0).to_numpy(), df["te_tix"].to_numpy(), df["seats"].to_numpy(), df["comfortable"].to_numpy(), trip_cv)
    df = df.with_columns(pl.Series("te_raw_level", level_names(lv)))

    S = df.filter(pl.col("te_tix") >= MIN_TEST_TICKETS)
    res["headline_cells"] = S.height
    res["headline_share_of_common"] = S.height / n_j
    res["headline_bus_share_of_common"] = S["te_buses"].sum() / df["te_buses"].sum()

    def within_route_spearman(d: pl.DataFrame, pred: str, obs: str, min_cells: int = 8) -> float:
        rs = []
        ws = []
        for _, g in d.group_by("pattern"):
            if g.height >= min_cells:
                r = spearman(g[pred], g[obs])
                if not np.isnan(r):
                    rs.append(r)
                    ws.append(g.height)
        return float(np.average(rs, weights=ws)) if rs else float("nan")

    def mae(d, pred, obs):
        return float((d[pred] - d[obs]).abs().mean())

    def med_abs_log(d, pred, obs):
        x = d.filter((pl.col(pred) > 0) & (pl.col(obs) > 0))
        return float((x[pred].log() - x[obs].log()).abs().median())

    preds = [("time-of-day model (train)", "tr_load", "tr_level"), ("route all-day average (train)", "route_avg", "route_avg_level"), ("segment all-day average (train)", "seg_avg", "seg_avg_level")]
    comp_rows = []
    for target, tcol, tlev in [("test model", "te_load", "te_level"), ("raw test count", "te_raw", "te_raw_level")]:
        for name, pcol, plev in preds:
            lm = level_metrics(S[plev].to_numpy(), S[tlev].to_numpy())
            comp_rows.append(
                {
                    "target": target,
                    "predictor": name,
                    "Spearman": spearman(S[pcol], S[tcol]),
                    "within-route Spearman": within_route_spearman(S, pcol, tcol),
                    "MAE (riders)": mae(S, pcol, tcol),
                    "median abs log ratio": med_abs_log(S, pcol, tcol),
                    "level agreement": lm["agreement"],
                    "kappa": lm["kappa"],
                }
            )
    comp = pl.DataFrame(comp_rows)
    res["comparison"] = comp_rows
    head = comp.filter((pl.col("target") == "test model") & (pl.col("predictor") == "time-of-day model (train)")).row(0, named=True)
    base_r = comp.filter((pl.col("target") == "test model") & (pl.col("predictor") == "route all-day average (train)")).row(0, named=True)
    seg_r = comp.filter((pl.col("target") == "test model") & (pl.col("predictor") == "segment all-day average (train)")).row(0, named=True)
    raw_r = comp.filter((pl.col("target") == "raw test count") & (pl.col("predictor") == "time-of-day model (train)")).row(0, named=True)
    res["headline_spearman"] = head["Spearman"]
    res["headline_within_route_spearman"] = head["within-route Spearman"]
    res["headline_level_agreement"] = head["level agreement"]
    res["headline_kappa"] = head["kappa"]
    res["baseline_route_spearman"] = base_r["Spearman"]
    res["baseline_route_within_route_spearman"] = base_r["within-route Spearman"]
    res["baseline_route_level_agreement"] = base_r["level agreement"]
    res["baseline_route_mae"] = base_r["MAE (riders)"]
    res["model_mae"] = head["MAE (riders)"]
    res["baseline_segment_spearman"] = seg_r["Spearman"]
    res["raw_target_spearman"] = raw_r["Spearman"]
    res["model_beats_route_baseline"] = bool(
        head["Spearman"] > base_r["Spearman"] and head["MAE (riders)"] < base_r["MAE (riders)"]
    )
    res["spearman_all_common"] = spearman(df["tr_load"], df["te_load"])
    bias = S.filter((pl.col("tr_load") > 0) & (pl.col("te_load") > 0))
    res["headline_median_log_ratio_test_over_train"] = float((bias["te_load"].log() - bias["tr_load"].log()).median())
    both = df.filter((pl.col("te_tix") >= MIN_TEST_TICKETS) & (pl.col("tr_tix") >= 3 * MIN_TEST_TICKETS))
    res["both_periods_cells"] = both.height
    res["both_periods_spearman"] = spearman(both["tr_load"], both["te_load"])
    res["both_periods_level_agreement"] = level_metrics(both["tr_level"].to_numpy(), both["te_level"].to_numpy())["agreement"]
    res["both_periods_kappa"] = level_metrics(both["tr_level"].to_numpy(), both["te_level"].to_numpy())["kappa"]
    lm_all = level_metrics(df["tr_level"].to_numpy(), df["te_level"].to_numpy())
    res["level_agreement_all_common"] = lm_all["agreement"]

    # Level agreement on the headline set, unweighted and bus-weighted, with the confusion matrix.
    lm = level_metrics(S["tr_level"].to_numpy(), S["te_level"].to_numpy())
    lmw = level_metrics(S["tr_level"].to_numpy(), S["te_level"].to_numpy(), S["te_buses"].to_numpy())
    seats_share = float((S["te_level"] == "seats").mean())
    res.update(
        {
            "level_agreement_bus_weighted": lmw["agreement"],
            "nonseats_precision": lm["nonseats_precision"],
            "nonseats_recall": lm["nonseats_recall"],
            "test_seats_share_headline": seats_share,
            "confusion": lm["cm"].tolist(),
        }
    )

    # By train confidence.
    conf_rows = []
    for c in ["low", "medium", "high"]:
        d = S.filter(pl.col("tr_conf") == c)
        if d.height:
            m = level_metrics(d["tr_level"].to_numpy(), d["te_level"].to_numpy())
            conf_rows.append({"train confidence": c, "cells": d.height, "Spearman": spearman(d["tr_load"], d["te_load"]), "level agreement": m["agreement"], "kappa": m["kappa"], "median abs log ratio": med_abs_log(d, "tr_load", "te_load")})
    for c in ["low", "medium", "high"]:
        d = df.filter(pl.col("tr_conf") == c)
        m = level_metrics(d["tr_level"].to_numpy(), d["te_level"].to_numpy())
        conf_rows.append({"train confidence": f"{c} (all common cells)", "cells": d.height, "Spearman": spearman(d["tr_load"], d["te_load"]), "level agreement": m["agreement"], "kappa": m["kappa"], "median abs log ratio": med_abs_log(d, "tr_load", "te_load")})
    conf_df = pl.DataFrame(conf_rows)
    low_hl = next((r["median abs log ratio"] for r in conf_rows if r["train confidence"] == "low"), float("nan"))
    low_k = next((r["kappa"] for r in conf_rows if r["train confidence"] == "low"), float("nan"))
    low_all = next((r["level agreement"] for r in conf_rows if r["train confidence"] == "low (all common cells)"), float("nan"))
    res["by_train_confidence"] = conf_rows

    # Agreement as a function of test tickets.
    tb_rows = []
    for lo, hi in zip(TICKET_BINS[:-1], TICKET_BINS[1:]):
        d = df.filter((pl.col("te_tix") >= lo) & (pl.col("te_tix") < hi))
        if d.height < 20:
            continue
        m = level_metrics(d["tr_level"].to_numpy(), d["te_level"].to_numpy())
        tb_rows.append(
            {
                "test tickets": bin_label(lo, hi),
                "cells": d.height,
                "Spearman model": spearman(d["tr_load"], d["te_load"]),
                "Spearman route average": spearman(d["route_avg"], d["te_load"]),
                "Spearman model vs raw": spearman(d["tr_load"], d["te_raw"]),
                "level agreement": m["agreement"],
                "kappa": m["kappa"],
                "median abs log ratio": med_abs_log(d, "tr_load", "te_load"),
            }
        )
    tb = pl.DataFrame(tb_rows)
    res["by_test_tickets"] = tb_rows

    # Route x hour totals (passengers through, summed over segments): less noisy.
    rh = q(
        con,
        """
        with a as (select pattern, bin_minute // 60 hr, sum(load * buses_per_day) pax, sum(tickets) tix
                   from seg_train where daytype = 'weekday' group by 1, 2),
             b as (select pattern, bin_minute // 60 hr, sum(load * buses_per_day) pax, sum(tickets) tix
                   from seg_test where daytype = 'weekday' group by 1, 2)
        select a.pattern, a.hr, a.pax tr_pax, b.pax te_pax, a.tix tr_tix, b.tix te_tix
        from a join b using (pattern, hr)
        """,
    )
    rhS = rh.filter(pl.col("te_tix") >= MIN_TEST_TICKETS)
    res["route_hour_cells"] = rh.height
    res["route_hour_cells_headline"] = rhS.height
    res["route_hour_spearman"] = spearman(rhS["tr_pax"], rhS["te_pax"])
    res["route_hour_spearman_all"] = spearman(rh["tr_pax"], rh["te_pax"])
    lr = rhS.filter((pl.col("tr_pax") > 0) & (pl.col("te_pax") > 0))
    res["route_hour_pearson_log"] = float(np.corrcoef(np.log(lr["tr_pax"]), np.log(lr["te_pax"]))[0, 1])
    res["route_hour_median_abs_log_ratio"] = float((lr["tr_pax"].log() - lr["te_pax"].log()).abs().median())
    # Route-level hourly profile shape: within-route Spearman across hours.
    shp = []
    for _, g in rhS.group_by("pattern"):
        if g.height >= 8:
            shp.append(spearman(g["tr_pax"], g["te_pax"]))
    res["route_hour_within_route_spearman_median"] = float(np.nanmedian(shp)) if shp else float("nan")
    res["route_hour_within_route_routes"] = len(shp)

    save_json("stability", {k: v for k, v in res.items() if k != "confusion"} | {"confusion": res["confusion"]})

    # ---- figures ----------------------------------------------------------------------------
    fig, axes = plt.subplots(1, 2, figsize=(11, 4.8))
    ax = axes[0]
    x, y = S["tr_load"].to_numpy(), S["te_load"].to_numpy()
    ok = (x > 0.1) & (y > 0.1)
    hb = ax.hexbin(x[ok], y[ok], xscale="log", yscale="log", gridsize=45, cmap=plt.matplotlib.colors.ListedColormap(SEQ_RAMP), mincnt=1, bins="log", linewidths=0)
    lim = [0.3, 200]
    ax.plot(lim, lim, color=TEXT2, lw=1)
    ax.set_xlim(lim)
    ax.set_ylim(lim)
    ax.set_xlabel("train load (Oct 1-24), riders per bus")
    ax.set_ylabel("test load (Oct 25-31), riders per bus")
    ax.set_title(f"Cells with >= {MIN_TEST_TICKETS:g} test tickets: Spearman {head['Spearman']:.2f}", fontsize=10)
    ax.grid(visible=False)
    cb = fig.colorbar(hb, ax=ax, fraction=0.046, pad=0.02)
    cb.set_label("cells", color=TEXT2)
    cb.outline.set_visible(False)
    ax = axes[1]
    x, y = rhS["tr_pax"].to_numpy(), rhS["te_pax"].to_numpy()
    ok = (x > 1) & (y > 1)
    hb = ax.hexbin(x[ok], y[ok], xscale="log", yscale="log", gridsize=45, cmap=plt.matplotlib.colors.ListedColormap(SEQ_RAMP), mincnt=1, bins="log", linewidths=0)
    lo = max(min(x[ok].min(), y[ok].min()), 1)
    hi = max(x[ok].max(), y[ok].max())
    ax.plot([lo, hi], [lo, hi], color=TEXT2, lw=1)
    ax.set_xlabel("train, passengers through per day (sum over segments)")
    ax.set_ylabel("test, passengers through per day")
    ax.set_title(f"Route x hour totals: Spearman {res['route_hour_spearman']:.2f}", fontsize=10)
    ax.grid(visible=False)
    cb = fig.colorbar(hb, ax=ax, fraction=0.046, pad=0.02)
    cb.set_label("route-hours", color=TEXT2)
    cb.outline.set_visible(False)
    fig.suptitle(
        f"Loads fitted on Oct 1-24 track the test week: rank correlation {head['Spearman']:.2f} per cell, "
        f"{res['route_hour_spearman']:.2f} per route-hour",
        x=0.05, ha="left", fontsize=12, fontweight="bold",
    )
    fig.text(0.05, 0.9, "Weekday cells present in both runs; diagonal = same value in both periods", fontsize=9, color=TEXT2)
    fig.subplots_adjust(top=0.8, wspace=0.35)
    fig_sc = save(fig, "stability_scatter")

    cm = lm["cm"]
    rown = cm / cm.sum(axis=1, keepdims=True)
    fig, ax = plt.subplots(figsize=(6.2, 5))
    ax.imshow(rown, cmap=plt.matplotlib.colors.ListedColormap(SEQ_RAMP), vmin=0, vmax=1)
    for i in range(3):
        for k in range(3):
            v = rown[i, k]
            ax.text(k, i, f"{v:.0%}\n({int(cm[i, k]):,})", ha="center", va="center", fontsize=9, color="white" if v > 0.55 else TEXT)
    ax.set_xticks(range(3), LEVELS)
    ax.set_yticks(range(3), LEVELS)
    ax.set_xlabel("level in test week (Oct 25-31)")
    ax.set_ylabel("level from train period (Oct 1-24)")
    ax.grid(visible=False)
    for sp in ax.spines.values():
        sp.set_visible(False)
    titles(
        ax,
        f"Levels agree on {lm['agreement']:.0%} of cells (kappa {lm['kappa']:.2f})",
        f"Row-normalised; cells with >= {MIN_TEST_TICKETS:g} test tickets, counts in brackets",
    )
    fig_cm = save(fig, "stability_confusion")

    fig, ax = plt.subplots(figsize=(9, 4.4))
    xs = np.arange(tb.height)
    ax.plot(xs, tb["Spearman model"], color=BLUE, marker="o", label="Spearman: time-of-day model vs test")
    ax.plot(xs, tb["Spearman route average"], color=GREY, marker="o", label="Spearman: route all-day average vs test")
    ax.plot(xs, tb["kappa"], color=ORANGE, marker="o", label="level kappa: model vs test")
    ax.set_xticks(xs, tb["test tickets"].to_list())
    ax.set_xlabel("ticketed riders behind the test-week estimate (kernel-weighted)")
    ax.set_ylabel("agreement (0 = none, 1 = perfect)")
    ax.set_ylim(-0.15, 1)
    ax.axhline(0, color=TEXT2, lw=0.8)
    ax.legend(loc="upper left", bbox_to_anchor=(0, -0.18), ncols=2)
    titles(
        ax,
        f"Level agreement beyond chance (kappa) rises from {tb['kappa'][0]:.2f} to {tb['kappa'].max():.2f} as test-week tickets grow",
        "Weekday cells present in both runs, grouped by test-week ticket count",
    )
    fig_tb = save(fig, "stability_vs_tickets")

    # ---- markdown -----------------------------------------------------------------------------
    cm_df = pl.DataFrame({"train \\ test": LEVELS} | {lv: [int(v) for v in cm[:, i]] for i, lv in enumerate(LEVELS)})
    md = f"""
# Week-to-week stability

Train: run `train` (Oct 1-24, {res['train_weekdays']} weekdays). Test: run `test` (Oct 25-31,
{nd_te} weekdays). Both are full model runs on disjoint dates; each is expanded with its own ridership
factor and capacity check. Compared on weekday cells (route pattern x stop-to-stop segment x 30-minute
bin) present in both runs: {n_j:,} of {n_tr:,} train and {n_te:,} test cells; the boarding stop of each
segment agrees in {pct(same, 2)} of joined cells, so cell keys line up.

**Threshold.** The test week has only {nd_te} weekdays, so test estimates are themselves noisy. Headline
numbers use the {S.height:,} cells with at least **{MIN_TEST_TICKETS:g} kernel-weighted ticketed riders in the
test week** ({pct(res['headline_share_of_common'])} of common cells, {pct(res['headline_bus_share_of_common'])} of
their buses). Results for all cells are given below for completeness.

## Headline

![scatter]({fig_sc})

- Rank correlation (Spearman) of load, train vs test: **{head['Spearman']:.2f}** (all common cells:
  {res['spearman_all_common']:.2f}). Within a route (ranking cells of the same route against each other,
  averaged over routes): {head['within-route Spearman']:.2f}.
- Level agreement: **{pct(head['level agreement'])}** of cells (bus-weighted {pct(lmw['agreement'])}),
  Cohen's kappa {head['kappa']:.2f}. {pct(seats_share)} of these cells show "seats" in the test week, so raw
  agreement is flattered by the dominant class; kappa corrects for that.
- When the train period says standing or crowded, the test week agrees (standing or crowded) in
  {pct(lm['nonseats_precision'])} of cells; of the test-week standing/crowded cells, the train period caught
  {pct(lm['nonseats_recall'])}.
- Route x hour totals (passengers through per day summed over a route's segments, {rhS.height:,} route-hours with
  >= {MIN_TEST_TICKETS:g} test tickets): Spearman **{res['route_hour_spearman']:.2f}**, Pearson on logs
  {res['route_hour_pearson_log']:.2f}, median absolute log ratio {res['route_hour_median_abs_log_ratio']:.2f}
  (about {np.exp(res['route_hour_median_abs_log_ratio']) - 1:.0%} typical difference). Within a route, the hourly
  profile shape correlates at a median Spearman of {res['route_hour_within_route_spearman_median']:.2f}
  ({res['route_hour_within_route_routes']:,} routes with >= 8 qualifying hours).

## Does time of day add information? Baselines

Each predictor is fitted on the train period and scored on the {S.height:,} headline cells. "Route all-day
average" predicts every cell of a route with the route's bus-weighted average weekday load; "segment all-day
average" uses the segment's own all-day average (a stronger baseline: it knows where on the route the load
builds up, but not when).

{md_table(comp, {"Spearman": "{:.3f}", "within-route Spearman": lambda v: "n/a" if v != v else f"{v:.3f}", "MAE (riders)": "{:.2f}", "median abs log ratio": "{:.3f}", "level agreement": pct, "kappa": "{:.3f}"})}

The time-of-day model {'beats' if res['model_beats_route_baseline'] else 'does NOT beat'} the route all-day average
(Spearman {head['Spearman']:.2f} vs {base_r['Spearman']:.2f}; mean absolute error {head['MAE (riders)']:.1f} vs
{base_r['MAE (riders)']:.1f} riders per bus) and the segment all-day average (Spearman {seg_r['Spearman']:.2f}).
The gap is widest within routes, which is where time of day is the only information: within-route Spearman
{head['within-route Spearman']:.2f} for the model; the route average is a constant within a route, so it
cannot rank cells of the same route at all (its within-route rank correlation is undefined;
the segment average gets {seg_r['within-route Spearman']:.2f}).

Against the raw test count (no shrinkage, so it does not borrow from neighbouring bins or the route), the
model's Spearman is {raw_r['Spearman']:.2f}: lower, because the raw count carries the full Poisson noise of a
few tickets, but the ordering of predictors is the same.

## Confusion matrix of levels (cells with >= {MIN_TEST_TICKETS:g} test tickets)

![confusion]({fig_cm})

{md_table(cm_df)}

## By confidence class of the train cell

{md_table(conf_df, {"Spearman": "{:.3f}", "level agreement": pct, "kappa": "{:.3f}", "median abs log ratio": "{:.3f}"})}

Rows without "(all common cells)" are restricted to the headline cells. The weak spot is the small group of
cells that were low confidence in the train period but well observed in the test week: their median absolute
log ratio is {low_hl:.2f} (a typical factor of {np.exp(low_hl):.1f} between periods) and kappa {low_k:.2f}. These are
cells where the train period had too few tickets to say much, so the estimate was mostly the prior. Medium
and high confidence cells behave alike. Over all common cells, the high agreement of low-confidence cells
({pct(low_all)}) only reflects that both periods show "seats" for nearly all of them.

## Agreement against test-week tickets

![vs tickets]({fig_tb})

{md_table(tb, {"Spearman model": "{:.3f}", "Spearman route average": "{:.3f}", "Spearman model vs raw": "{:.3f}", "level agreement": pct, "kappa": "{:.3f}", "median abs log ratio": "{:.3f}"})}

The typical difference between periods (median absolute log ratio) falls from {tb['median abs log ratio'][0]:.2f}
in cells with {tb['test tickets'][0]} test tickets to {tb['median abs log ratio'].min():.2f} in the best-observed
group, and kappa rises from {tb['kappa'][0]:.2f} to {tb['kappa'].max():.2f}. If the crowding pattern had shifted between
the two periods, well-observed cells would disagree about as much as sparse ones; instead disagreement
shrinks as the test week's sample grows, which is what sampling noise produces. Raw level agreement moves
the other way (near 100% in sparse cells) only because sparse cells are almost all "seats" in both periods.

Two cautions on the sparse end. First, in cells with few tickets both periods' estimates lean on the same
kind of prior (the route's level times the city-wide time-of-day shape), so their Spearman of
{tb['Spearman model'][0]:.2f} at {tb['test tickets'][0]} tickets partly measures agreement between priors, not
between observations. Second, selecting cells by test-week tickets could favour cells that happened to be busy in
the test week; the effect is small here (median test/train load ratio in the headline set
{np.exp(res['headline_median_log_ratio_test_over_train']):.2f}). Selecting on both periods instead (train >= {3 * MIN_TEST_TICKETS:g} and
test >= {MIN_TEST_TICKETS:g} tickets, {res['both_periods_cells']:,} cells) gives Spearman {res['both_periods_spearman']:.2f}, level
agreement {pct(res['both_periods_level_agreement'])} and kappa {res['both_periods_kappa']:.2f}.

## Caveats

- Real change between early and late October (festival season, weather) is counted as disagreement
  here; the numbers above include it rather than correct for it.
- Both runs use the same bus supply (timetable and live feed) and the same matching; errors shared by both (missing
  routes, wrong stop matches, a wrong city-wide expansion factor) cannot show up in a train/test comparison.
  This check measures stability, not absolute accuracy.
"""
    write_md("stability", md)
    return res


if __name__ == "__main__":
    main()
