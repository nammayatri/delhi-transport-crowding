# Data audit

Source: `data/raw/tickets/October-Delhi.csv` (2,559,899 rows, 2,753,606 passengers), matched by the
pipeline to the July 2025 static timetable (run `full`, Oct 1-31 2026).

## What the file is

- Every row has `ticket_type_id` = 1 (1 distinct value), and
  99.3% of tickets have a fare per passenger of Rs 4.5 / 9 / 13.5 / 18 / 22.5, i.e. the
  regular fares with a 10% discount. This is the digital (mobile/QR) ticket channel only, not all riders.
- Ticketed riders per weekday: 102,303 against an assumed city
  bus ridership of 4 million boardings (low 3M, high 5M). Expansion factor: 39.1
  (low 29.3, high 48.9); about 1 rider in 39 is in the file.
- 80.4% of tickets are on AC buses. 2,259 distinct route names.
- All timestamps carry +05:30 (share without: 0.00%). `created_at` is when the ticket
  was issued; the model uses it as the boarding time.
- 103 tickets have the same boarding and alighting stop name.

## Tickets per day

![tickets per day](../figures/audit_tickets_per_day.png)

| daytype | days | tickets/day | passengers/day |
| :--- | ---: | ---: | ---: |
| weekday | 20 | 95,782 | 102,303 |
| sunday | 6 | 44,540 | 50,216 |
| saturday | 5 | 75,405 | 81,251 |

Weekdays average 95,664 tickets in Oct 1-24 (training period) and 96,136 in Oct 25-31
(test week), so the test week is not unusual in volume. Holidays look like Sundays:
2026-10-02: 41,016, 2026-10-20: 49,375. The lowest ordinary weekdays were
2026-10-19 (74,801), 2026-10-06 (81,592), 2026-10-30 (91,896); Oct 19 is the eve of the Oct 20 holiday
and is kept as a weekday.

## Tickets by hour

![tickets by hour](../figures/audit_tickets_by_hour.png)

Weekday purchases peak in the 09:00 hour (13,499 per day) and the
18:00 hour (9,233 per day). 0.11% of tickets are
bought before 04:00, outside the modelled service day.

## Vehicle numbers

![vehicle share](../figures/audit_vehicle_share.png)

Daily share ranges from 48.0% to 85.2%. It is flat near half except for a jump on
2026-10-17 (65.0%), 2026-10-18 (85.2%), 2026-10-19 (61.8%): something in how vehicle
numbers were captured changed for those days (cause unknown from the data). Rebuilt trips and the capture
correction are pooled over all days of a day type, so the jump is diluted in the supply estimate, but it is
one reason the observed bus counts are not perfectly comparable day to day.

51.5% of tickets carry a vehicle number (AC 50.2%,
non-AC 56.8%); 6,809 distinct vehicles appear in the month,
about 6,000 per weekday. The pipeline rebuilds 507,187 bus trips from them, with
2.00 tagged tickets per trip on average, which is too few to read a load off a single trip. Loads
are therefore estimated by pooling running counts over all buses in a 30-minute bin.

## Passengers per ticket

| passenger_count | tickets | share | pax |
| ---: | ---: | ---: | ---: |
| 1 | 2,392,082 | 93.4% | 2,392,082 |
| 2 | 141,930 | 5.5% | 283,860 |
| 3 | 25,886 | 1.0% | 77,658 |
| 6 | 1 | 0.0% | 6 |

## Fares

| bus | fare_per_pax | tickets | share |
| :--- | ---: | ---: | ---: |
| AC | 9 | 838,086 | 32.7% |
| AC | 13.5 | 690,561 | 27.0% |
| AC | 18 | 274,722 | 10.7% |
| AC | 22.5 | 237,865 | 9.3% |
| non-AC | 4.5 | 189,711 | 7.4% |
| non-AC | 9 | 188,353 | 7.4% |
| non-AC | 13.5 | 122,741 | 4.8% |
| AC | 26.5 | 6,441 | 0.3% |
| AC | 22 | 4,656 | 0.2% |
| AC | 31 | 1,721 | 0.1% |
| AC | 13 | 1,597 | 0.1% |
| AC | 35.5 | 1,120 | 0.0% |

## Trip length against fare

![fare vs km](../figures/audit_fare_vs_km.png)

Only tickets with both stops matched are used (imputed alighting stops take their distance from the fare, so
they would make this check circular). Distances are the model's straight-line distances between consecutive
stops, which run slightly short of road distance.

| bus | fare (Rs/pax) | tickets | p25 km | median km | p75 km | assumed stage km | share in stage |
| :--- | ---: | ---: | ---: | ---: | ---: | :--- | ---: |
| non-AC | 4.5 | 139,459 | 2.0 | 2.8 | 3.6 | 0-4 | 83.5% |
| non-AC | 9 | 139,415 | 5.5 | 6.9 | 8.3 | 4-10 | 84.2% |
| non-AC | 13.5 | 90,536 | 12.4 | 16.6 | 22.9 | 10- | 89.5% |
| AC | 9 | 544,984 | 2.1 | 2.8 | 3.7 | 0-4 | 83.0% |
| AC | 13.5 | 469,783 | 4.9 | 6.0 | 7.1 | 4-8 | 78.9% |
| AC | 18 | 190,502 | 8.1 | 9.4 | 10.6 | 8-12 | 69.1% |
| AC | 22.5 | 169,043 | 13.5 | 16.7 | 21.5 | 12- | 86.8% |

Median trip length rises with every fare step for both bus types (monotonic: yes), and
81.2% of matched tickets fall inside the distance stage that their fare implies
(assumed stages: non-AC Rs 5 up to 4 km, Rs 10 for 4-10 km, Rs 15 beyond; AC Rs 10 / 15 / 20 / 25 for
0-4 / 4-8 / 8-12 / 12+ km, full fares before the 10% discount). Stop matching that was wrong would scatter
trip lengths independently of the fare; it does not. The non-AC Rs 13.5 and AC Rs 22.5 bands are open-ended,
which is why their spread is wide.

## Match outcome

![match outcome](../figures/audit_match_outcome.png)

| outcome | tickets | share |
| :--- | ---: | ---: |
| both stops matched | 1,752,742 | 68.5% |
| matched after flipping direction | 8,100 | 0.3% |
| boarding stop matched, alighting imputed | 229,574 | 9.0% |
| route not in 2025 timetable | 327,518 | 12.8% |
| boarding stop not found on route | 241,965 | 9.5% |

Of the matched tickets, 8.3% needed the geographic fallback (a same-named
stop within 400 m of a stop on the route).

## Routes missing from the timetable

![unknown routes](../figures/audit_unknown_routes.png)

Ticket route names that never matched (approximation of "route not in timetable": 327,854 tickets):

| family | tickets | routes | share |
| :--- | ---: | ---: | ---: |
| D- family | 161,921 | 291 | 49.4% |
| other | 146,161 | 599 | 44.6% |
| OMS family | 19,772 | 6 | 6.0% |

The D- and OMS families make up 55.4% of these tickets; the rest are 599
other names, led by 801-IGI-INDDOWN, 801-IND-IGIUP, 874EUP, 874EDOWN, 801STL-DW-2UP, which the July 2025
feed does not list under those names. Their riders are not in the model at all:
no crowding estimate exists for those buses, and the corridors they serve look emptier than they are.

## Index check

The model's ticket index matches the CSV row order for 100.00% of matched tickets (route name
compared), so raw rows and model outputs can be joined safely.
