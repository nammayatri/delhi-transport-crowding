# Coverage: where estimates are least reliable

A cell's confidence is set by the kernel-weighted number of ticketed riders behind it: high >=
40, medium >= 12, otherwise low (summed over all days of
the day type in October). Low-confidence cells are mostly the prior: the route's level and the city-wide
time-of-day shape. Shares below are of bus-segments (each bus passing each segment once).

## By day type

| daytype | high | medium | low |
| :--- | ---: | ---: | ---: |
| weekday | 17.8% | 25.3% | 56.9% |
| saturday | 1.3% | 10.4% | 88.3% |
| sunday | 0.6% | 7.1% | 92.4% |

Weekends rest on 5 Saturdays and 6 Sundays/holidays and far fewer
tickets per day, so 88.3% of Saturday and 92.4% of Sunday bus-segments are low confidence.
Weekend crowding should be shown as indicative only.

## Weekday, by time band

| band | bus_segments | high | medium | low | tickets_per_bus_segment |
| :--- | ---: | ---: | ---: | ---: | ---: |
| early 04:00-08:00 | 270,594 | 11.1% | 23.2% | 65.7% | 0.410 |
| AM peak 08:00-10:00 | 293,482 | 39.4% | 26.7% | 34.0% | 1.145 |
| midday 10:00-17:30 | 963,681 | 11.4% | 25.6% | 63.0% | 0.380 |
| PM peak 17:30-20:00 | 343,583 | 27.3% | 28.8% | 43.9% | 0.746 |
| late 20:00-24:00 | 336,116 | 13.0% | 21.4% | 65.6% | 0.449 |

`tickets_per_bus_segment` = ticketed riders crossing per bus pass per weekday. Among bins with at least a
quarter of the peak bus count, the least covered are 23:00 (91.2% low), 22:30 (83.3% low), 05:30 (82.9% low), 14:00 (76.0% low), 13:30 (75.5% low);
the best covered are 09:00 (30.3% low), 09:30 (32.0% low), 08:30 (33.4% low). Early morning,
midday and late evening estimates lean on the prior; the peaks are where the data is.

## Where

![coverage map](../figures/coverage_map.png)

Share of weekday bus-segments with medium or high confidence, by distance and direction from Connaught Place:

| ring | bus_segments | reliable | tickets_per_bus_segment |
| :--- | ---: | ---: | ---: |
| 0-5 km | 314,109 | 39.2% | 0.501 |
| 5-10 km | 737,281 | 46.8% | 0.538 |
| 10-20 km | 913,217 | 46.3% | 0.647 |
| 20+ km | 242,851 | 25.0% | 0.309 |

| sector | bus_segments | reliable | tickets_per_bus_segment |
| :--- | ---: | ---: | ---: |
| N | 204,034 | 33.1% | 0.380 |
| NE | 180,889 | 31.7% | 0.316 |
| E | 200,608 | 45.6% | 0.455 |
| SE | 166,132 | 54.5% | 0.793 |
| S | 255,339 | 58.0% | 0.945 |
| SW | 177,958 | 59.2% | 0.971 |
| W | 454,136 | 38.1% | 0.450 |
| NW | 568,362 | 38.3% | 0.428 |

| ring | N | NE | E | SE | S | SW | W | NW |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0-5 km | 23.9% | 32.2% | 47.1% | 50.3% | 47.9% | 43.4% | 48.2% | 37.0% |
| 5-10 km | 34.5% | 32.3% | 51.1% | 52.7% | 51.3% | 61.7% | 54.8% | 42.5% |
| 10-20 km | 39.4% | 30.1% | 30.6% | 58.1% | 66.3% | 64.1% | 37.7% | 43.1% |
| 20+ km | 38.5% |  |  | 25.8% | 34.6% | 42.7% | 26.1% | 20.6% |

