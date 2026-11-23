"""Known-pattern checks on the full-month model: time of day, day type, direction relative
to Connaught Place, busiest corridors and stops, and crowding in the peaks.

Writes docs/results/patterns.md, docs/results/patterns.json and docs/figures/patterns_*.png.
"""

from __future__ import annotations

import matplotlib.pyplot as plt
import numpy as np
import polars as pl
from matplotlib.patches import Patch

from common import (
    AM_PEAK,
    BLUE,
    DAYTYPE_COLORS,
    GREY,
    GRID,
    LEVEL_COLORS,
    LEVELS,
    ORANGE,
    OUT,
    PM_PEAK,
    SEQ_RAMP,
    TEXT2,
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
    time_axis,
    titles,
    write_md,
    bearing_words,
    sector_of,
)

HUB_WORDS = ["ISBT", "Metro", "Terminal", "Depot", "Station", "Railway", "Rly"]


def in_window(m: int, w: tuple[int, int]) -> bool:
    return w[0] <= m < w[1]


def main() -> dict:
    setup_style()
    con = connect(["full"])
    meta = load_json("full", "meta.json")
    e_mid = meta["meta"]["expansion"]["mid"]
    n_days = dict(zip(["weekday", "saturday", "sunday"], meta["meta"]["n_days"]))
    segments_table(con, "full", "seg", daytype=None)
    res: dict = {}

    # ---- time-of-day profiles -----------------------------------------------------------
    prof = q(
        con,
        """
        select daytype, bin_minute, sum(buses_per_day) buses,
               sum(buses_per_day * load) / sum(buses_per_day) avg_load,
               sum(buses_per_day * load_low) / sum(buses_per_day) avg_low,
               sum(buses_per_day * load_high) / sum(buses_per_day) avg_high,
               sum(buses_per_day * (level = 'standing')::int) / sum(buses_per_day) sh_standing,
               sum(buses_per_day * (level = 'crowded')::int) / sum(buses_per_day) sh_crowded,
               sum(buses_per_day * p_standing) / sum(buses_per_day) p_standing,
               sum(buses_per_day * p_crowded) / sum(buses_per_day) p_crowded,
               sum(tickets) tickets
        from seg group by all order by all
        """,
    )
    nd = pl.DataFrame({"daytype": list(n_days), "n_days": list(n_days.values())})
    prof = prof.join(nd, on="daytype").with_columns(
        (pl.col("tickets") / (pl.col("n_days") * pl.col("buses")) * e_mid).alias("raw_load")
    )
    wd = prof.filter(pl.col("daytype") == "weekday").sort("bin_minute")
    am = wd.filter(pl.col("bin_minute") < 13 * 60).sort("avg_load", descending=True).row(0, named=True)
    pm = wd.filter(pl.col("bin_minute") >= 13 * 60).sort("avg_load", descending=True).row(0, named=True)
    trough = wd.filter((pl.col("bin_minute") >= 11 * 60) & (pl.col("bin_minute") < 17 * 60)).sort("avg_load").row(0, named=True)
    am_raw = wd.filter(pl.col("bin_minute") < 13 * 60).sort("raw_load", descending=True).row(0, named=True)
    pm_raw = wd.filter(pl.col("bin_minute") >= 13 * 60).sort("raw_load", descending=True).row(0, named=True)
    n_adj = scalar(con, "select count(*) from routes_full where expansion_adjustment < 1")
    nocap_am = scalar(
        con,
        f"""select sum(buses_per_day * load) / sum(buses_per_day)
            from read_parquet('{OUT / 'sens_nocap' / 'segment_profiles.parquet'}')
            where daytype = 'weekday' and bin_minute = {am['bin_minute']}""",
    )
    res["capacity_adjusted_routes"] = n_adj
    res["nocap_am_peak_load"] = nocap_am
    res["raw_am_peak_load"] = am_raw["raw_load"]
    am_ok = 7 * 60 + 30 <= am["bin_minute"] < 10 * 60 + 30
    pm_ok = 17 * 60 <= pm["bin_minute"] < 20 * 60 + 30
    res["weekday_am_peak_bin"] = hhmm(am["bin_minute"])
    res["weekday_pm_peak_bin"] = hhmm(pm["bin_minute"])
    res["weekday_am_peak_load"] = am["avg_load"]
    res["weekday_pm_peak_load"] = pm["avg_load"]
    res["weekday_midday_trough_bin"] = hhmm(trough["bin_minute"])
    res["weekday_midday_trough_load"] = trough["avg_load"]
    res["weekday_am_peak_share_standing_or_crowded"] = am["sh_standing"] + am["sh_crowded"]
    res["weekday_pm_peak_share_standing_or_crowded"] = pm["sh_standing"] + pm["sh_crowded"]
    res["raw_am_peak_bin"] = hhmm(am_raw["bin_minute"])
    res["raw_pm_peak_bin"] = hhmm(pm_raw["bin_minute"])
    res["peaks_as_expected"] = bool(am_ok and pm_ok)

    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(10, 7), sharex=True, gridspec_kw={"height_ratios": [3, 2]})
    x = wd["bin_minute"].to_numpy() + 15
    ax1.fill_between(x, wd["avg_low"], wd["avg_high"], color=SEQ_RAMP[0], lw=0, label="low-high ridership scenario")
    ax1.plot(x, wd["avg_load"], color=BLUE, label="model estimate (mid scenario)")
    ax1.plot(x, wd["raw_load"], color=GREY, lw=1.2, label="raw pooled count x expansion (no shrinkage, no cap)")
    for r in (am, pm):
        top = max(r["avg_high"], r["raw_load"])
        ax1.annotate(
            f"model {hhmm(r['bin_minute'])}: {r['avg_load']:.1f}",
            (r["bin_minute"] + 15, top),
            xytext=(0, 6), textcoords="offset points", ha="center", fontsize=9, color=TEXT2,
        )
    ax1.set_ylabel("riders on board per bus\n(bus-weighted average)")
    ax1.set_ylim(0, max(wd["avg_high"].max(), wd["raw_load"].max()) * 1.15)
    ax1.legend(loc="upper right")
    titles(
        ax1,
        f"Weekday loads peak at {hhmm(am['bin_minute'])} and {hhmm(pm['bin_minute'])}"
        + (" - the expected commuting peaks" if am_ok and pm_ok else " - not where commuting peaks are expected"),
        "Average expected load per bus over all route segments, weighted by buses per day",
    )
    ax2.plot(x, wd["sh_standing"] * 100, color=LEVEL_COLORS["standing"], label="standing room")
    ax2.plot(x, wd["sh_crowded"] * 100, color=LEVEL_COLORS["crowded"], label="crowded")
    ax2.set_ylabel("bus-segments at level (%)")
    ax2.legend(loc="upper right")
    time_axis(ax2)
    fig.align_ylabels([ax1, ax2])
    fig_wd = save(fig, "patterns_weekday_profile")

    # ---- day types ----------------------------------------------------------------------
    dtm = q(
        con,
        """
        select daytype, sum(buses_per_day * load) / sum(buses_per_day) mean_load,
               sum(buses_per_day * (level <> 'seats')::int) / sum(buses_per_day) sh_not_seats,
               sum(buses_per_day) bus_segments
        from seg group by 1
        """,
    )
    dmean = {r["daytype"]: r for r in dtm.iter_rows(named=True)}
    peak_by_dt = prof.group_by("daytype").agg(
        pl.col("avg_load").max().alias("peak_load"),
        pl.col("bin_minute").sort_by("avg_load", descending=True).first().alias("peak_bin"),
    )
    pk = {r["daytype"]: r for r in peak_by_dt.iter_rows(named=True)}
    res["daytype_mean_load"] = {k: v["mean_load"] for k, v in dmean.items()}
    res["daytype_share_not_seats"] = {k: v["sh_not_seats"] for k, v in dmean.items()}
    res["daytype_peak_load"] = {k: v["peak_load"] for k, v in pk.items()}
    res["daytype_peak_bin"] = {k: hhmm(v["peak_bin"]) for k, v in pk.items()}
    sat_r = dmean["saturday"]["mean_load"] / dmean["weekday"]["mean_load"]
    sun_r = dmean["sunday"]["mean_load"] / dmean["weekday"]["mean_load"]
    res["saturday_vs_weekday_mean_load"] = sat_r
    res["sunday_vs_weekday_mean_load"] = sun_r
    res["daytype_bus_segments"] = {k: v["bus_segments"] for k, v in dmean.items()}
    sat_sup = dmean["saturday"]["bus_segments"] / dmean["weekday"]["bus_segments"]
    sun_sup = dmean["sunday"]["bus_segments"] / dmean["weekday"]["bus_segments"]

    fig, ax = plt.subplots(figsize=(10, 4.4))
    for k, lab in [("weekday", "weekday"), ("saturday", "Saturday"), ("sunday", "Sunday or holiday")]:
        s = prof.filter(pl.col("daytype") == k).sort("bin_minute")
        ax.plot(s["bin_minute"] + 15, s["avg_load"], color=DAYTYPE_COLORS[k], label=lab)
    ax.set_ylabel("riders on board per bus\n(bus-weighted average)")
    ax.set_ylim(0, None)
    ax.legend(loc="upper right")
    time_axis(ax)
    titles(
        ax,
        f"Saturdays carry {sat_r:.0%} and Sundays {sun_r:.0%} of the weekday load per bus",
        "Average expected load per bus by day type (mid scenario)",
    )
    fig_dt = save(fig, "patterns_daytypes")

    # ---- direction relative to Connaught Place ------------------------------------------
    dir_bins = q(
        con,
        """
        select direction, bin_minute,
               sum(buses_per_day * load) / sum(buses_per_day) avg_load,
               sum(buses_per_day * load * seg_km) pkm, sum(buses_per_day * seg_km) bkm
        from seg where daytype = 'weekday' and to_cp_km <> from_cp_km
        group by all order by all
        """,
    )

    def dir_window(where: str) -> dict:
        d = q(
            con,
            f"""
            select direction, sum(buses_per_day * load) / sum(buses_per_day) avg_load,
                   sum(buses_per_day * load * seg_km) pkm, sum(buses_per_day * seg_km) bkm
            from seg where daytype = 'weekday' and to_cp_km <> from_cp_km and {where}
            group by 1
            """,
        )
        m = {r["direction"]: r for r in d.iter_rows(named=True)}
        return {
            "inbound_load": m["inbound"]["avg_load"],
            "outbound_load": m["outbound"]["avg_load"],
            "load_ratio_in_over_out": m["inbound"]["avg_load"] / m["outbound"]["avg_load"],
            "pkm_ratio_in_over_out": m["inbound"]["pkm"] / m["outbound"]["pkm"],
        }

    amw = f"bin_minute >= {AM_PEAK[0]} and bin_minute < {AM_PEAK[1]}"
    pmw = f"bin_minute >= {PM_PEAK[0]} and bin_minute < {PM_PEAK[1]}"
    radial = "abs(to_cp_km - from_cp_km) >= 0.5 * seg_km"
    direction = {
        "all segments": {"AM": dir_window(amw), "PM": dir_window(pmw), "all day": dir_window("true")},
        "mostly radial segments": {"AM": dir_window(f"{amw} and {radial}"), "PM": dir_window(f"{pmw} and {radial}")},
    }
    rings = [("0-5 km", 0, 5), ("5-10 km", 5, 10), ("10-20 km", 10, 20), ("20+ km", 20, 999)]
    ring_rows = []
    for lab, a, b in rings:
        ring = f"(from_cp_km + to_cp_km) / 2 >= {a} and (from_cp_km + to_cp_km) / 2 < {b}"
        dA = dir_window(f"{amw} and {ring}")
        dP = dir_window(f"{pmw} and {ring}")
        ring_rows.append(
            {
                "distance from CP": lab,
                "AM in/out load": dA["load_ratio_in_over_out"],
                "AM in/out pass-km": dA["pkm_ratio_in_over_out"],
                "PM in/out load": dP["load_ratio_in_over_out"],
                "PM in/out pass-km": dP["pkm_ratio_in_over_out"],
            }
        )
    ring_df = pl.DataFrame(ring_rows)
    rings_ok = sum(1 for r in ring_rows if r["AM in/out load"] > 1 and r["PM in/out load"] < 1)
    best_ring = max(ring_rows, key=lambda r: r["AM in/out load"])["distance from CP"]
    worst_ring = min(ring_rows, key=lambda r: r["AM in/out load"])["distance from CP"]
    res["direction"] = direction
    res["direction_by_ring"] = ring_rows
    am_r = direction["all segments"]["AM"]["load_ratio_in_over_out"]
    pm_r = direction["all segments"]["PM"]["load_ratio_in_over_out"]
    dir_ok = am_r > 1 and pm_r < 1
    res["direction_as_expected"] = bool(dir_ok)

    fig, ax = plt.subplots(figsize=(10, 4.4))
    for w in (AM_PEAK, PM_PEAK):
        ax.axvspan(w[0], w[1], color=GRID, lw=0, zorder=0)
    for dname, col, lab in [("inbound", BLUE, "toward Connaught Place"), ("outbound", ORANGE, "away from Connaught Place")]:
        s = dir_bins.filter(pl.col("direction") == dname).sort("bin_minute")
        ax.plot(s["bin_minute"] + 15, s["avg_load"], color=col, label=lab)
    ax.set_ylabel("riders on board per bus\n(bus-weighted average)")
    ax.set_ylim(0, None)
    ax.legend(loc="upper right")
    time_axis(ax)
    if dir_ok:
        ttl = f"Toward the centre is heavier in the morning peak ({am_r:.2f}x) and away from it in the evening ({1 / pm_r:.2f}x)"
    else:
        ttl = f"Direction pattern is not the textbook one: AM in/out ratio {am_r:.2f}, PM {pm_r:.2f}"
    titles(ax, ttl, "Weekday segments classed by whether they move toward Connaught Place; grey bands = peak windows")
    fig_dir = save(fig, "patterns_direction")

    # ---- busiest corridors and stops ---------------------------------------------------
    links = q(
        con,
        """
        with b as (
            select from_stop_name, to_stop_name, bin_minute,
                   sum(buses_per_day * load) pax, sum(buses_per_day) buses,
                   count(distinct route_id) routes
            from seg where daytype = 'weekday' group by all
        )
        select from_stop_name, to_stop_name, max(routes) routes,
               sum(buses) buses_per_day, sum(pax) passengers_per_day,
               sum(pax) / sum(buses) avg_load, max(pax / buses) peak_bin_load
        from b group by all order by passengers_per_day desc limit 15
        """,
    )
    route_seg = q(
        con,
        """
        select route_id, route_name, seq, from_stop_name, to_stop_name,
               sum(buses_per_day * load) passengers_per_day, sum(buses_per_day) buses_per_day
        from seg where daytype = 'weekday' group by all order by passengers_per_day desc limit 15
        """,
    )
    top_seg_routes = route_seg["route_name"].unique().to_list()
    res["top15_route_segments_routes"] = top_seg_routes
    peak_routes = q(
        con,
        """
        with c as (
            select s.route_id, s.route_name, s.from_stop_name, s.to_stop_name, s.bin_start, s.load, s.level,
                   s.buses_per_day, s.tickets, s.confidence, s.bus_class,
                   row_number() over (partition by s.route_id order by s.load desc) rk
            from seg s
            where s.daytype = 'weekday' and s.buses_per_day >= 1 and s.confidence <> 'low'
        )
        select c.route_name route, r.agency_id agency, from_stop_name || ' -> ' || to_stop_name segment, bin_start time,
               load, level, buses_per_day buses_in_bin, tickets, confidence
        from c join gtfs_routes r using (route_id)
        where rk = 1 order by load desc limit 15
        """,
    )
    stops = q(
        con,
        """
        select stop_name, count(distinct route_id) routes, sum(boardings_per_day) boardings_per_day,
               sum(ticketed_boardings_per_day) ticketed_per_day, avg(lat) lat, avg(lon) lon
        from act_full where daytype = 'weekday'
        group by stop_name, round(lat * 50), round(lon * 50)
        order by boardings_per_day desc limit 15
        """,
    )
    stops = stops.with_columns(
        pl.struct("lat", "lon").map_elements(lambda r: bearing_words(r["lat"], r["lon"]), return_dtype=pl.Utf8).alias("where"),
        pl.struct("lat", "lon").map_elements(lambda r: sector_of(r["lat"], r["lon"]), return_dtype=pl.Utf8).alias("sector"),
    ).drop("lat", "lon")
    res["top15_stops_by_sector"] = stops.group_by("sector").len().sort("len", descending=True).to_dicts()
    res["top15_stops_south"] = int(stops.filter(pl.col("sector").is_in(["SE", "S", "SW"])).height)
    hub = stops.with_columns(
        pl.col("stop_name").map_elements(lambda s: int(sum(w.lower() in s.lower() for w in HUB_WORDS) > 0), return_dtype=pl.Int64).alias("hub")
    )
    res["top15_stops_with_hub_word"] = int(hub["hub"].sum())
    res["top_stops"] = stops["stop_name"].to_list()
    res["top_links"] = [f"{a} -> {b}" for a, b in zip(links["from_stop_name"], links["to_stop_name"])]
    res["peak_routes"] = peak_routes["route"].to_list()

    # ---- crowding in the peaks, by bus-km --------------------------------------------------
    bands = {
        "all day": "true",
        "AM peak 08:00-10:00": amw,
        "PM peak 17:30-20:00": pmw,
        "outside peaks": f"not ({amw}) and not ({pmw})",
    }
    band_rows = []
    for lab, cond in bands.items():
        r = q(
            con,
            f"""
            select sum(buses_per_day * seg_km) bus_km,
                   sum(buses_per_day * seg_km * (level = 'seats')::int) / sum(buses_per_day * seg_km) seats,
                   sum(buses_per_day * seg_km * (level = 'standing')::int) / sum(buses_per_day * seg_km) standing,
                   sum(buses_per_day * seg_km * (level = 'crowded')::int) / sum(buses_per_day * seg_km) crowded,
                   sum(buses_per_day * seg_km * p_standing) / sum(buses_per_day * seg_km) exp_standing,
                   sum(buses_per_day * seg_km * p_crowded) / sum(buses_per_day * seg_km) exp_crowded
            from seg where daytype = 'weekday' and {cond}
            """,
        ).row(0, named=True)
        band_rows.append({"window": lab, **r})
    band_df = pl.DataFrame(band_rows)
    res["bus_km_levels"] = band_rows
    peak_cond = f"(({amw}) or ({pmw}))"
    pk_all = q(
        con,
        f"""
        select sum(buses_per_day * seg_km * (level = 'seats')::int) / sum(buses_per_day * seg_km) seats,
               sum(buses_per_day * seg_km * (level = 'standing')::int) / sum(buses_per_day * seg_km) standing,
               sum(buses_per_day * seg_km * (level = 'crowded')::int) / sum(buses_per_day * seg_km) crowded,
               sum(buses_per_day * seg_km * p_standing) / sum(buses_per_day * seg_km) exp_standing,
               sum(buses_per_day * seg_km * p_crowded) / sum(buses_per_day * seg_km) exp_crowded
        from seg where daytype = 'weekday' and {peak_cond}
        """,
    ).row(0, named=True)
    res["peak_bus_km_levels"] = pk_all

    fig, ax = plt.subplots(figsize=(10, 3.6))
    ylabels = band_df["window"].to_list()[::-1]
    left = np.zeros(len(ylabels))
    for lv in LEVELS:
        vals = (band_df[lv].to_numpy()[::-1]) * 100
        ax.barh(ylabels, vals, left=left, color=LEVEL_COLORS[lv], height=0.6, edgecolor="white", linewidth=1)
        left += vals
    for i, (st, cr) in enumerate(zip(band_df["standing"].to_list()[::-1], band_df["crowded"].to_list()[::-1])):
        ax.text(101, i, f"{(st + cr) * 100:.1f}% standing or crowded", va="center", fontsize=9, color=TEXT2)
    ax.set_xlim(0, 100)
    ax.set_xlabel("share of weekday bus-km (%)")
    ax.grid(axis="y", visible=False)
    ax.grid(axis="x", visible=True)
    ax.legend(handles=[Patch(color=LEVEL_COLORS[lv], label=lv) for lv in LEVELS], loc="upper left", bbox_to_anchor=(0, -0.2), ncols=3)
    titles(
        ax,
        f"In the weekday peaks {pk_all['standing'] + pk_all['crowded']:.0%} of bus-km run standing or crowded",
        "Level shown to riders (the median bus) per segment and bin, weighted by buses x segment km",
    )
    fig_bands = save(fig, "patterns_level_by_band")

    # ---- scale and physical sanity -----------------------------------------------------
    boardings = scalar(con, "select sum(boardings_per_day) from act_full where daytype = 'weekday'")
    tot = q(
        con,
        """
        select sum(buses_per_day * seg_km) bus_km, sum(buses_per_day * seg_km * load) pkm,
               sum(buses_per_day * (load > case bus_class when '12m-nonac' then 90 when '9m-ev' then 55 else 85 end)::int)
                 / sum(buses_per_day) over_crush,
               sum(buses_per_day * (load > comfortable)::int) / sum(buses_per_day) over_comfortable,
               max(load) max_load
        from seg where daytype = 'weekday'
        """,
    ).row(0, named=True)
    res["model_weekday_boardings"] = boardings
    res["weekday_bus_km_straight_line"] = tot["bus_km"]
    res["weekday_passenger_km"] = tot["pkm"]
    res["weekday_load_per_bus_km"] = tot["pkm"] / tot["bus_km"]
    res["weekday_share_over_crush"] = tot["over_crush"]
    res["weekday_max_load"] = tot["max_load"]
    res["matched_share"] = meta["meta"]["tickets_assigned"] / meta["meta"]["tickets_total"]

    dup_names = set(
        q(
            con,
            "select route_name from (select route_name, count(distinct route_id) n from seg group by 1) where n > 1",
        )["route_name"].to_list()
    )
    table_names = set(route_seg["route_name"].to_list()) | set(peak_routes["route"].to_list())
    dup_in_tables = sorted(table_names & dup_names)
    res["duplicate_route_names_in_tables"] = dup_in_tables
    oc = peak_routes.filter(pl.col("load") > 90)
    res["peak_routes_over_crush"] = int(oc.height)
    oc_bmin = oc["buses_in_bin"].min() if oc.height else float("nan")
    oc_bmax = oc["buses_in_bin"].max() if oc.height else float("nan")

    save_json("patterns", res)

    # ---- markdown ------------------------------------------------------------------------
    def ratio_txt(d):
        return f"{d['load_ratio_in_over_out']:.2f} (passenger-km {d['pkm_ratio_in_over_out']:.2f})"

    dir_tab = pl.DataFrame(
        [
            {"segments": k, "window": w, "inbound load": v["inbound_load"], "outbound load": v["outbound_load"],
             "in/out load ratio": v["load_ratio_in_over_out"], "in/out passenger-km ratio": v["pkm_ratio_in_over_out"]}
            for k, ws in direction.items() for w, v in ws.items()
        ]
    )
    md = f"""
# Known-pattern checks

Model run `full` (Oct 1-31 2026, {n_days['weekday']} weekdays, {n_days['saturday']} Saturdays,
{n_days['sunday']} Sundays and holidays). "Load" is expected riders on board per bus on a stop-to-stop
segment in a 30-minute bin (mid ridership scenario); averages are weighted by buses per day so a busy
corridor counts more than a route with one trip. Shares of "bus-segments" count each bus passing each
segment once.

## 1. Time of day (weekday)

![weekday profile]({fig_wd})

- Morning peak: **{hhmm(am['bin_minute'])}** bin, average load {am['avg_load']:.1f} per bus
  ({pct(am['sh_standing'] + am['sh_crowded'])} of bus-segments standing or crowded).
- Evening peak: **{hhmm(pm['bin_minute'])}** bin, average load {pm['avg_load']:.1f}
  ({pct(pm['sh_standing'] + pm['sh_crowded'])} standing or crowded).
- Midday trough: {hhmm(trough['bin_minute'])} bin, {trough['avg_load']:.1f} per bus.
- Expected: AM peak around 08:00-10:00 and PM peak around 17:00-20:00. Result:
  **{'consistent' if am_ok and pm_ok else 'NOT consistent'}**. The morning peak is
  {am['avg_load'] / pm['avg_load']:.2f} times the evening peak; the evening peak is lower and broader, as in the
  raw ticket purchase times (see audit).
- The grey line is the raw pooled ticket count per bus pass times the expansion factor, without
  shrinkage or the per-route capacity cap. Its peaks fall in the same bins ({hhmm(am_raw['bin_minute'])},
  {hhmm(pm_raw['bin_minute'])}), so the shape comes from the data, not from the smoothing. The model sits
  below the raw line at the peaks ({am['avg_load']:.1f} vs {am_raw['raw_load']:.1f} in the AM peak bin of each);
  without the per-route capacity cap (run `sens_nocap`) the AM peak bin averages {nocap_am:.1f}, so most of
  the gap is the cap lowering {n_adj} routes' expansion, not the shrinkage.

## 2. Weekday vs Saturday vs Sunday

![day types]({fig_dt})

{md_table(pl.DataFrame([{"day type": k, "mean load per bus": dmean[k]["mean_load"], "peak bin": hhmm(pk[k]["peak_bin"]), "peak load": pk[k]["peak_load"], "bus-segments not 'seats'": dmean[k]["sh_not_seats"]} for k in ["weekday", "saturday", "sunday"]]), {"mean load per bus": "{:.1f}", "peak load": "{:.1f}", "bus-segments not 'seats'": pct})}

Saturday loads are {sat_r:.0%} and Sunday/holiday loads {sun_r:.0%} of weekday loads per bus. Bus supply in
the model barely changes at weekends (Saturday {sat_sup:.0%} and Sunday {sun_sup:.0%} of weekday bus-segments),
so per-bus loads fall roughly in line with ticket volume. One caveat: the same expansion factor, set from
weekday ridership, is applied to weekends, which assumes the share of riders using mobile tickets does not
change at weekends. Weekend estimates also rest on only {n_days['saturday']} Saturdays and
{n_days['sunday']} Sundays/holidays, so most weekend cells are low confidence (see coverage).

## 3. Direction relative to Connaught Place

Each segment is "inbound" if its to-stop is closer to Connaught Place (28.6315, 77.2167) than its from-stop
(straight-line distance from GTFS stop coordinates), otherwise "outbound". Peak windows: AM
{hhmm(AM_PEAK[0])}-{hhmm(AM_PEAK[1])}, PM {hhmm(PM_PEAK[0])}-{hhmm(PM_PEAK[1])}.

![direction]({fig_dir})

{md_table(dir_tab, {"inbound load": "{:.1f}", "outbound load": "{:.1f}", "in/out load ratio": "{:.2f}", "in/out passenger-km ratio": "{:.2f}"})}

By distance of the segment from Connaught Place (in/out ratios, weekday):

{md_table(ring_df, {c: "{:.2f}" for c in ring_df.columns if c != "distance from CP"})}

Expected: inbound heavier in the AM peak, outbound heavier in the PM peak. Result:
**{'consistent' if dir_ok else 'NOT consistent'}** (AM in/out ratio {ratio_txt(direction['all segments']['AM'])},
PM {ratio_txt(direction['all segments']['PM'])}). "Mostly radial" keeps segments whose change in distance to
CP is at least half their length; the asymmetry is stronger on them, as it should be if it reflects
travel to and from the centre. By distance ring, the AM ratio is above 1 and the PM ratio below 1 in
{rings_ok} of {len(ring_rows)} rings; the AM asymmetry is strongest in the {best_ring} ring and weakest in the
{worst_ring} ring, where trips to other centres (Gurugram, Noida, Okhla) matter more.

## 4. Busiest corridors and stops

### Top 15 stop-to-stop links by weekday passengers through per day (all routes combined)

{md_table(links, {"buses_per_day": "{:,.0f}", "passengers_per_day": "{:,.0f}", "avg_load": "{:.1f}", "peak_bin_load": "{:.1f}"})}

### Top 15 single-route segments by weekday passengers through per day

{md_table(route_seg.drop("seq"), {"passengers_per_day": "{:,.0f}", "buses_per_day": "{:,.0f}"})}

These are all on {len(top_seg_routes)} route name(s): {', '.join(top_seg_routes)}. One route family dominating the
single-route list is a warning sign, not a finding about Delhi: see the notes below.

### Heaviest peak load per route (top 15 routes, cells with medium or high confidence and at least 1 bus per day)

{md_table(peak_routes, {"load": "{:.1f}", "buses_in_bin": "{:.1f}", "tickets": "{:.0f}"})}

### Top 15 boarding stops (weekday, expanded boardings per day, all routes)

Stops are grouped by name within about 2 km, so same-named stops in different parts of the city stay apart.

{md_table(stops.drop("sector"), {"boardings_per_day": "{:,.0f}", "ticketed_per_day": "{:,.0f}"})}

{res['top15_stops_south']} of the 15 stops lie in the SE, S or SW sectors as seen from Connaught Place.
{res['top15_stops_with_hub_word']} of the 15 stop names contain a hub word ({', '.join(HUB_WORDS)}). The list is
not checked against counts; it is shown so a reader who knows Delhi can judge it.

## 5. Crowding in the weekday peaks (bus-km)

![levels by band]({fig_bands})

{md_table(band_df, {"bus_km": "{:,.0f}", "seats": pct, "standing": pct, "crowded": pct, "exp_standing": pct, "exp_crowded": pct})}

`seats/standing/crowded` = share of bus-km whose shown level (the median bus) is that level;
`exp_*` = expected share of bus-km on buses in that state (averaging the per-bus probabilities). Across both
peak windows, {pct(pk_all['seats'])} of bus-km show seats, {pct(pk_all['standing'])} standing and
{pct(pk_all['crowded'])} crowded.

## 6. Scale and physical sanity

- Expanded weekday boardings in the model: {num(boardings)} against the 4,000,000 the expansion targets.
  The gap is the {pct(1 - res['matched_share'])} of tickets that could not be matched to the timetable: their
  riders are counted in the expansion factor but are not placed on the network, so loads on matched
  routes are not inflated to compensate.
- Weekday bus-km (straight line) {num(tot['bus_km'])}; passenger-km {num(tot['pkm'])}; average
  {tot['pkm'] / tot['bus_km']:.1f} riders per bus-km.
- {pct(tot['over_crush'], 2)} of weekday bus-segments have an expected load above crush capacity; the
  highest single cell is {tot['max_load']:.0f} riders.

## Notes and warnings

- The corridor ranking measures riders who buy mobile tickets, scaled by one city-wide factor. A route
  whose riders use the app more than average ranks higher than its true ridership; the per-route
  capacity cap only corrects the extreme cases. Nothing in this data can separate app adoption from
  ridership, so the ranking should be read as "busiest for app users".
- Several route names are run by two operators and appear twice in the timetable (two `route_id`s with
  the same name). Tickets of one physical bus can be split between the two, and both copies then count
  the same rebuilt bus trips as supply. Route names in the tables above that have this issue:
  {', '.join(dup_in_tables) if dup_in_tables else 'none'}. See `coverage.md` for the size of the issue.
- {res['peak_routes_over_crush']} of the 15 heaviest per-route peaks above exceed crush capacity (85-90), in
  bins with only {oc_bmin:.1f}-{oc_bmax:.1f} buses per day. These cells are "high confidence" because confidence counts tickets only; a timetable
  that lists too few buses in the bin (and too few vehicle-tagged trips to correct it) inflates the
  per-bus load. Confidence does not capture supply uncertainty.
"""
    write_md("patterns", md)
    return res


if __name__ == "__main__":
    main()
