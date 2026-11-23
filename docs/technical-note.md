# Crowding-aware bus journeys for Delhi: technical note

## What we built

We built three things:
- **A pipeline** that turns the October 2026 ticket records into an estimate of how many people are on board, by route, direction, stop-to-stop stretch, day type and 30-minute bin. The number of buses comes from the timetable, checked against the live GTFS Realtime feed.
- **A crowding service** that serves those estimates to commuters and operators: a journey planner, a network map with the live buses, operator tables, and standard GTFS Realtime feeds with occupancy filled in.
- **A patch for OpenTripPlanner 2.5** so the planner itself can weigh crowding when it searches, not just display it afterwards.

All numbers in this note come from `scripts/run_all_models.sh`. The detail is in `docs/results/`.

## 1. What the data is, and what that means for the method

| Fact from the data | Consequence |
|---|---|
| 2,559,899 tickets for 1–31 Oct. All are `ticket_type_id` 1, and 99% are at discounted digital fares (₹4.5 / 9 / 13.5 non-AC, ₹9 / 13.5 / 18 / 22.5 AC). | These are mobile/QR tickets: a sample of riders, not all of them. |
| About 102,300 ticketed riders on an average weekday, against roughly 4 million city bus boardings. | We need an **expansion factor**, about 39 at the central assumption. |
| 51% of tickets carry a vehicle number, but a rebuilt bus trip carries only **2.0 tagged tickets on average** (median 1). | A single trip's running count is too noisy to use. We keep the running count **for every ticket**, then pool it over every bus that runs in the same bin. |
| The static timetable (feed v32, October 2026) has 2,574 routes, 6,862 stops and 80,357 trips a day. Each route has full trips and, often, short-turn trips over part of it. | Tickets name only the route, so a route's patterns are merged (§2.2). |
| The live GTFS Realtime feed reports every bus about every 15 s: registration number (the same as on tickets), route, start time of its trip, and position. It has no occupancy. 99% of its buses run a route in the timetable, and 99% of them sold a ticket in October. | It shows how many buses actually run each route (§2.5), and where each bus is now (§2.10). |
| The tickets miss many buses. At 19:35 on a weekday, the live feed shows about 4,300 buses running a route; in October only about 2,200 trips under way at that time sold a tagged ticket. | Buses are not counted from the tickets (§2.5). |
| Oct 2 (Gandhi Jayanti) and Oct 20 (Dussehra) have Sunday-like volumes. | Both are treated as Sundays. Day types are weekday (20 days), Saturday (5) and Sunday/holiday (6). |

## 2. Method

### 2.1 Matching tickets to the timetable
**Route.** Route names are matched exactly after normalising: no spaces, `DWN` becomes `DOWN`, and a trailing variant digit such as `522BDOWN2` is dropped.

**Stops.** Each stop name is fuzzy-matched only against the stops of that ticket's own route. Both names are normalised first: abbreviations are expanded (Xing → crossing, Bdr → border, (T) → terminal) and alternate names such as "Saidul-A-Jab / Saket Metro Station" are split. The score blends token-set and token-sort similarity with a no-space comparison. If no stop on the route matches, a stop elsewhere with exactly the same name that lies within 400 m of the route is accepted.

**Direction.** If the two stops appear in reverse order on the ticket's route, the ticket is assigned to the opposite direction (3,873 tickets).

**Result.** 93.4% of tickets were placed:
- 2,273,707 exact matches
- 3,873 direction fixes
- 113,377 with the alighting stop imputed

The rest could not be placed:
- 1.7% are on routes absent from the timetable.
- 4.9% have a boarding stop that was not found on the route.

**Evidence the matching is right.** Trip length rises with every fare step:
| Fare paid | Median trip |
|---|---|
| AC ₹9 | 2.8 km |
| AC ₹13.5 | 5.9 km |
| AC ₹18 | 9.2 km |
| AC ₹22.5 | 16.3 km |
| Non-AC ₹4.5 | 2.8 km |
| Non-AC ₹9 | 6.8 km |
| Non-AC ₹13.5 | 15.9 km |

82% of matched trips fall inside the distance stage their fare implies. If stops were matched at random, distance would not track fare like this.

### 2.2 Full and short-turn patterns of a route
A route can have several stop patterns, such as full trips and short-turn trips that cover part of it. A ticket says only "8UP", and a stretch served by both carries the riders and the buses of both. So the patterns of one route name are merged onto the pattern that covers the others (most stops, then most trips). Every stop is mapped to the nearest stop on that canonical pattern, moving only forward; a short-turn pattern may start anywhere along it. The median distance between matched stops is 0 m. Tickets and buses of all patterns are then counted on the canonical pattern.

924 patterns were merged and none kept apart. The same step merges two operators' copies of a route number, which older feeds listed separately.

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

