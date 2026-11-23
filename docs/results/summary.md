# Crowding model validation: summary

All numbers come from `analysis/run_all.py` (details in the other files in `docs/results/`).

**Data.** 2,559,899 mobile/QR tickets for October 2026 (one ticket type; discounted digital fares on
99.3% of tickets). About 102,303 ticketed riders per weekday
against an assumed 4M weekday boardings, so the expansion factor is 39 (range 29-49).
93% of tickets were matched to the static timetable (feed v32); 2% are on
routes it lacks and 5% board at a stop not found on the route. 51% carry a
vehicle number, but rebuilt trips hold only 2.0 tickets each, so loads are pooled over
all buses in a 30-minute bin. Buses come from the timetable scaled per route and hour to the buses the live GTFS Realtime feed shows running: 14 snapshots (2026-11-23 19:38 to 2026-11-23 21:25), 5,461 buses seen running, 0.89 running per timetabled trip under way. That is 1.24M bus-km per weekday (straight line between stops), against 0.74M if buses are counted from the tickets alone.

**Stop matching looks right.** Trip length rises with every fare step: AC Rs 9 / 13.5 / 18 / 22.5 give median
trips of 2.8 / 5.9 / 9.2 / 16.3 km; non-AC Rs 4.5 / 9 / 13.5 give
2.8 / 6.8 / 15.9 km. 82% of matched trips fall in
the distance stage their fare implies.

**Known patterns.** Weekday loads peak in the 09:00 bin (28.7 riders per bus,
bus-weighted) and the 18:30 bin (19.6), with a midday low of
5.6 at 14:00; the raw, unsmoothed counts peak in the same bins.
This matches the expected commuting peaks.
Saturday loads are 81% and Sunday 49% of weekday loads.
Toward Connaught Place is 1.55x heavier than away in the AM peak, and away is
1.77x heavier in the PM peak (4 of 4 distance rings agree)
- as expected. Busiest links: Rainy Well -> Delhi Secretariat; Delhi Secretariat -> Rainy Well; Shivaji Park -> Maharaja Agrasen Hospital; SPM College -> Shivaji Park.
Busiest boarding stops: Hamdard Nagar / Sangam Vihar, Badarpur Border(T), Saket Metro Station, Mahipal Pur Village, West Patel Nagar / Patel Nagar. 12 of the top 15 stops lie south-east to
south-west of the centre, the area with the densest tickets (59% of bus-segments with medium or high
confidence in the SW sector vs 32% in the NE), so the rankings may partly reflect app
adoption rather than ridership alone.

**How crowded.** Weekday bus-km shown as standing or crowded: 26.2% in the AM peak
(08:00-10:00), 13.7% in the PM peak (17:30-20:00), 8.6% all day.

**Week to week.** A model fitted on Oct 1-24 predicts Oct 25-31 (5 weekdays) with Spearman
0.91 across 157,530 weekday cells with >= 10 test tickets
(level agreement 81%, kappa 0.66); route x hour totals
correlate at 0.95. The route all-day average scores 0.07 (mean error
22.2 vs 6.2 riders per bus) and the segment all-day average
0.36, so time of day carries real signal. Agreement rises with test-week tickets, as sampling noise predicts.

**Sensitivity.** The ridership assumption (3M / 5M) changes the level of 5.6% /
6.2% of weekday bus-segments; structural choices change
sens_nocap 12.5%, sens_tickets 10.2%, sens_shrink 1.7%, sens_timetable 1.3%, sens_cv 1.0%, sens_spread 0.6%. The share of AM-peak bus-segments shown standing
or crowded ranges 14.7%-38.6% across runs: quote it with that range.

**Least reliable.** Only 43% of weekday bus-segments have medium or high confidence
(AM peak 66%, midday 37%);
88% of Saturday and 92% of Sunday bus-segments are low. Coverage is best in
the SW/S/SE sectors and weakest in the NE/N/W sectors and beyond 20 km
(25%); weakest busy areas: Rana Pratap Bagh (7 km N of CP), Ice Factory (Rani Jhansi Road) (4 km N of CP), Chhajjupur (10 km NE of CP), Narela Terminal (27 km NW of CP).

**Fixed during validation.** Tickets name only the route, so the stop patterns of one route (full and
short-turn trips) are merged onto the pattern that covers them (924 merged,
0 kept apart because their stops do not line up). Counting buses from tickets alone
missed many buses that ran; the live feed now sets the bus count. Loads above crush capacity
are capped at crush and flagged (0 high-confidence cells remain above crush), and a bin
whose bus count rests on few buses can no longer be high confidence.

**Known limits.**
(1) The live feed is from 2026-11-23 while the tickets are from October; hours without snapshots use the average factor of their day type (or of all snapshots).
(2) The network carries 3,922,768 weekday boardings: tickets on routes missing from the
timetable (2%) or boarding at a stop not on the route (5%) are scaled in per route, not placed.
(3) One city-wide expansion factor: corridor rankings reflect app adoption as well as ridership.
(4) Vehicle-number share jumps to 85% on 2026-10-18 (about half on other days).
