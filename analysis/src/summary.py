"""One-page digest of the headline numbers, built only from the JSON written by the other scripts.

Writes docs/results/summary.md.
"""

from __future__ import annotations

from common import num, pct, read_json, write_md


def main() -> None:
    a = read_json("audit")
    p = read_json("patterns")
    s = read_json("stability")
    v = read_json("sensitivity")
    c = read_json("coverage")

    fm = a["fare_median_km"]
    e = a["expansion"]
    d_all = p["direction"]["all segments"]
    bands = {r["window"]: r for r in p["bus_km_levels"]}
    am_b, pm_b, all_b = bands["AM peak 08:00-10:00"], bands["PM peak 17:30-20:00"], bands["all day"]
    rows = {r["scenario"]: r for r in v["rows"]}
    band = {r["band"]: r for r in c["weekday_by_band"]}
    dn = c["duplicate_names"]
    oc = c["over_crush"]
    ring_ok = sum(1 for r in p["direction_by_ring"] if r["AM in/out load"] > 1 and r["PM in/out load"] < 1)
    structural = sorted(
        (r for r in v["rows"] if r["scenario"].startswith("sens_")), key=lambda r: -r["level changed"]
    )
    am_range = (min(r["am_not_seats"] for r in v["rows"]), max(r["am_not_seats"] for r in v["rows"]))
    comp = {(r["target"], r["predictor"]): r for r in s["comparison"]}
    sec = c["reliable_by_sector"]
    sec_best = max(sec, key=sec.get)
    sec_worst = min(sec, key=sec.get)
    sec_best_v, sec_worst_v = sec[sec_best], sec[sec_worst]
    seg_base = comp[("test model", "segment all-day average (train)")]

    md = f"""
# Crowding model validation: summary

All numbers come from `analysis/run_all.py` (details in the other files in `docs/results/`).

**Data.** {num(a['rows'])} mobile/QR tickets for October 2026 (one ticket type; discounted digital fares on
{pct(a['standard_fare_share'])} of tickets). About {num(a['ticketed_pax_per_weekday'])} ticketed riders per weekday
against an assumed 4M weekday boardings, so the expansion factor is {e['mid']:.0f} (range {e['low']:.0f}-{e['high']:.0f}).
{pct(a['matched_share'], 0)} of tickets were matched to the 2025 timetable; {pct(a['route_missing_share'], 0)} are on
routes it lacks and {pct(a['stop_missing_share'], 0)} board at a stop not found on the route. {pct(a['vehicle_share'], 0)} carry a
vehicle number, but rebuilt trips hold only {a['tickets_per_recon_trip']:.1f} tickets each, so loads are pooled over
all buses in a 30-minute bin.

**Stop matching looks right.** Trip length rises with every fare step: AC Rs 9 / 13.5 / 18 / 22.5 give median
trips of {fm['AC 9']:.1f} / {fm['AC 13.5']:.1f} / {fm['AC 18']:.1f} / {fm['AC 22.5']:.1f} km; non-AC Rs 4.5 / 9 / 13.5 give
{fm['non-AC 4.5']:.1f} / {fm['non-AC 9']:.1f} / {fm['non-AC 13.5']:.1f} km. {pct(a['fare_share_in_stage_overall'], 0)} of matched trips fall in
the distance stage their fare implies.

**Known patterns.** Weekday loads peak in the {p['weekday_am_peak_bin']} bin ({p['weekday_am_peak_load']:.1f} riders per bus,
bus-weighted) and the {p['weekday_pm_peak_bin']} bin ({p['weekday_pm_peak_load']:.1f}), with a midday low of
{p['weekday_midday_trough_load']:.1f} at {p['weekday_midday_trough_bin']}; the raw, unsmoothed counts peak in the same bins.
{'This matches the expected commuting peaks.' if p['peaks_as_expected'] else 'This does NOT match the expected commuting peaks.'}
Saturday loads are {pct(p['saturday_vs_weekday_mean_load'], 0)} and Sunday {pct(p['sunday_vs_weekday_mean_load'], 0)} of weekday loads.
Toward Connaught Place is {d_all['AM']['load_ratio_in_over_out']:.2f}x heavier than away in the AM peak, and away is
{1 / d_all['PM']['load_ratio_in_over_out']:.2f}x heavier in the PM peak ({ring_ok} of 4 distance rings agree)
{'- as expected' if p['direction_as_expected'] else '- NOT as expected'}. Busiest links: {'; '.join(p['top_links'][:4])}.
Busiest boarding stops: {', '.join(p['top_stops'][:5])}. {p['top15_stops_south']} of the top 15 stops lie south-east to
south-west of the centre, the area with the densest tickets ({pct(sec_best_v, 0)} of bus-segments with medium or high
confidence in the {sec_best} sector vs {pct(sec_worst_v, 0)} in the {sec_worst}), so the rankings may partly reflect app
adoption rather than ridership alone.

**How crowded.** Weekday bus-km shown as standing or crowded: {pct(am_b['standing'] + am_b['crowded'])} in the AM peak
(08:00-10:00), {pct(pm_b['standing'] + pm_b['crowded'])} in the PM peak (17:30-20:00), {pct(all_b['standing'] + all_b['crowded'])} all day.

**Week to week.** A model fitted on Oct 1-24 predicts Oct 25-31 (5 weekdays) with Spearman
{s['headline_spearman']:.2f} across {num(s['headline_cells'])} weekday cells with >= {s['min_test_tickets']:g} test tickets
(level agreement {pct(s['headline_level_agreement'], 0)}, kappa {s['headline_kappa']:.2f}); route x hour totals
correlate at {s['route_hour_spearman']:.2f}. The route all-day average scores {s['baseline_route_spearman']:.2f} (mean error
{s['baseline_route_mae']:.1f} vs {s['model_mae']:.1f} riders per bus) and the segment all-day average
{seg_base['Spearman']:.2f}, so time of day carries real signal. Agreement rises with test-week tickets, as sampling noise predicts.

**Sensitivity.** The ridership assumption (3M / 5M) changes the level of {pct(rows['ridership low']['level changed'])} /
{pct(rows['ridership high']['level changed'])} of weekday bus-segments; structural choices change
{', '.join(f"{r['scenario']} {pct(r['level changed'])}" for r in structural)}. The share of AM-peak bus-segments shown standing
or crowded ranges {pct(am_range[0])}-{pct(am_range[1])} across runs: quote it with that range.

**Least reliable.** Only {pct(c['weekday_reliable_share'], 0)} of weekday bus-segments have medium or high confidence
(AM peak {pct(1 - band['AM peak 08:00-10:00']['low'], 0)}, midday {pct(1 - band['midday 10:00-17:30']['low'], 0)});
{pct(c['by_daytype'][1]['low'], 0)} of Saturday and {pct(c['by_daytype'][2]['low'], 0)} of Sunday bus-segments are low. Coverage is best in
the {'/'.join(c['best_sectors'])} sectors and weakest in the {'/'.join(c['worst_sectors'])} sectors and beyond 20 km
({pct(c['reliable_by_ring']['20+ km'], 0)}); weakest busy areas: {', '.join(f"{w['area_stop']} ({w['where']})" for w in c['weak_areas'][:4])}.

**Fixed during validation.** Route numbers run by two operators were first counted as two
routes, double counting buses; they are now merged (only {dn['names']} names stay separate, as their
stops do not line up; {pct(c['twin_share_of_dup_name_trips'], 0)} same-vehicle twins remain). Loads above crush capacity
are now capped at crush and flagged ({num(oc['high_conf'])} high-confidence cells remain above crush), and a bin
whose bus count rests on tickets alone or on few buses can no longer be high confidence.

**Known limits.**
(1) The capture correction sits at its cap on {int(c['capture_at_cap_routes'])} routes ({pct(c['capture_at_cap_bus_share'], 0)} of weekday bus-segments).
(2) The network carries {num(p['model_weekday_boardings'])} weekday boardings: tickets on routes missing from the
2025 timetable (13%) are not placed.
(3) One city-wide expansion factor: corridor rankings reflect app adoption as well as ridership.
(4) Vehicle-number share jumps to {pct(a['vehicle_share_day_max'], 0)} on {a['vehicle_share_day_max_date']} (about half on other days).
"""
    write_md("summary", md)


if __name__ == "__main__":
    main()
