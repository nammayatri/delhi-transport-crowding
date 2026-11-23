"""Data audit: what the ticket file contains, how it was matched to the timetable, and
whether matched trip lengths agree with the fares paid.

Writes docs/results/audit.md, docs/results/audit.json and docs/figures/audit_*.png.
"""

from __future__ import annotations

import datetime as dt
import time

import matplotlib.pyplot as plt
from matplotlib.patches import Patch
import numpy as np
import polars as pl

from common import (
    AQUA,
    BLUE,
    DAYTYPE_COLORS,
    GREY,
    GRID,
    ORANGE,
    TEXT2,
    TICKETS_CSV,
    connect,
    load_json,
    md_table,
    num,
    pct,
    q,
    save,
    save_json,
    scalar,
    setup_style,
    titles,
    write_md,
)

# Assumed distance stages of the city bus fare (full fare Rs 5/10/15 non-AC and
# Rs 10/15/20/25 AC; digital tickets carry a 10% discount). Straight-line km along stops.
STAGES = {
    False: {4.5: (0, 4), 9.0: (4, 10), 13.5: (10, None)},
    True: {9.0: (0, 4), 13.5: (4, 8), 18.0: (8, 12), 22.5: (12, None)},
}


def load_raw(con) -> int:
    t0 = time.time()
    raw = (
        pl.read_csv(TICKETS_CSV, null_values=["NULL"], infer_schema=False)
        .with_row_index("ticket")
        .with_columns(
            pl.col("passenger_count").cast(pl.Int32),
            pl.col("amount").cast(pl.Float64),
            pl.col("ticket_type_id").cast(pl.Int32),
            (pl.col("is_ac") == "True").alias("is_ac"),
            pl.col("created_at").str.slice(0, 10).alias("date"),
            pl.col("created_at").str.slice(11, 2).cast(pl.Int32).alias("hour"),
        )
    )
    con.register("raw_df", raw.to_arrow())
    con.sql(
        """
        create or replace table raw as
        select *, case when dayofweek(date::date) = 6 then 'saturday'
                       when dayofweek(date::date) = 0 or date in ('2026-10-02', '2026-10-20') then 'sunday'
                       else 'weekday' end as daytype,
                  date in ('2026-10-02', '2026-10-20') as holiday
        from raw_df
        """
    )
    print(f"  read {raw.height:,} ticket rows in {time.time() - t0:.1f}s")
    return raw.height