The SW, S, SE sectors are best covered; the NE, N, W sectors are worst. Coverage also thins
beyond 20 km (25.0% reliable) and in the central 5 km
(39.2%), where ticketed riders per bus pass are also lower
(0.501 against
0.647 in the best ring). Confidence is per route, so a central corridor
served by several lightly ticketed routes can be busy overall while each route's cells stay low confidence.

### Least reliable busy areas

2 km grid cells in the top 30% by weekday bus-segments, ranked by the share of their bus-segments with
medium or high confidence (lowest first). 0 of 120 busy grid cells
have no medium- or high-confidence bus-segment at all.

| area_stop | where | routes | bus_segments | reliable | tickets_per_bus_segment |
| :--- | :--- | ---: | ---: | ---: | ---: |
| Rana Pratap Bagh | 7 km N of CP | 111 | 12,822 | 17.4% | 0.247 |
| Ice Factory (Rani Jhansi Road) | 4 km N of CP | 179 | 20,409 | 17.6% | 0.234 |
| Chhajjupur | 10 km NE of CP | 53 | 10,531 | 18.1% | 0.173 |
| Narela Terminal | 27 km NW of CP | 79 | 10,527 | 19.4% | 0.228 |
| Shivam Plywood | 23 km NW of CP | 98 | 7,136 | 20.1% | 0.242 |
| Nanak Piyau | 24 km W of CP | 154 | 16,857 | 24.2% | 0.289 |
| ISBT Nityanand Marg | 4 km N of CP | 386 | 40,056 | 24.8% | 0.292 |
| Bawana School | 25 km NW of CP | 96 | 7,255 | 25.3% | 0.318 |
| Mundka Village Crossing | 19 km W of CP | 121 | 11,474 | 27.0% | 0.284 |
| Bhajanpura | 10 km NE of CP | 96 | 13,605 | 27.1% | 0.281 |
| Bank Colony | 12 km NE of CP | 94 | 17,676 | 27.2% | 0.266 |
| DPS Matiyala (Delhi Public School ) | 17 km W of CP | 88 | 7,658 | 27.9% | 0.461 |

In words: the weakest areas are places with plenty of buses but few app tickets, named here by their
busiest stop: Rana Pratap Bagh (7 km N of CP); Ice Factory (Rani Jhansi Road) (4 km N of CP); Chhajjupur (10 km NE of CP); Narela Terminal (27 km NW of CP); Shivam Plywood (23 km NW of CP); Nanak Piyau (24 km W of CP); ISBT Nityanand Marg (4 km N of CP); Bawana School (25 km NW of CP).

## Problems the confidence label does not capture

1. **Riders the model cannot place.** 1.7% of tickets are on routes missing
   from the timetable and 4.9% have a boarding stop not found on their route.
   Those buses have no estimate at all, and corridors shared with them look emptier than they are. Another
   4.4% of tickets have an imputed alighting stop. 615 of
   3,498 timetable patterns have no matched ticket.
2. **Duplicate route names.** No route name appears twice in this timetable, so a bus cannot be matched to two copies of its route. Each route's full and short-turn patterns share stops and are merged onto the pattern that covers them (924 patterns merged).
3. **Bus counts.** Buses come from the timetable scaled to the live feed (14 snapshots, 2026-11-23 19:38 to 2026-11-23 21:25). The feed is from 2026-11-23, the tickets from October: this assumes the same service. Hours without snapshots use the average factor of their day type. Route factors (buses running against the city-wide expectation) span 0.80-1.22 (10th-90th percentile, median 0.99), and 92.6% of buses with a known route were within 500 m of it.
4. **Tickets-only bus counts are not used.** On 111 routes nearly every bus trip rebuilt
   from tickets carries a single tagged ticket, so the number of trips the tickets miss cannot be estimated from
   the tickets themselves. This is why the bus count now comes from the timetable and the live feed; the
   tickets-only count is kept as the `sens_tickets` sensitivity run.
5. **Implausible loads with high confidence.** 0 weekday cells exceed crush capacity,
   0 of them "high confidence" (on 0 routes). These are capped at crush and
   flagged. Confidence counts tickets, not the reliability of the bus count they are divided by.
