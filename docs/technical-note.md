# Crowding-aware bus journeys for Delhi: technical note

## What we built

We built three things:
- **A pipeline** that turns the October 2026 ticket records into an estimate of how many people are on board, by route, direction, stop-to-stop stretch, day type and 30-minute bin.
- **A crowding service** that serves those estimates to commuters and operators: a journey planner, a network map and operator tables.
- **A patch for OpenTripPlanner 2.5** so the planner itself can weigh crowding when it searches, not just display it afterwards.

All numbers in this note come from `scripts/run_all_models.sh`. The detail is in `docs/results/`.

## 1. What the ticket data is, and what that means for the method

| Fact from the data | Consequence |
|---|---|
| 2,559,899 tickets for 1–31 Oct. All are `ticket_type_id` 1, and 99% are at discounted digital fares (₹4.5 / 9 / 13.5 non-AC, ₹9 / 13.5 / 18 / 22.5 AC). | These are mobile/QR tickets: a sample of riders, not all of them. |
| About 102,300 ticketed riders on an average weekday, against roughly 4 million city bus boardings. | We need an **expansion factor**, about 39 at the central assumption. |
| 51% of tickets carry a vehicle number, but a rebuilt bus trip carries only **2.0 tagged tickets on average** (median 1). | A single trip's running count is too noisy to use. We keep the running count **for every ticket**, then pool it over every bus that runs in the same bin. |
| The 2025 timetable does not match the buses actually running. At its peak it needs about 9,100 buses at once, but only about 5,300 different buses sell a ticket on these routes all day. Some routes run far more than planned: 419UP has 1 trip a day in the timetable and about 60 in the tickets. | **Buses are counted from the tickets only** (§2.5). The timetable only gives stop order and running times. |
| 431 route numbers are run by **two operators**. Each is listed as two routes with separate stop ids at the same locations. | The two copies are merged into one route group (§2.2). |
| Oct 2 (Gandhi Jayanti) and Oct 20 (Dussehra) have Sunday-like volumes. | Both are treated as Sundays. Day types are weekday (20 days), Saturday (5) and Sunday/holiday (6). |

## 2. Method

### 2.1 Matching tickets to the timetable
**Route.** Route names are matched exactly after normalising: no spaces, `DWN` becomes `DOWN`, and a trailing variant digit such as `522BDOWN2` is dropped.

**Stops.** Each stop name is fuzzy-matched only against the stops of that ticket's own route. Both names are normalised first: abbreviations are expanded (Xing → crossing, Bdr → border, (T) → terminal) and alternate names such as "Saidul-A-Jab / Saket Metro Station" are split. The score blends token-set and token-sort similarity with a no-space comparison. If no stop on the route matches, a stop elsewhere with exactly the same name that lies within 400 m of the route is accepted.

**Direction.** If the two stops appear in reverse order on the ticket's route, the ticket is assigned to the opposite direction (8,100 tickets).

**Result.** 77.8% of tickets were placed:
- 1,752,742 exact matches
- 8,100 direction fixes
- 229,574 with the alighting stop imputed

The rest could not be placed:
- 12.8% are on routes absent from the 2025 timetable, mostly the newer D- and OMS families.
- 9.5% have a boarding stop that was not found on the route.

**Evidence the matching is right.** Trip length rises with every fare step:
| Fare paid | Median trip |
|---|---|
| AC ₹9 | 2.8 km |
| AC ₹13.5 | 6.0 km |
| AC ₹18 | 9.4 km |
| AC ₹22.5 | 16.7 km |
| Non-AC ₹4.5 | 2.8 km |
| Non-AC ₹9 | 6.9 km |
| Non-AC ₹13.5 | 16.6 km |

81% of matched trips fall inside the distance stage their fare implies. If stops were matched at random, distance would not track fare like this.

### 2.2 Routes run by two operators
Each operator's copy of a route is mapped onto the copy with the most trips. Every stop is mapped to the nearest stop on that canonical copy, moving only forward along the route; the median distance between matched stops is 0 m. Tickets and buses from both copies are then counted on the canonical pattern.

392 copies were merged. 40 were kept apart because their stops do not line up.

### 2.3 Missing alighting stops
When the alighting stop cannot be matched, it is imputed:
- Candidate stops are those downstream whose distance from the boarding stop falls inside the 5th–95th percentile distance seen for that fare. These bands are learned from exact matches.
- Each candidate is weighted by where riders who boarded at the same stop were seen to get off.
- The rider is spread fractionally over the candidates.

Riders may also get off earlier or later than their ticket says. A sensitivity run moves 50% of riders one stop early or late (§4).

### 2.4 Running onboard count, pooled over buses
A ticket counts as on board on every stretch between its boarding and alighting stop. The time it crosses each stretch is the ticket time plus the scheduled running time of the nearest timetabled trip.

For each stretch, day type and 30-minute bin:

```
ticketed load per bus = ticketed riders crossing the stretch in the bin / bus passes of the stretch in the bin
```