### 2.5 Bus supply: the timetable, checked against the live feed
How many buses pass a stretch in a half hour comes from the timetable, scaled per route and hour to the buses the live feed shows running.

1. **Collect.** `scripts/rt_collect.sh` saves the live VehiclePositions feed once a minute.
2. **Place each bus.** A bus is placed on the nearest stretch of its route, measured to the straight line between consecutive stops. It counts as **running** if it is within 500 m of its route, is not parked at the first stop waiting to start, and is not at the last stop having finished. 93% of buses with a known route are within 500 m of it.
3. **Compare with the timetable.** For each snapshot, the running buses on each route are compared with the timetabled trips of that route under way at the same moment.
4. **City factor per day type and hour.** Running buses divided by timetabled trips under way. An hour needs 5,000 trip-snapshots (two or three snapshots at the peak) to get its own factor. Otherwise it uses its day type's average, then the average of all snapshots.
5. **Route factor.** Each route's running buses against what the city factor predicts for it, shrunk toward 1 with 60 bus-snapshots (about one bus for an hour) and kept within ⅓–3.
6. **Supply.** Timetabled bus passes × city factor × route factor.

**What the feed showed.** Snapshots so far cover Monday 23 November, 19:38–21:25 (14 snapshots, 5,461 different buses running). Between 19:00 and 20:00, buses running match the timetable (factor 1.00). By 21:00 only 0.77 as many run as the timetable lists, and the average over all snapshots is 0.89. Route factors span 0.80–1.22 of the city factor between the 10th and 90th percentile.

**Why not count buses from the tickets.** Earlier versions did. The live feed shows the tickets miss about half the buses. Over a weekday, the buses used here cover 1.24M bus-km (straight line between stops). The timetable as published lists 1.40M, and buses rebuilt from tickets, even after correcting for trips that sold no tagged ticket, only 0.74M. Counting from tickets made every bus about 1.5× fuller (§4).

**Assumptions.** The live feed is from November and the tickets from October, so this assumes the same service in both months; the timetable is the same feed. Hours without snapshots use an average factor. The collector keeps running, and `just validate` recalibrates on every snapshot saved so far.

**Alternatives** for sensitivity runs: `supply_source = "timetable"` (as published), `"tickets"` (rebuilt trips with the capture correction, the earlier method) and `"max"`.

### 2.6 From ticketed riders to all riders
Tickets miss passholders, concession riders (including free travel for women), cash tickets and fare evasion. We scale up so that an average weekday matches published city bus ridership:

| Assumption | Weekday boardings | Expansion factor |
|---|---|---|
| Low | 3.0 M | 29.3 |
| Mid | 4.0 M | 39.1 |
| High | 5.0 M | 48.9 |

Every load is reported under all three.

Two route-level adjustments:
- **Unplaced tickets.** On routes we do cover, tickets whose boarding stop could not be placed still rode that route. The route is scaled up for them, at most 1.5×; the average is 1.06×.
- **Capacity consistency.** A bus cannot carry more than crush load. If the city-wide factor puts a route's 99th-percentile weekday load above crush, that route's riders evidently use tickets more than average, so its factor is lowered until it fits. This applies to 838 of the 1,979 routes with tickets. Displayed loads are also capped at crush capacity and flagged `over_capacity`.

### 2.7 Bus size and crowding levels
No fleet list was available, so the bus class comes from rules in `crowding/config/model.dhall`:

