# Crowding model validation: summary

All numbers come from `analysis/run_all.py` (details in the other files in `docs/results/`).

**Data.** 2,559,899 mobile/QR tickets for October 2026 (one ticket type; discounted digital fares on
99.3% of tickets). About 102,303 ticketed riders per weekday
against an assumed 4M weekday boardings, so the expansion factor is 39 (range 29-49).
78% of tickets were matched to the 2025 timetable; 13% are on
routes it lacks and 9% board at a stop not found on the route. 51% carry a
vehicle number, but rebuilt trips hold only 2.0 tickets each, so loads are pooled over
all buses in a 30-minute bin.

**Stop matching looks right.** Trip length rises with every fare step: AC Rs 9 / 13.5 / 18 / 22.5 give median
trips of 2.8 / 6.0 / 9.4 / 16.7 km; non-AC Rs 4.5 / 9 / 13.5 give
2.8 / 6.9 / 16.6 km. 81% of matched trips fall in
the distance stage their fare implies.

**Known patterns.** Weekday loads peak in the 09:00 bin (35.9 riders per bus,
bus-weighted) and the 19:00 bin (28.9), with a midday low of
13.3 at 13:00; the raw, unsmoothed counts peak in the same bins.
This matches the expected commuting peaks.
Saturday loads are 90% and Sunday 57% of weekday loads.
Toward Connaught Place is 1.40x heavier than away in the AM peak, and away is
1.53x heavier in the PM peak (4 of 4 distance rings agree)
- as expected. Busiest links: West Patel Nagar -> South Patel Nagar (Metro Station); South Patel Nagar (Metro Station) -> East Patel Nagar; Delhi Secretariat -> Rainy Well; Madipur Village -> SPM College.
Busiest boarding stops: Badarpur Border (T), Hamdard Nagar / Sangam Vihar, Said ul zeb Village, West Patel Nagar, Ambedkar Nagar Depot. 10 of the top 15 stops lie south-east to
south-west of the centre, the area with the densest tickets (77% of bus-segments with medium or high
confidence in the S sector vs 45% in the N), so the rankings may partly reflect app
adoption rather than ridership alone.

**How crowded.** Weekday bus-km shown as standing or crowded: 36.8% in the AM peak
(08:00-10:00), 25.0% in the PM peak (17:30-20:00), 18.1% all day.

**Week to week.** A model fitted on Oct 1-24 predicts Oct 25-31 (5 weekdays) with Spearman
0.87 across 138,978 weekday cells with >= 10 test tickets
(level agreement 76%, kappa 0.61); route x hour totals
correlate at 0.93. The route all-day average scores 0.14 (mean error
20.8 vs 7.4 riders per bus) and the segment all-day average
0.54, so time of day carries real signal. Agreement rises with test-week tickets, as sampling noise predicts.

**Sensitivity.** The ridership assumption (3M / 5M) changes the level of 12.2% /
12.7% of weekday bus-segments; structural choices change
sens_nocap 19.4%, sens_noobs 13.9%, sens_shrink 2.3%, sens_cv 2.2%, sens_spread 1.2%. The share of AM-peak bus-segments shown standing
or crowded ranges 20.6%-48.9% across runs: quote it with that range.

**Least reliable.** Only 58% of weekday bus-segments have medium or high confidence
(AM peak 75%, midday 53%);
80% of Saturday and 87% of Sunday bus-segments are low. Coverage is best in
the S/SW/SE sectors and weakest in the N/NE/W sectors and beyond 20 km
(39%); weakest busy areas: Om Vihar (16 km W of CP), Narela Terminal (27 km NW of CP), Ice Factory (Rani Jhansi Road) (4 km N of CP), Telephone Exchange (7 km N of CP).

**Fixed during validation.** Route numbers run by two operators were first counted as two
routes, double counting buses; they are now merged (only 27 names stay separate, as their
stops do not line up; 3% same-vehicle twins remain). Loads above crush capacity
are now capped at crush and flagged (0 high-confidence cells remain above crush), and a bin
whose bus count rests on tickets alone or on few buses can no longer be high confidence.

**Known limits.**
(1) The capture correction sits at its cap on 69 routes (2% of weekday bus-segments).
(2) The network carries 3,462,064 weekday boardings: tickets on routes missing from the
2025 timetable (13%) are not placed.
(3) One city-wide expansion factor: corridor rankings reflect app adoption as well as ridership.
(4) Vehicle-number share jumps to 85% on 2026-10-18 (about half on other days).
