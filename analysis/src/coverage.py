"""Where the estimates are least reliable: ticket density by place and time, and model-structure
issues that the per-cell confidence label does not capture.

Writes docs/results/coverage.md, docs/results/coverage.json and docs/figures/coverage_*.png.
"""

from __future__ import annotations

import math

import matplotlib.pyplot as plt
import numpy as np
import polars as pl
from matplotlib.colors import BoundaryNorm, ListedColormap

from common import (
    CP_LAT,
    CP_LON,
    SEQ_RAMP,
    TEXT,
    TEXT2,
    SECTORS,
    TIME_BANDS,
    bearing_words,
    band_case_sql,
    connect,
    hhmm,
    load_json,
    md_table,
    num,
    pct,
    q,
    save,
    save_json,
    scalar,
    segments_table,
    setup_style,
    titles,
    write_md,
)

RING_SQL = """case when (from_cp_km + to_cp_km) / 2 < 5 then '0-5 km'
                   when (from_cp_km + to_cp_km) / 2 < 10 then '5-10 km'
                   when (from_cp_km + to_cp_km) / 2 < 20 then '10-20 km' else '20+ km' end"""
SECTOR_SQL = f"""(((degrees(atan2((mid_lon - {CP_LON}) * cos(radians({CP_LAT})), mid_lat - {CP_LAT})) + 360 + 22.5)::int % 360) // 45)"""
GRID_DEG = 0.02  # about 2.2 km north-south, 2.0 km east-west