Both numerator and denominator are summed over all days of that day type. This is the average of every bus's running count in the bin, including buses that carried no ticket.

Bins are smoothed with a [¼ ½ ¼] kernel. Sparse bins are shrunk step by step, each step adding 8 trip-days of prior weight:
1. Each bin is pulled toward the same stretch over ±2 hours.
2. That window is pulled toward the stretch's all-day rate, scaled by the city-wide time-of-day shape.
3. The stretch's all-day rate is pulled toward the route's average.

### 2.5 Bus supply: counted from the tickets
How many buses pass a stretch in a half hour is counted from the tickets, not from the timetable.
1. **Rebuild trips.** Bus trips are rebuilt from vehicle-tagged tickets, grouped by vehicle and date. A new trip starts when the route number changes, after a gap of more than 90 minutes, or when boardings jump back toward the start of the route. That gives 507,187 trips.
2. **Place the bus along its route.** Each rebuilt trip is placed along its route using the running times of the nearest timetabled trip.
3. **Correct for trips that sold no tagged ticket.** If `o` trips are seen carrying `t` tagged tickets, and tickets fall on trips at random, the number of trips that ran, `n`, solves `o = n·(1 − e^(−t/n))`. A bus sells more tickets at the peak than at midday, so the correction is estimated per route, day type and 2-hour band. When a cell has too few trips it falls back to the route's whole day, then to the city. The correction is capped at 3×.
4. **No bus, no estimate.** A stretch with no bus seen around that time gets no estimate.

**Why not the timetable.** At its peak, the 2025 timetable has about 9,100 trips in progress at once. Yet only about 5,300 different buses sell a tagged ticket on these routes on a weekday. Each bus sells about 8 a day, so nearly every bus that runs is seen. Using the timetable spread riders over buses that do not run, and made peak buses look far emptier than riders report. At 09:00, 50% of riders were seated instead of 37%.

**Remaining bias.** Ticket-counted buses add up to 0.68M bus-km a weekday. About 5,300 buses would cover roughly 1.0M. So some trips still go unseen, the correction is conservative, and loads may lean high. The capacity check (§2.6) limits this.

**Alternatives.** The timetable, scaled to the fleet seen in the tickets, is available as `supply_source = "timetable"`; `"max"` takes the larger of the two. The timetable variant is a sensitivity run in §4.

### 2.6 From ticketed riders to all riders
Tickets miss passholders, concession riders (including free travel for women), cash tickets and fare evasion. We scale up so that an average weekday matches published city bus ridership:

| Assumption | Weekday boardings | Expansion factor |
|---|---|---|
| Low | 3.0 M | 29.3 |
| Mid | 4.0 M | 39.1 |
| High | 5.0 M | 48.9 |

Every load is reported under all three.

Two route-level adjustments:
- **Unplaced tickets.** On routes we do cover, tickets whose boarding stop could not be placed still rode that route. The route is scaled up for them, at most 1.5×; the average is 1.14×.
- **Capacity consistency.** A bus cannot carry more than crush load. If the city-wide factor puts a route's 99th-percentile weekday load above crush, that route's riders evidently use tickets more than average, so its factor is lowered until it fits. This applies to 888 of 1,394 routes. Displayed loads are also capped at crush capacity and flagged `over_capacity`.

### 2.7 Bus size and crowding levels
No fleet list was available, so the bus class comes from rules in `crowding/config/model.dhall`:

| Class | Seats | Comfortable up to | Crush |
|---|---|---|---|
| 12 m AC | 37 | 62 | 85 |
| 12 m non-AC (most of a route's tickets are non-AC) | 40 | 65 | 90 |
| 9 m electric (routes `D-…`) | 23 | 38 | 55 |

The 2025 timetable has no D- routes, so the 9 m rule is unused on this feed.

Loads vary from bus to bus within a bin. We model that spread as a gamma distribution around the expected load. Its coefficient of variation combines a 0.35 trip-to-trip spread with the sampling error, 1/√tickets. From this we get the share of buses with seats, with standing room and crowded.

The commuter sees **the level of the median bus**: Seats available, Standing room or Crowded. If that level holds on at least 60% of buses we say "usually", otherwise "often".

Internally we keep the GTFS-Realtime `OccupancyStatus` scale, so the same numbers can be published in a standard feed.

### 2.8 Comfort cost, and crowding-aware routing
Each stretch's ride time is weighted by the expected crowding multiplier: seated 1.0, standing 1.4, crowded 1.9, from crowding-valuation studies.

For every trip we publish a cumulative crowding cost: `C[k+1] = C[k] + ride(k)·(multiplier(k) − 1)`. Riding from stop i to stop j "feels" `C[j] − C[i]` seconds longer than it is.

The OpenTripPlanner patch adds `crowdingReluctance × (C[j] − C[i])` to the generalized cost of every ride inside RAPTOR:
- The extra cost is never negative, so RAPTOR's lower bounds stay valid.
- When RAPTOR compares two trips of the same route boarded at the same stop, it also counts the crowding downstream of the boarding stop.

The journeys endpoint plans at the requested time and at +10, +20 and +30 minutes, both with and without crowding reluctance. It labels the options **Fastest**, **Most comfortable** and **Leave later, seats likely**, and ranks them using the user's slider.

### 2.9 Live tickets
At about 2 tickets per bus trip, live tickets say little about any single bus. They do say whether a route is busier than usual right now. The service compares a route's tickets over the last hour with the number expected for that route, day type and hour. It shrinks the ratio toward 1 (6 pseudo-tickets), clamps it to 0.5–2.5, and applies it to the next three hours.

Live tickets arrive over HTTP (`POST /v1/tickets`) or Kafka. `crowd-replay` replays any October day as a live stream.

## 3. Checks against known patterns
| Check | Result |
|---|---|
| Weekday peaks | Load peaks in the 09:00 bin (35.9 riders per bus) and the 19:00 bin (28.9), with a midday low of 13.3. The raw counts peak in the same bins. At 09:00, 37% of riders are seated, 31% stand and 32% are in a crowded bus. Toward the centre, 8–16 km out, the average bus carries 44 riders, so its 37 seats are full. |
| Weekday vs weekend | Saturday loads are 90% of weekday, Sunday/holiday 57%. |
| Direction | Travel toward Connaught Place is 1.40× heavier than away in the morning peak. Away is 1.53× heavier in the evening. All four distance rings agree. |
| Busiest corridors | Delhi Secretariat ↔ Rainy Well (ITO), Patel Nagar, Saket–Asian Market, Badarpur Border, and Mahipalpur/NH-8. |
| How crowded | In the weekday morning peak (08:00–10:00), 36.8% of bus-km are standing room or crowded; 20.6–48.9% across all sensitivity runs. In the evening peak (17:30–20:00) it is 25.0%. |
| Week to week | The model fitted on Oct 1–24 predicts Oct 25–31 with Spearman **0.87** over 138,978 weekday cells that have at least 10 test tickets. Levels agree on 76% of cells (κ 0.61), and route × hour totals correlate at 0.93. A route all-day average scores only 0.14, so time of day carries real signal. |

Figures are in `docs/figures/`.

## 4. Sensitivity
Share of weekday bus-segments whose level changes, compared with the main run:

| Run | Level changes |
|---|---|
| Ridership low (3 M) / high (5 M) | 12.2% / 12.7% |
| No capacity consistency | 19.4% |
| Timetable supply (scaled to the fleet) instead of ticket-counted buses | 13.9% |
| Shrinkage weight 20 | 2.3% |
| Trip-to-trip CV 0.5 | 2.2% |
| 50% of riders get off one stop early or late | 1.2% |

The modelling details (smoothing, spread, alighting) matter little. What matters is how many riders there are and how many buses carry them: the ridership assumption, the capacity check and the bus count. That is why every load comes with a low and a high value, and why the map and dashboard let you switch scenarios.

## 5. Where the estimates are least reliable
- **Weekends.** 80% of Saturday and 87% of Sunday bus-segments are low confidence, because there are only 5–6 days of each.
- **North and outer Delhi.** In the north, north-east and north-west, and beyond 20 km from the centre (39% reliable), ticket density is low. The weakest busy areas include Om Vihar, Narela Terminal, Ice Factory (Rani Jhansi Road) and Telephone Exchange. Coverage is best in the south, south-east and south-west. **Busy-corridor rankings therefore partly reflect where people use the ticketing app**, since one city-wide expansion factor cannot separate the two.
- **Routes missing from the 2025 timetable.** These carry 13% of tickets, mostly D- and OMS, and get no estimate. Loading the current timetable fixes this with no code change.
- **Supply.** Buses are counted from the tickets, so trips that sell no tagged ticket are estimated. The correction is at its cap on 2% of bus-segments. A bin that rests on very few buses is never rated high confidence. Total ticket-counted bus-km suggests the count is still on the low side (§2.5).
- **Hours.** One expansion factor is used for every hour. Pass holders and free-travel riders are not in the tickets and probably ride more at the peaks, which would make peaks busier than shown.
- **Absolute levels** depend on the ridership assumption (§2.6) and on seat counts inferred without a fleet list.

Every cell carries a confidence of high, medium or low. Confidence is high with at least 40 ticketed riders behind the estimate, medium with at least 12, and is capped further by supply quality. Overall, 58% of weekday bus-segments are medium or high (75% in the morning peak). The UI fades low-confidence stretches and shows confidence dots on every leg.

## 6. Reproducing
```
just fetch && just build      # data, Rust and web builds
just profiles                 # model from all of October, about 15 s
just validate                 # all runs plus the validation suite, about 3 min
just serve                    # API and web app at http://localhost:8090
```
Every assumption is in `crowding/config/model.dhall`. Any value can be overridden for one run, e.g. `--set trip_cv=0.5`.
