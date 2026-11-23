"""Sensitivity of the weekday crowding picture to model assumptions.

Compares the `full` run with each sens_* run (one assumption changed each) and with the low / high
ridership scenarios (levels re-derived from load_low / load_high in `full` with the model's own
level rule and capacity thresholds).

Writes docs/results/sensitivity.md and docs/results/sensitivity.json.
"""

from __future__ import annotations

import numpy as np
import polars as pl

from common import (
    AM_PEAK,
    LEVELS,
    PM_PEAK,
    connect,
    level_names,
    level_probs,
    load_json,
    md_table,
    pct,
    q,
    save_json,
    scalar,
    write_md,
)

SCENARIOS = [
    ("sens_spread", "alighting spread: 50% of riders get off one stop early or late"),
    ("sens_cv", "trip-to-trip load CV 0.50 instead of 0.35"),
    ("sens_noobs", "bus supply from the timetable only (no ticket-based supply)"),
    ("sens_nocap", "no per-route capacity-consistency cap"),
    ("sens_shrink", "shrinkage prior weight 20 trip-days instead of 8"),
]

LV = "case level when 'seats' then 0 when 'standing' then 1 else 2 end"


def shares(con, view: str) -> dict:
    return q(
        con,
        f"""
        select sum(buses_per_day * (level = 'seats')::int) / sum(buses_per_day) seats,
               sum(buses_per_day * (level = 'standing')::int) / sum(buses_per_day) standing,
               sum(buses_per_day * (level = 'crowded')::int) / sum(buses_per_day) crowded,
               sum(buses_per_day * load) / sum(buses_per_day) mean_load,
               sum(buses_per_day * (level <> 'seats' and bin_minute >= {AM_PEAK[0]} and bin_minute < {AM_PEAK[1]})::int)
                 / sum(buses_per_day * (bin_minute >= {AM_PEAK[0]} and bin_minute < {AM_PEAK[1]})::int) am_not_seats,
               sum(buses_per_day) bus_segments
        from {view} where daytype = 'weekday'
        """,
    ).row(0, named=True)