def main() -> dict:
    setup_style()
    con = connect(["full"])
    meta = load_json("full", "meta.json")
    rep = load_json("full", "match_report.json")
    conf_cfg = meta["config"]["confidence"]
    segments_table(con, "full", "seg", daytype=None)
    res: dict = {"confidence_thresholds": conf_cfg}

    # ---- confidence by day type and time band ------------------------------------------
    by_dt = q(
        con,
        """
        select daytype, sum(buses_per_day * (confidence = 'high')::int) / sum(buses_per_day) high,
               sum(buses_per_day * (confidence = 'medium')::int) / sum(buses_per_day) medium,
               sum(buses_per_day * (confidence = 'low')::int) / sum(buses_per_day) low
        from seg group by 1 order by case daytype when 'weekday' then 0 when 'saturday' then 1 else 2 end
        """,
    )
    res["by_daytype"] = by_dt.to_dicts()
    by_band = q(
        con,
        f"""
        select {band_case_sql()} band, min(bin_minute) m,
               sum(buses_per_day) bus_segments,
               sum(buses_per_day * (confidence = 'high')::int) / sum(buses_per_day) high,
               sum(buses_per_day * (confidence = 'medium')::int) / sum(buses_per_day) medium,
               sum(buses_per_day * (confidence = 'low')::int) / sum(buses_per_day) low,
               sum(tickets) / 20 / sum(buses_per_day) tickets_per_bus_segment
        from seg where daytype = 'weekday' group by 1 order by m
        """,
    ).drop("m")
    res["weekday_by_band"] = by_band.to_dicts()
    wd = by_dt.filter(pl.col("daytype") == "weekday").row(0, named=True)
    res["weekday_reliable_share"] = wd["high"] + wd["medium"]
    bins = q(
        con,
        """
        select bin_minute, sum(buses_per_day) buses, sum(buses_per_day * (confidence = 'low')::int) / sum(buses_per_day) low
        from seg where daytype = 'weekday' group by 1 order by 1
        """,
    )
    busy = bins.filter(pl.col("buses") >= bins["buses"].max() * 0.25)
    worst_bins = busy.sort("low", descending=True).head(5)
    best_bins = busy.sort("low").head(3)
    res["weekday_worst_bins"] = {hhmm(m): l for m, l in zip(worst_bins["bin_minute"], worst_bins["low"])}
    res["weekday_best_bins"] = {hhmm(m): l for m, l in zip(best_bins["bin_minute"], best_bins["low"])}

    # ---- by ring and sector ------------------------------------------------------------
    rs = q(
        con,
        f"""
        select {RING_SQL} ring, {SECTOR_SQL} sector,
               sum(buses_per_day) bus_segments,
               sum(buses_per_day * (confidence <> 'low')::int) / sum(buses_per_day) reliable
        from seg where daytype = 'weekday' group by all
        """,
    )
    ring_tab = q(
        con,
        f"""
        select {RING_SQL} ring, sum(buses_per_day) bus_segments,
               sum(buses_per_day * (confidence <> 'low')::int) / sum(buses_per_day) reliable,
               sum(tickets) / 20 / sum(buses_per_day) tickets_per_bus_segment
        from seg where daytype = 'weekday' group by 1 order by min((from_cp_km + to_cp_km) / 2)
        """,
    )
    sec_tab = q(
        con,
        f"""
        select {SECTOR_SQL} sector, sum(buses_per_day) bus_segments,
               sum(buses_per_day * (confidence <> 'low')::int) / sum(buses_per_day) reliable,
               sum(tickets) / 20 / sum(buses_per_day) tickets_per_bus_segment
        from seg where daytype = 'weekday' group by 1 order by 1
        """,
    ).with_columns(pl.col("sector").map_elements(lambda i: SECTORS[int(i) % 8], return_dtype=pl.Utf8))
    res["reliable_by_ring"] = dict(zip(ring_tab["ring"], ring_tab["reliable"]))
    res["reliable_by_sector"] = dict(zip(sec_tab["sector"], sec_tab["reliable"]))
    best_sec = sec_tab.sort("reliable", descending=True)["sector"].head(3).to_list()
    worst_sec = sec_tab.sort("reliable")["sector"].head(3).to_list()
    res["best_sectors"] = best_sec
    res["worst_sectors"] = worst_sec
    pivot = rs.with_columns(pl.col("sector").map_elements(lambda i: SECTORS[int(i) % 8], return_dtype=pl.Utf8)).pivot(
        on="sector", index="ring", values="reliable"
    )
    order = {"0-5 km": 0, "5-10 km": 1, "10-20 km": 2, "20+ km": 3}
    pivot = pivot.with_columns(pl.col("ring").replace_strict(order, return_dtype=pl.Int64).alias("_o")).sort("_o").drop("_o")
    pivot = pivot.select(["ring"] + [s for s in SECTORS if s in pivot.columns])

    # ---- map -------------------------------------------------------------------------------
    pts = q(
        con,
        """
        select from_stop_id, to_stop_id, min(mid_lat) lat, min(mid_lon) lon, sum(buses_per_day) buses,
               sum(buses_per_day * (confidence <> 'low')::int) / sum(buses_per_day) reliable
        from seg where daytype = 'weekday' group by 1, 2
        """,
    ).sort("reliable")
    res["map_points"] = pts.height
    fig, ax = plt.subplots(figsize=(9, 9.6))
    bounds = [0, 0.2, 0.4, 0.6, 0.8, 1.0001]
    cmap = ListedColormap([SEQ_RAMP[0], SEQ_RAMP[2], SEQ_RAMP[3], SEQ_RAMP[5], SEQ_RAMP[6]])
    norm = BoundaryNorm(bounds, cmap.N)
    sc = ax.scatter(pts["lon"], pts["lat"], c=pts["reliable"], cmap=cmap, norm=norm, s=3, linewidths=0)
    ax.plot(CP_LON, CP_LAT, marker="+", color=TEXT, markersize=10, mew=1.5)
    ax.annotate("Connaught Place", (CP_LON, CP_LAT), xytext=(6, 6), textcoords="offset points", fontsize=8, color=TEXT)
    ax.set_aspect(1 / math.cos(math.radians(CP_LAT)))
    lat_lo, lat_hi = np.quantile(pts["lat"], [0.002, 0.998])
    lon_lo, lon_hi = np.quantile(pts["lon"], [0.002, 0.998])
    ax.set_xlim(lon_lo - 0.02, lon_hi + 0.02)
    ax.set_ylim(lat_lo - 0.02, lat_hi + 0.02)
    ax.set_xlabel("longitude (degrees E)")
    ax.set_ylabel("latitude (degrees N)")
    ax.grid(True, axis="both")
    cb = fig.colorbar(sc, ax=ax, fraction=0.035, pad=0.02, ticks=[0, 0.2, 0.4, 0.6, 0.8, 1.0])
    cb.ax.set_yticklabels(["0%", "20%", "40%", "60%", "80%", "100%"])
    cb.set_label("weekday bus-segments with medium or high confidence", color=TEXT2)
    cb.outline.set_visible(False)
    titles(
        ax,
        f"Only {res['weekday_reliable_share']:.0%} of weekday bus-segments rest on enough tickets; "
        f"best covered: {', '.join(best_sec)} of the centre",
        f"Each dot = one stop-to-stop link (midpoint), all routes and weekday bins; confidence: medium >= "
        f"{conf_cfg['medium_min_tickets']:g}, high >= {conf_cfg['high_min_tickets']:g} kernel-weighted tickets",
    )
    fig_map = save(fig, "coverage_map")

    # ---- least reliable areas: 2 km grid cells with much bus service and few tickets ------
    con.sql(
        f"""
        create table cells as
        select floor(mid_lat / {GRID_DEG})::int gy, floor(mid_lon / {GRID_DEG})::int gx,
               sum(buses_per_day) bus_segments,
               sum(buses_per_day * (confidence <> 'low')::int) / sum(buses_per_day) reliable,
               sum(tickets) / 20 / sum(buses_per_day) tickets_per_bus_segment,
               avg(mid_lat) lat, avg(mid_lon) lon,
               count(distinct route_id) routes
        from seg where daytype = 'weekday' group by 1, 2
        """
    )
    p70 = scalar(con, "select quantile_cont(bus_segments, 0.7) from cells")
    weak = q(
        con,
        f"""
        with w as (select * from cells where bus_segments >= {p70} order by reliable, bus_segments desc limit 12),
             agg as (
                select floor(mid_lat / {GRID_DEG})::int gy, floor(mid_lon / {GRID_DEG})::int gx, from_stop_name,
                       sum(buses_per_day) b
                from seg where daytype = 'weekday' group by all),
             names as (
                select gy, gx, from_stop_name, row_number() over (partition by gy, gx order by b desc, from_stop_name) rk
                from agg)
        select n.from_stop_name area_stop, w.lat, w.lon, w.routes, w.bus_segments, w.reliable, w.tickets_per_bus_segment
        from w join names n using (gy, gx) where n.rk = 1 order by w.reliable, w.bus_segments desc
        """,
    )
    weak = weak.with_columns(
        pl.struct("lat", "lon").map_elements(lambda r: bearing_words(r["lat"], r["lon"]), return_dtype=pl.Utf8).alias("where")
    )
    res["weak_areas"] = weak.select("area_stop", "where", "reliable").to_dicts()
    res["grid_cells_busy_threshold_bus_segments"] = p70
    res["busy_cells_with_no_reliable_segment"] = scalar(con, f"select count(*) from cells where bus_segments >= {p70} and reliable = 0")
    res["busy_cells"] = scalar(con, f"select count(*) from cells where bus_segments >= {p70}")

    # ---- structural reliability issues ----------------------------------------------------
    # 1. Route names shared by two route_ids (two operators): split tickets, double-counted supply.
    dup = q(
        con,
        """
        with d as (select route_name from (select route_name, count(distinct route_id) n from seg group by 1) where n > 1)
        select count(distinct s.route_name) names, count(distinct s.route_id) route_ids,
               sum(s.buses_per_day) filter (where s.daytype = 'weekday') /
                 (select sum(buses_per_day) from seg where daytype = 'weekday') bus_share
        from seg s where s.route_name in (select route_name from d)
        """,
    ).row(0, named=True)
    twins = q(
        con,
        """
        with d as (select route_name from (select route_name, count(distinct route_id) n from trips_full group by 1) where n > 1),
             t as (select * from trips_full where route_name in (select route_name from d)),
             p as (select x.vehicle, x.date, x.route_name, x.first_secs, x.route_id,
                          (select min(abs(y.first_secs::int - x.first_secs::int)) from t y
                           where y.vehicle = x.vehicle and y.date = x.date and y.route_name = x.route_name
                             and y.route_id <> x.route_id) gap
                   from t x)
        select (select count(*) from t) trips_dup_names,
               (select count(*) from p where gap <= 3600) trips_with_twin,
               (select count(*) from p where gap <= 1800) trips_with_twin_30min,
               (select count(*) from trips_full) trips_all
        """,
    ).row(0, named=True)
    dup_ag = q(
        con,
        """select count(*) names, sum((a > 1)::int) multi_agency
           from (select route_long_name, count(*) c, count(distinct agency_id) a from gtfs_routes group by 1) where c > 1""",
    ).row(0, named=True)
    res["duplicate_names"] = dup | twins | {"timetable_names": dup_ag["names"], "timetable_multi_agency": dup_ag["multi_agency"]}
    res["twin_share_of_dup_name_trips"] = twins["trips_with_twin"] / twins["trips_dup_names"] if twins["trips_dup_names"] else 0.0

    # 2. Capture correction at its cap (trips not identified).
    cap = meta["config"]["capture_cap"]
    capped = q(
        con,
        f"""
        select count(*) routes, sum((weekday_inverse_capture >= {cap} - 0.01)::int) at_cap
        from routes_full
        """,
    ).row(0, named=True)
    cap_bus = scalar(
        con,
        f"""
        select sum(s.buses_per_day) / (select sum(buses_per_day) from seg where daytype = 'weekday')
        from seg s join routes_full r using (route_id)
        where s.daytype = 'weekday' and r.weekday_inverse_capture >= {cap} - 0.01
        """,
    )
    res["capture_at_cap_routes"] = capped["at_cap"]
    res["capture_at_cap_bus_share"] = cap_bus

    # 3. Supply from tickets vs timetable, and loads above crush.
    sup = q(
        con,
        """
        select sum(buses_per_day * (observed_buses_per_day > sched_buses_per_day)::int) / sum(buses_per_day) obs_used,
               sum(buses_per_day * (sched_buses_per_day = 0)::int) / sum(buses_per_day) no_timetable,
               sum(sched_buses_per_day) / sum(buses_per_day) sched_ratio
        from seg where daytype = 'weekday'
        """,
    ).row(0, named=True)
    crush = q(
        con,
        """
        with c as (select *, case bus_class when '12m-nonac' then 90 when '9m-ev' then 55 else 85 end crush
                   from seg where daytype = 'weekday')
        select count(*) filter (where load > crush) cells,
               count(*) filter (where load > crush and confidence = 'high') high_conf,
               count(*) filter (where load > crush and sched_buses_per_day = 0) no_timetable,
               count(distinct route_id) filter (where load > crush and confidence = 'high') routes_high
        from c
        """,
    ).row(0, named=True)
    res["supply"] = sup
    res["over_crush"] = crush
    rt = meta.get("rt")
    res["rt"] = rt

    # 4. Riders the model cannot place at all.
    res["route_not_in_timetable_share"] = rep["rejected"].get("RouteNotInGtfs", 0) / rep["tickets"]
    res["stop_not_found_share"] = rep["rejected"].get("StartStopNotOnRoute", 0) / rep["tickets"]
    res["imputed_share"] = rep["by_method"].get("Imputed", 0) / rep["tickets"]
    res["patterns_with_data"] = meta["patterns_with_data"]
    res["patterns_total"] = meta["patterns_total"]

    save_json("coverage", res)

    # ---- markdown ---------------------------------------------------------------------------
    sat = by_dt.filter(pl.col("daytype") == "saturday").row(0, named=True)
    sun = by_dt.filter(pl.col("daytype") == "sunday").row(0, named=True)
    if dup_ag["names"] == 0:
        dup_text = (
            "No route name appears twice in this timetable, so a bus cannot be matched to two copies of its "
            "route. Each route's full and short-turn patterns share stops and are merged onto the pattern "
            f"that covers them ({meta['route_groups'].get('patterns_merged', 0)} patterns merged)."
        )
    else:
        dup_text = (
            f"In the timetable, {dup_ag['names']} route names exist more than once ({dup_ag['multi_agency']} with two "
            f"operators). {dup['names']} such names carry tickets, covering {pct(dup['bus_share'])} of weekday "
            "bus-segments; their patterns are merged when their stops line up."
        )
    if rt:
        lo, mid, hi = rt["route_factor_p10_p50_p90"]
        supply_text = (
            f"Buses come from the timetable scaled to the live feed ({rt['snapshots']} snapshots, {rt['first']} to "
            f"{rt['last']}). The feed is from {', '.join(rt['dates'])}, the tickets from October: this assumes the same "
            "service. Hours without snapshots use the average factor of their day type. Route factors (buses "
            f"running against the city-wide expectation) span {lo:.2f}-{hi:.2f} (10th-90th percentile, median {mid:.2f}), "
            f"and {pct(rt['placed_on_route_share'])} of buses with a known route were within 500 m of it."
        )
    else:
        supply_text = "No live feed snapshots were given: buses come from the timetable as published."
    md = f"""
# Coverage: where estimates are least reliable

A cell's confidence is set by the kernel-weighted number of ticketed riders behind it: high >=
{conf_cfg['high_min_tickets']:g}, medium >= {conf_cfg['medium_min_tickets']:g}, otherwise low (summed over all days of
the day type in October). Low-confidence cells are mostly the prior: the route's level and the city-wide
time-of-day shape. Shares below are of bus-segments (each bus passing each segment once).

## By day type

{md_table(by_dt, {"high": pct, "medium": pct, "low": pct})}

Weekends rest on {meta['meta']['n_days'][1]} Saturdays and {meta['meta']['n_days'][2]} Sundays/holidays and far fewer
tickets per day, so {pct(sat['low'])} of Saturday and {pct(sun['low'])} of Sunday bus-segments are low confidence.
Weekend crowding should be shown as indicative only.

## Weekday, by time band

{md_table(by_band, {"bus_segments": "{:,.0f}", "high": pct, "medium": pct, "low": pct, "tickets_per_bus_segment": "{:.3f}"})}

`tickets_per_bus_segment` = ticketed riders crossing per bus pass per weekday. Among bins with at least a
quarter of the peak bus count, the least covered are {', '.join(f"{k} ({pct(v)} low)" for k, v in res['weekday_worst_bins'].items())};
the best covered are {', '.join(f"{k} ({pct(v)} low)" for k, v in res['weekday_best_bins'].items())}. Early morning,
midday and late evening estimates lean on the prior; the peaks are where the data is.

## Where

![coverage map]({fig_map})

Share of weekday bus-segments with medium or high confidence, by distance and direction from Connaught Place:

{md_table(ring_tab, {"bus_segments": "{:,.0f}", "reliable": pct, "tickets_per_bus_segment": "{:.3f}"})}

{md_table(sec_tab, {"bus_segments": "{:,.0f}", "reliable": pct, "tickets_per_bus_segment": "{:.3f}"})}

{md_table(pivot, {c: pct for c in pivot.columns if c != "ring"})}

The {', '.join(best_sec)} sectors are best covered; the {', '.join(worst_sec)} sectors are worst. Coverage also thins
beyond 20 km ({pct(res['reliable_by_ring'].get('20+ km'))} reliable) and in the central 5 km
({pct(res['reliable_by_ring'].get('0-5 km'))}), where ticketed riders per bus pass are also lower
({ring_tab.filter(pl.col('ring') == '0-5 km')['tickets_per_bus_segment'][0]:.3f} against
{ring_tab['tickets_per_bus_segment'].max():.3f} in the best ring). Confidence is per route, so a central corridor
served by several lightly ticketed routes can be busy overall while each route's cells stay low confidence.

### Least reliable busy areas

2 km grid cells in the top 30% by weekday bus-segments, ranked by the share of their bus-segments with
medium or high confidence (lowest first). {res['busy_cells_with_no_reliable_segment']} of {res['busy_cells']} busy grid cells
have no medium- or high-confidence bus-segment at all.

{md_table(weak.select("area_stop", "where", "routes", "bus_segments", "reliable", "tickets_per_bus_segment"), {"bus_segments": "{:,.0f}", "reliable": pct, "tickets_per_bus_segment": "{:.3f}"})}

In words: the weakest areas are places with plenty of buses but few app tickets, named here by their
busiest stop: {'; '.join(f"{r['area_stop']} ({r['where']})" for r in res['weak_areas'][:8])}.

## Problems the confidence label does not capture

1. **Riders the model cannot place.** {pct(res['route_not_in_timetable_share'])} of tickets are on routes missing
   from the timetable and {pct(res['stop_not_found_share'])} have a boarding stop not found on their route.
   Those buses have no estimate at all, and corridors shared with them look emptier than they are. Another
   {pct(res['imputed_share'])} of tickets have an imputed alighting stop. {res['patterns_total'] - res['patterns_with_data']:,} of
   {res['patterns_total']:,} timetable patterns have no matched ticket.
2. **Duplicate route names.** {dup_text}
3. **Bus counts.** {supply_text}
4. **Tickets-only bus counts are not used.** On {res['capture_at_cap_routes']} routes nearly every bus trip rebuilt
   from tickets carries a single tagged ticket, so the number of trips the tickets miss cannot be estimated from
   the tickets themselves. This is why the bus count now comes from the timetable and the live feed; the
   tickets-only count is kept as the `sens_tickets` sensitivity run.
5. **Implausible loads with high confidence.** {crush['cells']:,} weekday cells exceed crush capacity,
   {crush['high_conf']:,} of them "high confidence" (on {crush['routes_high']} routes). These are capped at crush and
   flagged. Confidence counts tickets, not the reliability of the bus count they are divided by.
"""
    write_md("coverage", md)
    return res


if __name__ == "__main__":
    main()