| Class | Seats | Comfortable up to | Crush |
|---|---|---|---|
| 12 m AC | 37 | 62 | 85 |
| 12 m non-AC (most of a route's tickets are non-AC) | 40 | 65 | 90 |
| 9 m electric (routes `D-…`) | 23 | 38 | 55 |

280 D- routes carry tickets and use the 9 m class.

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
At about 2 tickets per bus trip, live tickets say little about any single bus. They do say whether a route is busier than usual right now. While a ticket stream is flowing, the service compares a route's tickets over the last hour with the number expected for that route, day type and hour. It shrinks the ratio toward 1 (6 pseudo-tickets), clamps it to 0.5–2.5, and applies it to the next three hours.

Live tickets arrive over HTTP (`POST /v1/tickets`) or Kafka. `crowd-replay` replays any October day as a live stream.

### 2.10 Live bus positions
The service polls the live feed every 30 s (the API key comes from an environment variable) or accepts it pushed to `POST /v1/live/vehicle-positions`. Each bus is placed on its route and stretch as in §2.5. That gives three things:
- **Crowding on every bus now.** Each running bus gets the usual load for its stretch, day type and half hour (`GET /v1/live/vehicles`, and a layer on the network map).
- **Fewer buses than usual means fuller buses.** For each route, the buses running now are compared with the number usually running at this time (the calibrated timetable). The ratio usual / running, shrunk by one bus and kept within 0.67–1.5, scales riders per bus for trips from 30 minutes before now to an hour after. It is combined with the live-ticket factor and reaches the planner, the map and the OpenTripPlanner crowding index.
- **A standard feed.** `GET /gtfs-rt/vehicle-positions.pb` re-publishes the live feed with `occupancy_status` and `occupancy_percentage` filled for every running bus with an estimate, so any GTFS Realtime consumer can show crowding.

## 3. Checks against known patterns
| Check | Result |
|---|---|
| Weekday peaks | Load peaks in the 09:00 bin (28.7 riders per bus) and the 18:30 bin (19.6), with a midday low of 5.6 at 14:00. The raw counts peak in the same bins. At 09:00, 41% of riders are seated, 30% stand and 29% are in a crowded bus. In the morning peak, buses heading toward the centre carry 30.6 riders on average, against 19.8 heading away. |
| Weekday vs weekend | Saturday loads are 81% of weekday, Sunday/holiday 49%. |
| Direction | Travel toward Connaught Place is 1.55× heavier than away in the morning peak. Away is 1.77× heavier in the evening. All four distance rings agree. |
| Busiest corridors | Rainy Well ↔ Delhi Secretariat (ITO), Patel Nagar, Shivaji Park–Punjabi Bagh, and Madipur–SPM College. |
| How crowded | In the weekday morning peak (08:00–10:00), 26.2% of bus-km are standing room or crowded; across all sensitivity runs the share of peak bus-segments ranges 14.7–38.6%. In the evening peak (17:30–20:00) it is 13.7%. |
| Week to week | The model fitted on Oct 1–24 predicts Oct 25–31 with Spearman **0.91** over 157,530 weekday cells that have at least 10 test tickets. Levels agree on 81% of cells (κ 0.66), and route × hour totals correlate at 0.95. A route all-day average scores only 0.07, so time of day carries real signal. |

Figures are in `docs/figures/`.

## 4. Sensitivity
Share of weekday bus-segments whose level changes, compared with the main run:

| Run | Level changes |
|---|---|
| Ridership low (3 M) / high (5 M) | 5.6% / 6.2% |
| No capacity consistency | 12.5% |
| Buses counted from the tickets only | 10.2% |
| Timetable as published (no live-feed calibration) | 1.3% |
| Shrinkage weight 20 | 1.7% |
| Trip-to-trip CV 0.5 | 1.0% |
| 50% of riders get off one stop early or late | 0.6% |

The modelling details (smoothing, spread, alighting) matter little. What matters is how many riders there are and how many buses carry them: the capacity check, the bus count and the ridership assumption. Counting buses from tickets alone makes loads about 1.5× higher on the same stretches, which is why the live feed now sets the bus count. Every load comes with a low and a high value, and the map and dashboard let you switch scenarios.

## 5. Where the estimates are least reliable
- **Weekends.** 88% of Saturday and 92% of Sunday bus-segments are low confidence, because there are only 5–6 days of each.
- **North and outer Delhi.** In the north, north-east and west, and beyond 20 km from the centre (25% reliable), ticket density is low. The weakest busy areas include Rana Pratap Bagh, Ice Factory (Rani Jhansi Road), Chhajjupur and Narela Terminal. Coverage is best in the south, south-east and south-west. **Busy-corridor rankings therefore partly reflect where people use the ticketing app**, since one city-wide expansion factor cannot separate the two.
- **Bus supply.** So far the live feed has been sampled on one weekday evening. Other hours and day types use an average factor until more snapshots are saved, and the feed is from November while the tickets are from October.
- **Riders not placed.** 1.7% of tickets are on routes not in the timetable and 4.9% board at a stop not found on their route. They are scaled in per route, not placed on stretches.
- **Hours.** One expansion factor is used for every hour. Pass holders and free-travel riders are not in the tickets and probably ride more at the peaks, which would make peaks busier than shown.
- **Absolute levels** depend on the ridership assumption (§2.6) and on seat counts inferred without a fleet list.

Every cell carries a confidence of high, medium or low. Confidence is high with at least 40 ticketed riders behind the estimate, medium with at least 12, and is capped further when the bin rests on very few buses. Overall, 43% of weekday bus-segments are medium or high (66% in the morning peak). The UI fades low-confidence stretches and shows confidence dots on every leg.

## 6. Reproducing
```
just fetch && just build      # tickets, Rust and web builds (put the GTFS zip at data/raw/gtfs/)
just rt-collect               # save the live feed every minute (needs DTS_API_KEY in .env); leave running
just profiles                 # model from all of October, calibrated on the saved snapshots, about 20 s
just validate                 # all runs plus the validation suite, about 3 min
just serve                    # API and web app at http://localhost:8090, with live buses
```
Every assumption is in `crowding/config/model.dhall`. Any value can be overridden for one run, e.g. `--set trip_cv=0.5`.
