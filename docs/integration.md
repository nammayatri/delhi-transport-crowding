# Adopting the crowding model in an OpenTripPlanner 2.5-based stack

The crowding system only uses standard surfaces, so it fits into an existing realtime planning
stack without changes to that stack's own services:

| Surface | Used for |
|---|---|
| HTTP JSON API (`/v1/...`) | Any backend or app can show crowding next to its own journey results via `POST /v1/legs:annotate`. |
| OpenTripPlanner patch (`otp-crowding/patches`) | Crowding inside routing (`plan(crowdingReluctance:)`) and on every leg (`Leg.crowding`). |
| GTFS Realtime | `departure_occupancy_status` per stop, for any consumer that reads standard feeds. |
| Live tickets | HTTP `POST /v1/tickets` or a Kafka topic, in the published ticket shape. |

## Steps
1. **Timetable.** Load the current Delhi bus GTFS into both the planner and the crowding pipeline, from the same feed. Trip ids must match for the OTP crowding index and for leg annotation by `trip_id`. A newer feed that includes the D- and OMS route families also gives estimates for the 13% of tickets that are unplaced today.
2. **Model.** Run `crowd-pipeline` nightly or weekly on the latest weeks of tickets. The service hot-reloads `snapshot.msgpack.zst` when it changes.
3. **OpenTripPlanner.**
   - Apply the patch to the 2.5 build: `otp-crowding/apply-patch.sh /path/to/OpenTripPlanner`. It applies cleanly to v2.5.0, and to forks that have not changed the same lines.
   - Enable the feature in `otp-config.json` with `{"otpFeatures": {"Crowding": true}}`.
   - Point OTP at the service: `OTP_CROWDING_INDEX_URL=http://crowd-service:8090/v1/otp/crowding-index`.
   - The index refreshes every `OTP_CROWDING_REFRESH_SECONDS` (default 300), which also brings in live adjustments. Routing never waits on the network inside a search. The search date's index is loaded once, when the request is mapped.
4. **Clients.**
   - Clients that call OTP directly read `Leg.crowding`. They can pass `crowdingReluctance` (0 = off, about 1 = the literature values, 2.5 = strongly avoid crowds).
   - Clients that use their own planner call `/v1/legs:annotate`.
   - Or use `/v1/journeys` as the front door. It adds the "leave later" and "most comfortable" alternatives.
5. **Realtime feeds.** An existing GTFS-RT producer can copy `departure_occupancy_status` from `/gtfs-rt/trip-updates.pb`, or call the API per trip and fill `occupancy_status` and `occupancy_percentage` on its vehicle positions.
6. **Live tickets.** Publish ticket events to Kafka, rebuilding the service with the `kafka` feature, or POST them in batches. With no live feed the service serves the historical profile.

## Configuration
- **Service:** `crowding/config/service.dhall`, passed as `DHALL_CONFIG`. Every field has a default; environment overrides are `CROWD_SNAPSHOT`, `OTP_ENABLED`, `OTP_GRAPHQL_URL`, `CROWD_CLOCK` and `CROWD_WEB_DIR`.
- **Model:** `crowding/config/model.dhall`.

## Operations
- **Health and metrics:** `/health` (liveness), `/ready` (snapshot loaded), and `/metrics` in Prometheus format.
- **Logs:** structured, via `tracing`. Set `RUST_LOG=info`.
- **Footprint:** the service keeps everything in memory, about 1 GB for Delhi. It needs no database or cache.
