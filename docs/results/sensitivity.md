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
| full (reference) | mid ridership 4M weekday boardings (factor 39.1) | 82.6% | 13.2% | 4.2% | 35.6% | 22.86 | 0.0% | 0.0% | 0.0% | 1.000 | 645 |
| ridership low | 3M weekday boardings (factor 29.3) | 91.0% | 8.7% | 0.4% | 22.1% | 17.17 | 12.2% | 0.0% | 12.2% | 0.751 | 645 |
| ridership high | 5M weekday boardings (factor 48.9) | 74.7% | 16.3% | 9.0% | 45.5% | 28.20 | 12.7% | 12.7% | 0.0% | 1.233 | 645 |
| sens_spread | alighting spread: 50% of riders get off one stop early or late | 82.6% | 13.2% | 4.2% | 35.7% | 22.87 | 1.2% | 0.6% | 0.6% | 1.000 | 637 |
| sens_cv | trip-to-trip load CV 0.50 instead of 0.35 | 84.0% | 12.6% | 3.4% | 33.6% | 22.86 | 2.2% | 0.0% | 2.2% | 1.000 | 645 |
| sens_noobs | bus supply from the timetable only (no ticket-based supply) | 93.0% | 5.3% | 1.7% | 20.6% | 12.03 | 13.9% | 6.3% | 7.6% | 0.901 | 706 |
| sens_nocap | no per-route capacity-consistency cap | 69.8% | 16.2% | 14.0% | 48.9% | 31.60 | 19.4% | 19.4% | 0.0% | 1.382 | 0 |
| sens_shrink | shrinkage prior weight 20 trip-days instead of 8 | 81.7% | 13.9% | 4.4% | 36.2% | 23.75 | 2.3% | 1.7% | 0.6% | 1.039 | 592 |

"Load vs full (shared cells)" compares bus-weighted mean load on the cells both runs have, with the `full`
run's bus weights, so composition changes do not blur it. "Capped routes" is the number of routes whose
expansion the capacity check lowered.

Cells of `full` missing from a scenario (bus-weighted): sens_noobs 9.19%.
Without ticket-based supply (`sens_noobs`) the model has 138.3% of the reference
weekday bus-segments. Two things happen. (1) Cells where only rebuilt trips showed a bus drop out
(9.2% of reference bus-segments); in `full` these carry an average load of
20.1, well above the network mean of 22.9, i.e. the 2025 timetable
carries buses with real riders (partly a selection effect: these cells exist only where tagged tickets were
seen). (2) On the shared cells the ticketed rate per bus rises by
62.7%, but the capacity check then lowers 706 routes instead of
645, so the load rises only -9.9%. The mean load over the run's own cells
therefore falls, even though each remaining bus is fuller before the cap.

## Reading

- The ridership assumption matters most: it moves every load by the same factor. Going from 4M to 3M or 5M
  weekday boardings changes the level of 12.2% / 12.7% of
  weekday bus-segments. Most of the network stays "seats" under all scenarios; 49.3%
  of the cells that change under the high scenario are in the two peak windows, which hold
  34.8% of weekday bus-segments.
- Ranked by level changes: sens_nocap 19.4%; sens_noobs 13.9%; ridership high 12.7%; ridership low 12.2%; sens_shrink 2.3%; sens_cv 2.2%; sens_spread 1.2%.
- Supply is the largest of the structural choices (see above): the ticket-based supply adds buses the
  timetable lacks, and the capacity cap absorbs much of the difference when it is removed. The two
  mechanisms interact, so neither should be judged alone.
- The capacity cap only lowers loads on the routes it touches, so dropping it only moves cells toward
  more crowded; the alighting spread and the shrinkage weight barely matter at the network level.
- No assumption flips the overall picture (a large majority of weekday bus-segments with seats, crowding
  concentrated in the AM peak), but the share of peak cells shown as standing or crowded ranges from
  20.6% to 48.9% across these runs, so the absolute
  crowding level should be presented with its range.