def main() -> dict:
    runs = ["full"] + [r for r, _ in SCENARIOS]
    con = connect(runs)
    meta = load_json("full", "meta.json")
    e = meta["meta"]["expansion"]
    trip_cv = meta["config"]["trip_cv"]
    rows = []
    base = shares(con, "seg_full")
    total_bus = base["bus_segments"]
    full_capped = scalar(con, "select count(*) from routes_full where expansion_adjustment < 1")
    rows.append({"scenario": "full (reference)", "change": f"mid ridership 4M weekday boardings (factor {e['mid']:.1f})", **base, "level changed": 0.0, "more crowded": 0.0, "less crowded": 0.0, "cells missing": 0.0, "shared-cell load ratio": 1.0, "capped routes": full_capped})

    # Ridership scenarios from full: same cells, loads scaled; levels re-derived.
    f = q(con, "select load, load_low, load_high, tickets, seats, comfortable, level, buses_per_day, bin_minute from seg_full where daytype = 'weekday'")
    w = f["buses_per_day"].to_numpy()
    am = ((f["bin_minute"] >= AM_PEAK[0]) & (f["bin_minute"] < AM_PEAK[1])).to_numpy()
    peak = am | ((f["bin_minute"] >= PM_PEAK[0]) & (f["bin_minute"] < PM_PEAK[1])).to_numpy()
    peak_share_all = float(w[peak].sum() / w.sum())
    ref = np.array([LEVELS.index(x) for x in f["level"]])
    _, _, _, mid_lv = level_probs(f["load"].to_numpy(), f["tickets"].to_numpy(), f["seats"].to_numpy(), f["comfortable"].to_numpy(), trip_cv)
    rederive_match = float((mid_lv == ref).mean())
    for col, lab, fac in [("load_low", "ridership low", e["low"]), ("load_high", "ridership high", e["high"])]:
        _, _, _, lv = level_probs(f[col].to_numpy(), f["tickets"].to_numpy(), f["seats"].to_numpy(), f["comfortable"].to_numpy(), trip_cv)
        r = {
            "scenario": lab,
            "change": f"{'3M' if col == 'load_low' else '5M'} weekday boardings (factor {fac:.1f})",
            "seats": float(w[lv == 0].sum() / w.sum()),
            "standing": float(w[lv == 1].sum() / w.sum()),
            "crowded": float(w[lv == 2].sum() / w.sum()),
            "mean_load": float((f[col].to_numpy() * w).sum() / w.sum()),
            "am_not_seats": float(w[am & (lv > 0)].sum() / w[am].sum()),
            "bus_segments": float(w.sum()),
            "level changed": float(w[lv != ref].sum() / w.sum()),
            "changed share in peaks": float(w[(lv != ref) & peak].sum() / max(w[lv != ref].sum(), 1e-9)),
            "more crowded": float(w[lv > ref].sum() / w.sum()),
            "less crowded": float(w[lv < ref].sum() / w.sum()),
            "cells missing": 0.0,
            "shared-cell load ratio": float((f[col].to_numpy() * w).sum() / (f["load"].to_numpy() * w).sum()),
            "capped routes": full_capped,
        }
        rows.append(r)

    for run, desc in SCENARIOS:
        sh = shares(con, f"seg_{run}")
        ch = q(
            con,
            f"""
            select sum(f.buses_per_day) joined_bus,
                   sum(f.buses_per_day * (f.level <> s.level)::int) / sum(f.buses_per_day) changed,
                   sum(f.buses_per_day * ((case s.level when 'seats' then 0 when 'standing' then 1 else 2 end)
                        > (case f.level when 'seats' then 0 when 'standing' then 1 else 2 end))::int) / sum(f.buses_per_day) up,
                   sum(f.buses_per_day * ((case s.level when 'seats' then 0 when 'standing' then 1 else 2 end)
                        < (case f.level when 'seats' then 0 when 'standing' then 1 else 2 end))::int) / sum(f.buses_per_day) down,
                   sum(f.buses_per_day * s.load) / sum(f.buses_per_day * f.load) load_ratio,
                   sum(f.buses_per_day * s.ticketed_rate) / sum(f.buses_per_day * f.ticketed_rate) rate_ratio
            from seg_full f join seg_{run} s using (route_id, pattern, seq, daytype, bin_minute)
            where f.daytype = 'weekday'
            """,
        ).row(0, named=True)
        rows.append(
            {
                "scenario": run,
                "change": desc,
                **sh,
                "level changed": ch["changed"],
                "more crowded": ch["up"],
                "less crowded": ch["down"],
                "cells missing": 1 - ch["joined_bus"] / total_bus,
                "shared-cell load ratio": ch["load_ratio"],
                "shared-cell rate ratio": ch["rate_ratio"],
                "capped routes": scalar(con, f"select count(*) from routes_{run} where expansion_adjustment < 1"),
            }
        )
    tab = pl.DataFrame(rows)
    res = {"rows": rows, "rederived_mid_level_match": rederive_match, "peak_share_of_bus_segments": peak_share_all}
    noobs_bus = next(r["bus_segments"] for r in rows if r["scenario"] == "sens_noobs")
    res["noobs_supply_ratio"] = noobs_bus / total_bus
    nb = next(r for r in rows if r["scenario"] == "sens_noobs")
    only_obs = q(
        con,
        """
        select sum(buses_per_day * load) / sum(buses_per_day) load, sum(buses_per_day) buses
        from seg_full f where daytype = 'weekday' and not exists (
            select 1 from seg_sens_noobs s
            where s.pattern = f.pattern and s.seq = f.seq and s.daytype = f.daytype and s.bin_minute = f.bin_minute)
        """,
    ).row(0, named=True)
    res["observed_only_cells_load"] = only_obs["load"]
    res["observed_only_cells_bus_share"] = only_obs["buses"] / total_bus
    res["max_level_change"] = max(r["level changed"] for r in rows)
    res["max_level_change_scenario"] = max(rows, key=lambda r: r["level changed"])["scenario"]
    save_json("sensitivity", res)

    show = tab.select(
        "scenario",
        "change",
        pl.col("seats").alias("seats"),
        pl.col("standing").alias("standing"),
        pl.col("crowded").alias("crowded"),
        pl.col("am_not_seats").alias("AM peak not seats"),
        pl.col("mean_load").alias("mean load"),
        pl.col("level changed").alias("level changed vs full"),
        pl.col("more crowded"),
        pl.col("less crowded"),
        pl.col("shared-cell load ratio").alias("load vs full (shared cells)"),
        pl.col("capped routes"),
    )
    ordered = sorted(rows[1:], key=lambda r: -r["level changed"])
    md = f"""
# Sensitivity to assumptions

Weekday cells (route pattern x segment x 30-minute bin). Shares of seats / standing / crowded are of
bus-segments (each cell weighted by its buses per day in that run). "Level changed vs full" is the share of
weekday bus-segments (weighted by the `full` run's buses) whose level differs from `full`, split into
cells that become more or less crowded. "AM peak not seats" is the share of bus-segments in the
{AM_PEAK[0] // 60:02d}:00-{AM_PEAK[1] // 60:02d}:00 bins at standing or crowded. The ridership rows reuse `full` with
`load_low` / `load_high` and the same level rule (gamma load, CV sqrt(0.35^2 + 1/tickets), seats and
comfortable thresholds of the bus class) and keep the per-route capacity adjustment computed under the mid
scenario (a high-ridership run would cap more routes, so its crowding is an upper bound); re-deriving the mid level this way reproduces the stored level in
{pct(rederive_match, 3)} of cells.

{md_table(show, {"seats": pct, "standing": pct, "crowded": pct, "AM peak not seats": pct, "mean load": "{:.2f}", "level changed vs full": pct, "more crowded": pct, "less crowded": pct, "load vs full (shared cells)": "{:.3f}"})}

"Load vs full (shared cells)" compares bus-weighted mean load on the cells both runs have, with the `full`
run's bus weights, so composition changes do not blur it. "Capped routes" is the number of routes whose
expansion the capacity check lowered.

Cells of `full` missing from a scenario (bus-weighted): {', '.join(f"{r['scenario']} {pct(r['cells missing'], 2)}" for r in rows if r['cells missing'] > 0) or 'none'}.
Without ticket-based supply (`sens_noobs`) the model has {pct(res['noobs_supply_ratio'])} of the reference
weekday bus-segments. Two things happen. (1) Cells where only rebuilt trips showed a bus drop out
({pct(res['observed_only_cells_bus_share'])} of reference bus-segments); in `full` these carry an average load of
{res['observed_only_cells_load']:.1f}, well above the network mean of {base['mean_load']:.1f}, i.e. the 2025 timetable
carries buses with real riders (partly a selection effect: these cells exist only where tagged tickets were
seen). (2) On the shared cells the ticketed rate per bus rises by
{pct(nb['shared-cell rate ratio'] - 1)}, but the capacity check then lowers {nb['capped routes']} routes instead of
{full_capped}, so the load rises only {pct(nb['shared-cell load ratio'] - 1)}. The mean load over the run's own cells
therefore falls, even though each remaining bus is fuller before the cap.

## Reading

- The ridership assumption matters most: it moves every load by the same factor. Going from 4M to 3M or 5M
  weekday boardings changes the level of {pct(rows[1]['level changed'])} / {pct(rows[2]['level changed'])} of
  weekday bus-segments. Most of the network stays "seats" under all scenarios; {pct(rows[2]['changed share in peaks'])}
  of the cells that change under the high scenario are in the two peak windows, which hold
  {pct(peak_share_all)} of weekday bus-segments.
- Ranked by level changes: {'; '.join(f"{r['scenario']} {pct(r['level changed'])}" for r in ordered)}.
- Supply is the largest of the structural choices (see above): the ticket-based supply adds buses the
  timetable lacks, and the capacity cap absorbs much of the difference when it is removed. The two
  mechanisms interact, so neither should be judged alone.
- The capacity cap only lowers loads on the routes it touches, so dropping it only moves cells toward
  more crowded; the alighting spread and the shrinkage weight barely matter at the network level.
- No assumption flips the overall picture (a large majority of weekday bus-segments with seats, crowding
  concentrated in the AM peak), but the share of peak cells shown as standing or crowded ranges from
  {pct(min(r['am_not_seats'] for r in rows))} to {pct(max(r['am_not_seats'] for r in rows))} across these runs, so the absolute
  crowding level should be presented with its range.
"""
    write_md("sensitivity", md)
    return res


if __name__ == "__main__":
    main()
