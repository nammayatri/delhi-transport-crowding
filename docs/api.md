# Crowding service API

Base URL `http://localhost:8090`. All responses are JSON unless stated. Errors return
`{"error": "...", "code": "BAD_REQUEST|NOT_FOUND|NOT_READY|UPSTREAM_ERROR|INTERNAL_ERROR"}`.

Time parameters, accepted by most endpoints:
- `at`: an ISO date-time such as `2026-10-28T18:00:00+05:30`.
- Or `date=YYYY-MM-DD` and `time=HH:MM`.
- `daytype` (`weekday|saturday|sunday`) overrides the day type for browsing.
- `scenario` (`low|mid|high`) picks the ridership assumption.

Without any of these, "now" is used. In replay mode, "now" follows the replayed tickets.

## Commuters
| Method | Path | What it returns |
|---|---|---|
| POST | `/v1/journeys` | Journey options with crowding per leg and labels (Fastest, Most comfortable, Leave later, seats likely). |
| POST | `/v1/legs:annotate` | Crowding for legs you already have, e.g. from your own planner. |
| GET | `/v1/stops?q=` | Stop and place search for the planner. |

`/v1/journeys` body:
```json
{"from": {"lat": 28.6469, "lon": 77.3159}, "to": {"stop_id": "15270"},
 "depart_at": "2026-10-28T18:00:00+05:30", "comfort": 0.5, "scenario": "mid", "engine": "auto"}
```
- `comfort` runs from 0 (fastest) to 1 (avoid crowds). It sets the crowding reluctance.
- `engine` is `otp`, `builtin` (RAPTOR over the snapshot) or `auto`.

Each option has:
- times, minutes, transfers and walking minutes;
- `worst_level` and a one-line `summary`;
- `crowding_extra_minutes` and `comfort_minutes`;
- `legs[]`. Each bus leg carries `crowding` with:
  - `summary`, `level_at_boarding`, `worst_level`, `worst_from`, `worst_to`, `confidence`;
  - `segments[]`: load, low/high, `p_seats`/`p_standing`/`p_crowded`, `level`, `usually`, `occupancy_status`, `confidence`, `over_capacity`.

`/v1/legs:annotate` body:
`{"legs": [{"trip_id": "1:142_12_50", "from_stop_id": "1:146", "to_stop_id": "1:160", "start_time": "..."}]}`.
- Feed prefixes are optional.
- Without a known `trip_id`, pass `route_id`; the nearest scheduled trip is used.

## Operators and maps
| Method | Path | What it returns |
|---|---|---|
| GET | `/v1/map/segments?daytype=&time=&scenario=` | GeoJSON: every stop-to-stop stretch with buses in that half hour. Gives load, level, `p_crowded`, buses per day, confidence and routes. |
| GET | `/v1/operator/hotspots?daytype=&from=HH:MM&to=HH:MM` | Stretches ranked by riders above comfortable load, with the extra buses needed at the peak. |
| GET | `/v1/operator/routes?daytype=&from=&to=` | Per route: peak load, where and when; share of bus-km crowded or standing; extra trips needed. |
| GET | `/v1/routes?q=` | Routes and patterns, flagged when there is data. |
| GET | `/v1/routes/{route_id}/profile?daytype=` | Heatmap data: load, level, confidence and buses per stretch per 30-minute bin, plus boardings per stop. |

## Live tickets
| Method | Path | What it does |
|---|---|---|
| POST | `/v1/tickets` | One ticket or an array, in the published shape (`passenger_count, amount, vehicle_number, ticket_type_id, start_stop_name, end_stop_name, route, created_at, is_ac`). Returns `{accepted, rejected}`. |
| GET | `/v1/live/status` | Clock, tickets seen, routes seen, and the latest live bus positions (feed time, buses, running). |
| GET | `/v1/live/routes/{route_id}` | Live factor for this route now: live tickets against the usual, times usual buses against buses running (`buses_running`, `buses_usual`, `supply_factor`). |

## Live bus positions
| Method | Path | What it does |
|---|---|---|
| GET | `/v1/live/vehicles?scenario=&route=` | GeoJSON points, one per bus in the latest GTFS Realtime VehiclePositions feed: `vehicle` (registration), `route`, `running`, the stretch it is on (`from`, `to`), and its expected `load`, `level`, `p_seats`/`p_standing`/`p_crowded` and `confidence`, with `buses_running_on_route` and `buses_usual_on_route`. |
| POST | `/v1/live/vehicle-positions` | Push a VehiclePositions feed (protobuf body, plain or gzip) instead of, or as well as, polling. |

The service polls the feed set in `rt.url` every `rt.poll_secs` (30 s), sending the key from the environment variable named in `rt.api_key_env` (`DTS_API_KEY`) as `x-api-key`. Without the key, polling stays off and pushed feeds still work.

Kafka: build with `--features kafka` and set `live.kafka.enabled`. Messages carry the same JSON.

## Feeds for other systems
| Method | Path | What it serves |
|---|---|---|
| GET | `/v1/otp/crowding-index?date=` | Binary per-trip cumulative crowding cost and levels for the OpenTripPlanner patch. Add `format=json` to inspect. |
| GET | `/gtfs-rt/trip-updates.pb?at=` | GTFS Realtime TripUpdates with `departure_occupancy_status` per stop, for trips running in the next 2 hours. Add `format=json` to inspect. |
| GET | `/gtfs-rt/vehicle-positions.pb` | The live VehiclePositions feed re-published with `occupancy_status` and `occupancy_percentage` (load / crush capacity) filled for every running bus with an estimate. Add `format=json` for counts. |

## Operations
| Path | Purpose |
|---|---|
| `/health` | Liveness. |
| `/ready` | 200 once the snapshot is loaded. |
| `/metrics` | Prometheus: requests, latency, live tickets, buses running in the live feed, snapshot load time, build info. |
| `/v1/meta` | Model metadata and assumptions. |

The snapshot file is watched and hot-reloaded when the pipeline rewrites it.