def main() -> dict:
    setup_style()
    con = connect(["full"])
    n_rows = load_raw(con)
    meta = load_json("full", "meta.json")
    rep = load_json("full", "match_report.json")
    res: dict = {"rows": n_rows}

    # ---- basic composition ---------------------------------------------------------
    comp = q(
        con,
        """
        select count(*) n, sum(passenger_count) pax,
               avg((vehicle_number is not null)::int) veh_share,
               avg(is_ac::int) ac_share,
               count(distinct route) routes,
               count(distinct ticket_type_id) ticket_types,
               min(ticket_type_id) ticket_type,
               avg((created_at not like '%+05:30')::int) non_ist,
               avg((start_stop_name = end_stop_name)::int) same_stop,
               count(distinct vehicle_number) vehicles
        from raw
        """,
    ).row(0, named=True)
    res.update(
        {
            "pax": comp["pax"],
            "vehicle_share": comp["veh_share"],
            "ac_share": comp["ac_share"],
            "routes": comp["routes"],
            "ticket_types": comp["ticket_types"],
            "non_ist_share": comp["non_ist"],
            "same_stop_share": comp["same_stop"],
            "vehicles": comp["vehicles"],
        }
    )

    # ---- per day ---------------------------------------------------------------------
    per_day = q(
        con,
        """
        select date, min(daytype) daytype, min(holiday) holiday, count(*) n,
               sum(passenger_count) pax, avg((vehicle_number is not null)::int) veh_share,
               count(distinct vehicle_number) vehicles
        from raw group by date order by date
        """,
    )
    by_dt = per_day.group_by("daytype").agg(
        pl.len().alias("days"), pl.col("n").mean().alias("tickets_per_day"), pl.col("pax").mean().alias("pax_per_day")
    )
    dtm = {r["daytype"]: r for r in by_dt.iter_rows(named=True)}
    res["tickets_per_day"] = {k: v["tickets_per_day"] for k, v in dtm.items()}
    res["pax_per_day"] = {k: v["pax_per_day"] for k, v in dtm.items()}
    res["days"] = {k: v["days"] for k, v in dtm.items()}
    wk = per_day.filter(pl.col("daytype") == "weekday")
    train_wd = wk.filter(pl.col("date") <= "2026-10-24")["n"].mean()
    test_wd = wk.filter(pl.col("date") >= "2026-10-25")["n"].mean()
    res["weekday_tickets_train"] = train_wd
    res["weekday_tickets_test"] = test_wd
    hol = per_day.filter(pl.col("holiday"))
    res["holiday_tickets"] = dict(zip(hol["date"].to_list(), hol["n"].to_list()))
    lowest_wd = wk.sort("n").head(3)
    res["lowest_weekdays"] = dict(zip(lowest_wd["date"].to_list(), lowest_wd["n"].to_list()))

    fig, ax = plt.subplots(figsize=(10, 4.2))
    dates = [dt.date.fromisoformat(d) for d in per_day["date"]]
    colors = [DAYTYPE_COLORS[d] for d in per_day["daytype"]]
    ax.bar(dates, per_day["n"] / 1000, color=colors, width=0.75)
    for d, n, h in zip(dates, per_day["n"], per_day["holiday"]):
        if h:
            ax.text(d, n / 1000 + 4, "holiday", ha="center", fontsize=8, color=TEXT2)
    handles = [
        Patch(color=DAYTYPE_COLORS[k], label=lab)
        for k, lab in [("weekday", "weekday"), ("saturday", "Saturday"), ("sunday", "Sunday or holiday")]
    ]
    ax.legend(handles=handles, loc="upper left", ncols=3, bbox_to_anchor=(0, 1.0))
    ax.set_ylim(0, per_day["n"].max() / 1000 * 1.18)
    ax.set_ylabel("tickets per day (thousands)")
    ax.set_xlabel("date (October 2026)")
    ax.xaxis.set_major_formatter(plt.matplotlib.dates.DateFormatter("%d %b"))
    titles(
        ax,
        f"Ticket volume follows the working week: {dtm['weekday']['tickets_per_day'] / 1000:.0f}k per weekday, "
        f"{dtm['sunday']['tickets_per_day'] / 1000:.0f}k per Sunday or holiday",
        "Mobile/QR tickets per calendar day; Oct 2 and Oct 20 are holidays and are modelled as Sundays",
    )
    fig_day = save(fig, "audit_tickets_per_day")

    # ---- by hour ------------------------------------------------------------------
    by_hour = q(
        con,
        """
        with d as (select daytype, count(distinct date) nd from raw group by 1)
        select r.daytype, hour, count(*) / min(d.nd) per_day
        from raw r join d using (daytype) group by 1, 2 order by 1, 2
        """,
    )
    wdh = by_hour.filter(pl.col("daytype") == "weekday")
    am = wdh.filter(pl.col("hour") < 13).sort("per_day", descending=True).row(0, named=True)
    pm = wdh.filter(pl.col("hour") >= 13).sort("per_day", descending=True).row(0, named=True)
    res["weekday_peak_hour_am"] = am["hour"]
    res["weekday_peak_hour_pm"] = pm["hour"]
    res["weekday_peak_hour_am_tickets"] = am["per_day"]
    res["weekday_peak_hour_pm_tickets"] = pm["per_day"]
    res["out_of_hours_share"] = scalar(con, "select avg((hour < 4)::int) from raw")

    fig, ax = plt.subplots(figsize=(9, 4))
    for k, lab in [("weekday", "weekday"), ("saturday", "Saturday"), ("sunday", "Sunday or holiday")]:
        s = by_hour.filter(pl.col("daytype") == k)
        ax.plot(s["hour"], s["per_day"] / 1000, color=DAYTYPE_COLORS[k], label=lab, marker="o", markersize=3.5)
    ax.set_xticks(range(0, 24, 2))
    ax.set_xticklabels([f"{h:02d}:00" for h in range(0, 24, 2)])
    ax.set_xlabel("hour of purchase (local time)")
    ax.set_ylabel("tickets per day in the hour (thousands)")
    ax.legend(loc="upper right")
    titles(
        ax,
        f"Weekday ticket purchases peak at {am['hour']:02d}:00 and {pm['hour']:02d}:00",
        "Average tickets per day bought in each clock hour, by day type",
    )
    fig_hour = save(fig, "audit_tickets_by_hour")

    # ---- vehicle number share ---------------------------------------------------------
    veh = q(
        con,
        """
        select is_ac, daytype, count(*) n, avg((vehicle_number is not null)::int) share
        from raw group by all order by all
        """,
    )
    veh_ac = q(con, "select is_ac, avg((vehicle_number is not null)::int) share from raw group by 1 order by 1")
    res["vehicle_share_by_ac"] = {("AC" if r[0] else "non-AC"): r[1] for r in veh_ac.iter_rows()}
    vmin, vmax = per_day["veh_share"].min(), per_day["veh_share"].max()
    vmax_date = per_day.sort("veh_share", descending=True)["date"][0]
    high_days = per_day.filter(pl.col("veh_share") > 0.6)
    res["vehicle_share_day_min"] = vmin
    res["vehicle_share_day_max"] = vmax
    res["vehicle_share_day_max_date"] = vmax_date
    res["vehicle_share_high_days"] = dict(zip(high_days["date"].to_list(), high_days["veh_share"].to_list()))
    res["vehicles_per_weekday"] = wk["vehicles"].mean()

    fig, ax = plt.subplots(figsize=(10, 3.8))
    ax.plot(dates, per_day["veh_share"] * 100, color=BLUE, marker="o", markersize=4)
    ax.set_ylim(0, 100)
    ax.set_ylabel("tickets with a vehicle number (%)")
    ax.set_xlabel("date (October 2026)")
    ax.xaxis.set_major_formatter(plt.matplotlib.dates.DateFormatter("%d %b"))
    titles(
        ax,
        f"{comp['veh_share']:.0%} of tickets carry a vehicle number; the daily share jumps to {vmax:.0%} on "
        f"{dt.date.fromisoformat(vmax_date):%d %b}",
        f"Share of tickets per day with vehicle_number set; about {wk['vehicles'].mean():,.0f} distinct buses appear per weekday",
    )
    fig_veh = save(fig, "audit_vehicle_share")

    # ---- passenger count and fares -------------------------------------------------
    pc = q(
        con,
        """
        select passenger_count, count(*) tickets, count(*) / sum(count(*)) over () share,
               sum(passenger_count) pax
        from raw group by 1 order by 1
        """,
    )
    res["single_rider_share"] = pc.filter(pl.col("passenger_count") == 1)["share"].item()
    fares = q(
        con,
        """
        select case when is_ac then 'AC' else 'non-AC' end bus, amount / passenger_count fare_per_pax,
               count(*) tickets, count(*) / sum(count(*)) over () share
        from raw group by all order by tickets desc limit 12
        """,
    )
    main_fares = q(
        con,
        """
        select avg((amount / passenger_count in (4.5, 9, 13.5, 18, 22.5))::int) from raw
        """,
    ).item()
    res["standard_fare_share"] = main_fares

    # ---- trip km vs fare (exact stop matches only; imputed trips take km from fare) ---
    fk = q(
        con,
        """
        select is_ac, fare_per_pax, count(*) n,
               quantile_cont(km, 0.05) p05, quantile_cont(km, 0.25) p25, quantile_cont(km, 0.5) p50,
               quantile_cont(km, 0.75) p75, quantile_cont(km, 0.95) p95
        from tix_full
        where method in ('exact', 'direction_fixed') and is_ac is not null
          and ((not is_ac and fare_per_pax in (4.5, 9, 13.5)) or (is_ac and fare_per_pax in (9, 13.5, 18, 22.5)))
        group by all order by all
        """,
    )
    within = []
    for ac, stages in STAGES.items():
        for fare, (lo, hi) in stages.items():
            cond = f"km >= {lo}" + (f" and km < {hi}" if hi is not None else "")
            share = scalar(
                con,
                f"""select avg(({cond})::int) from tix_full
                    where method in ('exact', 'direction_fixed') and is_ac = {str(ac).lower()} and fare_per_pax = {fare}""",
            )
            within.append({"bus": "AC" if ac else "non-AC", "fare_per_pax": fare, "stage_km": f"{lo}-{hi if hi else ''}", "share_in_stage": share})
    within_df = pl.DataFrame(within)
    fk = fk.with_columns(pl.when(pl.col("is_ac")).then(pl.lit("AC")).otherwise(pl.lit("non-AC")).alias("bus"))
    fk = fk.join(within_df, on=["bus", "fare_per_pax"], how="left")
    medians = {f"{r['bus']} {r['fare_per_pax']:g}": r["p50"] for r in fk.iter_rows(named=True)}
    res["fare_median_km"] = medians
    res["fare_share_in_stage"] = {f"{r['bus']} {r['fare_per_pax']:g}": r["share_in_stage"] for r in fk.iter_rows(named=True)}
    tot_exact = fk["n"].sum()
    res["fare_share_in_stage_overall"] = float((fk["n"] * fk["share_in_stage"]).sum() / tot_exact)
    mono = True
    for bus in ["non-AC", "AC"]:
        m = fk.filter(pl.col("bus") == bus).sort("fare_per_pax")["p50"].to_list()
        mono = mono and all(b > a for a, b in zip(m, m[1:]))
    res["fare_km_monotonic"] = mono

    fig, axes = plt.subplots(1, 2, figsize=(10, 4.4), sharey=True, gridspec_kw={"width_ratios": [3, 4]})
    for ax, bus, col in [(axes[0], "non-AC", ORANGE), (axes[1], "AC", BLUE)]:
        s = fk.filter(pl.col("bus") == bus).sort("fare_per_pax")
        stats = [
            {"med": r["p50"], "q1": r["p25"], "q3": r["p75"], "whislo": r["p05"], "whishi": r["p95"], "fliers": [], "label": f"Rs {r['fare_per_pax']:g}"}
            for r in s.iter_rows(named=True)
        ]
        for i, r in enumerate(s.iter_rows(named=True)):
            lo, hi = STAGES[bus == "AC"][r["fare_per_pax"]]
            ax.add_patch(plt.Rectangle((i + 1 - 0.42, lo), 0.84, (hi if hi else 40) - lo, color=GRID, zorder=0, lw=0))
        ax.bxp(
            stats,
            widths=0.45,
            patch_artist=True,
            boxprops={"facecolor": col, "edgecolor": col, "alpha": 0.9},
            medianprops={"color": "white", "linewidth": 2},
            whiskerprops={"color": col, "linewidth": 1.2},
            capprops={"color": col, "linewidth": 1.2},
        )
        for i, r in enumerate(s.iter_rows(named=True)):
            ax.text(i + 1.27, r["p50"], f"{r['p50']:.1f} km", va="center", fontsize=8, color=TEXT2)
        ax.set_title(f"{bus} buses", fontsize=10, loc="left")
        ax.set_xlabel("fare paid per passenger")
        ax.grid(axis="x", visible=False)
    axes[0].set_ylabel("trip length, km (straight line along stops)")
    axes[0].set_ylim(0, 36)
    fig.suptitle(
        f"Trip length rises with fare: AC median {medians['AC 9']:.1f} km at Rs 9 to {medians['AC 22.5']:.1f} km at Rs 22.5",
        x=0.06, ha="left", fontsize=12, fontweight="bold",
    )
    fig.text(
        0.06, 0.905,
        "Tickets with both stops matched. Box = 25-75%, whiskers = 5-95%; grey band = assumed fare stage for that fare",
        fontsize=9, color=TEXT2, ha="left",
    )
    fig.subplots_adjust(top=0.8, wspace=0.08)
    fig_fare = save(fig, "audit_fare_vs_km")

    # ---- match outcome ----------------------------------------------------------------
    bm, rj = rep["by_method"], rep["rejected"]
    outcomes = [
        ("both stops matched", bm.get("Exact", 0)),
        ("matched after flipping direction", bm.get("DirectionFixed", 0)),
        ("boarding stop matched, alighting imputed", bm.get("Imputed", 0)),
        ("route not in 2025 timetable", rj.get("RouteNotInGtfs", 0)),
        ("boarding stop not found on route", rj.get("StartStopNotOnRoute", 0)),
    ]
    for k, v in rj.items():
        if k not in ("RouteNotInGtfs", "StartStopNotOnRoute"):
            outcomes.append((k, v))
    tot = rep["tickets"]
    out_df = pl.DataFrame({"outcome": [o[0] for o in outcomes], "tickets": [o[1] for o in outcomes]}).with_columns(
        (pl.col("tickets") / tot).alias("share")
    )
    res["match"] = {o: s for o, s in zip(out_df["outcome"], out_df["share"])}
    res["matched_share"] = rep["assigned"] / tot
    res["route_missing_share"] = rj.get("RouteNotInGtfs", 0) / tot
    res["stop_missing_share"] = rj.get("StartStopNotOnRoute", 0) / tot
    res["imputed_share"] = bm.get("Imputed", 0) / tot
    res["geo_match_share_of_assigned"] = rep["geo_matches"] / rep["assigned"]

    fig, ax = plt.subplots(figsize=(9, 3.4))
    labels = out_df["outcome"].to_list()[::-1]
    vals = (out_df["share"] * 100).to_list()[::-1]
    cols = [GREY if ("not" in l) else BLUE for l in labels]
    ax.barh(labels, vals, color=cols, height=0.6)
    for i, v in enumerate(vals):
        ax.text(v + 0.8, i, f"{v:.1f}%", va="center", fontsize=9, color=TEXT2)
    ax.set_xlim(0, 100)
    ax.set_xlabel("share of tickets (%)")
    ax.grid(axis="y", visible=False)
    ax.grid(axis="x", visible=True)
    titles(
        ax,
        f"{res['matched_share']:.0%} of tickets were matched to the 2025 timetable",
        "Blue = used by the model, grey = dropped",
    )
    fig_match = save(fig, "audit_match_outcome")

    # ---- unknown routes ---------------------------------------------------------------
    # Ticket route names with no matched ticket at all approximate "route not in GTFS".
    fam_sql = """case when route like 'D-%' then 'D- family'
                      when route like 'OMS%' then 'OMS family'
                      else 'other' end"""
    unk = q(
        con,
        f"""
        with matched_routes as (select distinct ticket_route from tix_full)
        select {fam_sql} family, count(*) tickets, count(distinct route) routes
        from raw where route not in (select ticket_route from matched_routes)
        group by 1 order by tickets desc
        """,
    )
    unk_total = unk["tickets"].sum()
    res["unknown_route_tickets_est"] = unk_total
    res["unknown_family_share"] = {r["family"]: r["tickets"] / unk_total for r in unk.iter_rows(named=True)}
    res["unknown_family_routes"] = {r["family"]: r["routes"] for r in unk.iter_rows(named=True)}

    top = pl.DataFrame(rep["unknown_routes"], schema=["route", "tickets"], orient="row").head(20)
    top = top.with_columns(
        pl.when(pl.col("route").str.starts_with("D-")).then(pl.lit("D- family"))
        .when(pl.col("route").str.starts_with("OMS")).then(pl.lit("OMS family"))
        .otherwise(pl.lit("other")).alias("family")
    )
    fam_col = {"D- family": BLUE, "OMS family": ORANGE, "other": GREY}
    fig, ax = plt.subplots(figsize=(9, 5.6))
    rows = top.reverse()
    ax.barh(rows["route"], rows["tickets"] / 1000, color=[fam_col[f] for f in rows["family"]], height=0.65)
    ax.legend(handles=[Patch(color=c, label=f) for f, c in fam_col.items()], loc="lower right")
    ax.set_xlabel("tickets in October (thousands)")
    ax.grid(axis="y", visible=False)
    ax.grid(axis="x", visible=True)
    dshare = res["unknown_family_share"].get("D- family", 0) + res["unknown_family_share"].get("OMS family", 0)
    titles(
        ax,
        f"Routes missing from the 2025 timetable: D- and OMS families hold {dshare:.0%} of those tickets",
        "Top 20 ticket route names with no timetable match",
    )
    fig_unk = save(fig, "audit_unknown_routes")

    # ---- consistency between raw rows and the model's ticket index ---------------------
    idx_ok = scalar(
        con,
        "select avg((r.route = t.ticket_route)::int) from tix_full t join raw r on r.ticket = t.ticket",
    )
    res["ticket_index_consistent"] = idx_ok
    # Vehicle share among matched tickets, and tickets per rebuilt trip.
    res["matched_vehicle_share"] = scalar(con, "select avg((vehicle <> '')::int) from tix_full")
    tr = q(con, "select count(*) n, avg(tickets) tix, avg(pax) pax from trips_full").row(0, named=True)
    res["recon_trips"] = tr["n"]
    res["tickets_per_recon_trip"] = tr["tix"]
    res["expansion"] = meta["meta"]["expansion"]
    res["ticketed_pax_per_weekday"] = meta["meta"]["ticketed_pax_per_weekday"]

    save_json("audit", res)

    # ---- markdown ---------------------------------------------------------------------
    e = meta["meta"]["expansion"]
    fk_tab = fk.select(
        "bus",
        pl.col("fare_per_pax").alias("fare (Rs/pax)"),
        pl.col("n").alias("tickets"),
        pl.col("p25").alias("p25 km"),
        pl.col("p50").alias("median km"),
        pl.col("p75").alias("p75 km"),
        pl.col("stage_km").alias("assumed stage km"),
        pl.col("share_in_stage").alias("share in stage"),
    ).sort("bus", "fare (Rs/pax)", descending=[True, False])
    md = f"""
# Data audit

Source: `data/raw/tickets/October-Delhi.csv` ({n_rows:,} rows, {comp['pax']:,} passengers), matched by the
pipeline to the July 2025 static timetable (run `full`, Oct 1-31 2026).

## What the file is

- Every row has `ticket_type_id` = {comp['ticket_type']} ({comp['ticket_types']} distinct value), and
  {pct(main_fares)} of tickets have a fare per passenger of Rs 4.5 / 9 / 13.5 / 18 / 22.5, i.e. the
  regular fares with a 10% discount. This is the digital (mobile/QR) ticket channel only, not all riders.
- Ticketed riders per weekday: {num(meta['meta']['ticketed_pax_per_weekday'])} against an assumed city
  bus ridership of 4 million boardings (low 3M, high 5M). Expansion factor: {e['mid']:.1f}
  (low {e['low']:.1f}, high {e['high']:.1f}); about 1 rider in {e['mid']:.0f} is in the file.
- {pct(comp['ac_share'])} of tickets are on AC buses. {comp['routes']:,} distinct route names.
- All timestamps carry +05:30 (share without: {pct(comp['non_ist'], 2)}). `created_at` is when the ticket
  was issued; the model uses it as the boarding time.
- {round(comp['same_stop'] * n_rows):,} tickets have the same boarding and alighting stop name.

## Tickets per day

![tickets per day]({fig_day})

{md_table(by_dt.sort('days', descending=True).select('daytype', 'days', pl.col('tickets_per_day').alias('tickets/day'), pl.col('pax_per_day').alias('passengers/day')), {'tickets/day': '{:,.0f}', 'passengers/day': '{:,.0f}'})}

Weekdays average {num(train_wd)} tickets in Oct 1-24 (training period) and {num(test_wd)} in Oct 25-31
(test week), so the test week is not unusual in volume. Holidays look like Sundays:
{', '.join(f"{d}: {n:,}" for d, n in res['holiday_tickets'].items())}. The lowest ordinary weekdays were
{', '.join(f"{d} ({n:,})" for d, n in res['lowest_weekdays'].items())}; Oct 19 is the eve of the Oct 20 holiday
and is kept as a weekday.

## Tickets by hour

![tickets by hour]({fig_hour})

Weekday purchases peak in the {am['hour']:02d}:00 hour ({num(am['per_day'])} per day) and the
{pm['hour']:02d}:00 hour ({num(pm['per_day'])} per day). {pct(res['out_of_hours_share'], 2)} of tickets are
bought before 04:00, outside the modelled service day.

## Vehicle numbers

![vehicle share]({fig_veh})

Daily share ranges from {pct(vmin)} to {pct(vmax)}. It is flat near half except for a jump on
{', '.join(f"{d} ({pct(v)})" for d, v in res['vehicle_share_high_days'].items())}: something in how vehicle
numbers were captured changed for those days (cause unknown from the data). Rebuilt trips and the capture
correction are pooled over all days of a day type, so the jump is diluted in the supply estimate, but it is
one reason the observed bus counts are not perfectly comparable day to day.

{pct(comp['veh_share'])} of tickets carry a vehicle number (AC {pct(res['vehicle_share_by_ac'].get('AC'))},
non-AC {pct(res['vehicle_share_by_ac'].get('non-AC'))}); {comp['vehicles']:,} distinct vehicles appear in the month,
about {num(wk['vehicles'].mean())} per weekday. The pipeline rebuilds {tr['n']:,} bus trips from them, with
{tr['tix']:.2f} tagged tickets per trip on average, which is too few to read a load off a single trip. Loads
are therefore estimated by pooling running counts over all buses in a 30-minute bin.

## Passengers per ticket

{md_table(pc.select('passenger_count', 'tickets', 'share', 'pax'), {'share': pct, 'pax': '{:,.0f}'})}

## Fares

{md_table(fares, {'fare_per_pax': '{:g}', 'share': pct})}

## Trip length against fare

![fare vs km]({fig_fare})

Only tickets with both stops matched are used (imputed alighting stops take their distance from the fare, so
they would make this check circular). Distances are the model's straight-line distances between consecutive
stops, which run slightly short of road distance.

{md_table(fk_tab, {'fare (Rs/pax)': '{:g}', 'p25 km': '{:.1f}', 'median km': '{:.1f}', 'p75 km': '{:.1f}', 'share in stage': pct})}

Median trip length rises with every fare step for both bus types (monotonic: {'yes' if mono else 'NO'}), and
{pct(res['fare_share_in_stage_overall'])} of matched tickets fall inside the distance stage that their fare implies
(assumed stages: non-AC Rs 5 up to 4 km, Rs 10 for 4-10 km, Rs 15 beyond; AC Rs 10 / 15 / 20 / 25 for
0-4 / 4-8 / 8-12 / 12+ km, full fares before the 10% discount). Stop matching that was wrong would scatter
trip lengths independently of the fare; it does not. The non-AC Rs 13.5 and AC Rs 22.5 bands are open-ended,
which is why their spread is wide.

## Match outcome

![match outcome]({fig_match})

{md_table(out_df, {'share': pct})}

Of the matched tickets, {pct(res['geo_match_share_of_assigned'])} needed the geographic fallback (a same-named
stop within 400 m of a stop on the route).

## Routes missing from the timetable

![unknown routes]({fig_unk})

Ticket route names that never matched (approximation of "route not in timetable": {unk_total:,} tickets):

{md_table(unk.with_columns((pl.col('tickets') / unk_total).alias('share')), {'share': pct})}

The D- and OMS families make up {pct(dshare)} of these tickets; the rest are {res['unknown_family_routes'].get('other', 0)}
other names, led by {', '.join(top.filter(pl.col('family') == 'other')['route'].head(5).to_list())}, which the July 2025
feed does not list under those names. Their riders are not in the model at all:
no crowding estimate exists for those buses, and the corridors they serve look emptier than they are.

## Index check

The model's ticket index matches the CSV row order for {pct(idx_ok, 2)} of matched tickets (route name
compared), so raw rows and model outputs can be joined safely.
"""
    write_md("audit", md)
    return res


if __name__ == "__main__":
    main()
