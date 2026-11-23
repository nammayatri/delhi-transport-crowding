# Sensitivity to assumptions

Weekday cells (route pattern x segment x 30-minute bin). Shares of seats / standing / crowded are of
bus-segments (each cell weighted by its buses per day in that run). "Level changed vs full" is the share of
weekday bus-segments (weighted by the `full` run's buses) whose level differs from `full`, split into
cells that become more or less crowded. "AM peak not seats" is the share of bus-segments in the
08:00-10:00 bins at standing or crowded. The ridership rows reuse `full` with
`load_low` / `load_high` and the same level rule (gamma load, CV sqrt(0.35^2 + 1/tickets), seats and
comfortable thresholds of the bus class) and keep the per-route capacity adjustment computed under the mid
scenario (a high-ridership run would cap more routes, so its crowding is an upper bound); re-deriving the mid level this way reproduces the stored level in
100.000% of cells.

| scenario | change | seats | standing | crowded | AM peak not seats | mean load | level changed vs full | more crowded | less crowded | load vs full (shared cells) | capped routes |
| :--- | :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| full (reference) | mid ridership 4M weekday boardings (factor 39.1) | 91.9% | 6.2% | 1.9% | 25.1% | 13.19 | 0.0% | 0.0% | 0.0% | 1.000 | 841 |
| ridership low | 3M weekday boardings (factor 29.3) | 95.9% | 3.8% | 0.3% | 14.7% | 9.91 | 5.6% | 0.0% | 5.6% | 0.752 | 841 |
| ridership high | 5M weekday boardings (factor 48.9) | 87.8% | 8.1% | 4.1% | 33.3% | 16.30 | 6.2% | 6.2% | 0.0% | 1.236 | 841 |
| sens_spread | alighting spread: 50% of riders get off one stop early or late | 91.9% | 6.2% | 1.9% | 25.3% | 13.19 | 0.6% | 0.3% | 0.3% | 0.999 | 836 |
| sens_cv | trip-to-trip load CV 0.50 instead of 0.35 | 92.6% | 5.8% | 1.6% | 23.6% | 13.22 | 1.0% | 0.0% | 1.0% | 1.002 | 843 |
| sens_timetable | bus supply from the timetable as published (no live-feed calibration) | 92.4% | 5.8% | 1.8% | 24.2% | 12.58 | 1.3% | 0.5% | 0.8% | 0.975 | 765 |
| sens_tickets | bus supply from buses seen in the tickets only (capture-corrected) | 82.6% | 13.2% | 4.2% | 35.4% | 22.43 | 10.2% | 8.7% | 1.5% | 1.521 | 842 |
| sens_nocap | no per-route capacity-consistency cap | 83.0% | 9.0% | 8.0% | 38.6% | 19.62 | 12.5% | 12.4% | 0.1% | 1.487 | 0 |
| sens_shrink | shrinkage prior weight 20 trip-days instead of 8 | 91.1% | 6.9% | 2.1% | 26.7% | 14.19 | 1.7% | 1.3% | 0.4% | 1.074 | 723 |

"Load vs full (shared cells)" compares bus-weighted mean load on the cells both runs have, with the `full`
run's bus weights, so composition changes do not blur it. "Capped routes" is the number of routes whose
expansion the capacity check lowered.

Cells of `full` missing from a scenario (bus-weighted): sens_tickets 5.77%.

**Bus supply.** The reference run counts buses from the timetable, scaled per route and hour to the buses the
live GTFS Realtime feed shows running (14 snapshots from 2026-11-23 19:38 to 2026-11-23 21:25; 5,461 distinct buses seen running; overall 0.89 buses running per timetabled trip under way). Straight-line bus-km per weekday:
full (live-feed calibrated) 1.24M, timetable 1.40M, tickets 0.74M. Using the timetable as published
(`sens_timetable`) gives 112.8% of the reference weekday bus-segments; counting
buses from the tickets only (`sens_tickets`) gives 63.8%, because many buses that
run sell no tagged ticket, and its loads on shared cells are
52.1% higher.

## Reading

- The ridership assumption matters most: it moves every load by the same factor. Going from 4M to 3M or 5M
  weekday boardings changes the level of 5.6% / 6.2% of
  weekday bus-segments. Most of the network stays "seats" under all scenarios; 56.5%
  of the cells that change under the high scenario are in the two peak windows, which hold
  28.9% of weekday bus-segments.
- Ranked by level changes: sens_nocap 12.5%; sens_tickets 10.2%; ridership high 6.2%; ridership low 5.6%; sens_shrink 1.7%; sens_timetable 1.3%; sens_cv 1.0%; sens_spread 0.6%.
- Bus supply is the largest structural choice (see above). Counting buses from tickets alone misses the
  buses that sell no tagged ticket and makes every bus fuller; the live feed removes most of that guesswork
  by showing which buses actually run. The capacity cap absorbs part of any supply error, so the two
  should be judged together.
- The capacity cap only lowers loads on the routes it touches, so dropping it only moves cells toward
  more crowded; the alighting spread and the shrinkage weight barely matter at the network level.
- No assumption flips the overall picture (a large majority of weekday bus-segments with seats, crowding
  concentrated in the AM peak), but the share of peak cells shown as standing or crowded ranges from
  14.7% to 38.6% across these runs, so the absolute
  crowding level should be presented with its range.
