# Adopting the crowding model in an OpenTripPlanner 2.5-based stack

The crowding system only uses standard surfaces, so it fits into an existing realtime planning
stack without changes to that stack's own services:

| Surface | Used for |
|---|---|
| HTTP JSON API (`/v1/...`) | Any backend or app can show crowding next to its own journey results via `POST /v1/legs:annotate`. |
| OpenTripPlanner patch (`otp-crowding/patches`) | Crowding inside routing (`plan(crowdingReluctance:)`) and on every leg (`Leg.crowding`). |
| GTFS Realtime in | The live VehiclePositions feed: polled, or pushed to `POST /v1/live/vehicle-positions`. |
| GTFS Realtime out | `departure_occupancy_status` per stop (TripUpdates) and `occupancy_status` per bus (VehiclePositions), for any consumer that reads standard feeds. |
| Live tickets | HTTP `POST /v1/tickets` or a Kafka topic, in the published ticket shape. |

## Steps
1. **Timetable.** Load the current Delhi bus GTFS into both the planner and the crowding pipeline, from the same feed. Trip ids must match for the OTP crowding index and for leg annotation by `trip_id`.
2. **Model.** Keep `scripts/rt_collect.sh` (or any poller) saving the live VehiclePositions feed, and run `crowd-pipeline --rt <snapshots>` nightly or weekly on the latest weeks of tickets. The snapshots set how many buses actually run each route and hour. The service hot-reloads `snapshot.msgpack.zst` when it changes.
3. **OpenTripPlanner.**
   - Apply the patch to the 2.5 build: `otp-crowding/apply-patch.sh /path/to/OpenTripPlanner`. It applies cleanly to v2.5.0, and to forks that have not changed the same lines.
   - Enable the feature in `otp-config.json` with `{"otpFeatures": {"Crowding": true}}`.
   - Point OTP at the service: `OTP_CROWDING_INDEX_URL=http://crowd-service:8090/v1/otp/crowding-index`.
   - The index refreshes every `OTP_CROWDING_REFRESH_SECONDS` (default 300), which also brings in live adjustments. Routing never waits on the network inside a search. The search date's index is loaded once, when the request is mapped.
4. **Clients.**
   - Clients that call OTP directly read `Leg.crowding`. They can pass `crowdingReluctance` (0 = off, about 1 = the literature values, 2.5 = strongly avoid crowds).
   - Clients that use their own planner call `/v1/legs:annotate`.
   - Or use `/v1/journeys` as the front door. It adds the "leave later" and "most comfortable" alternatives.
5. **Realtime feeds.** Point the service at the live VehiclePositions feed (`rt.url`, key in the environment variable named by `rt.api_key_env`), or push it to `POST /v1/live/vehicle-positions` from an existing poller. An existing GTFS-RT producer can then serve `/gtfs-rt/vehicle-positions.pb` (the same feed with `occupancy_status` and `occupancy_percentage` filled) or copy `departure_occupancy_status` from `/gtfs-rt/trip-updates.pb`.
6. **Live tickets.** Publish ticket events to Kafka, rebuilding the service with the `kafka` feature, or POST them in batches. With no live feed the service serves the historical profile. The live adjustment is a statistical rule by default. Set `LIVE_MODEL=trees` to use the gradient-boosted trees, retraining them with `just validate` as tickets accumulate (technical note §5).
7. **Deployment.** The root `Dockerfile` builds one image with the service, the web app, the model and the patched OpenTripPlanner, and `deploy/kubernetes.yaml` runs it as a Deployment with a Service and an Istio VirtualService. The live-feed key goes in a Secret.

## Configuration
- **Service:** `crowding/config/service.dhall`, passed as `DHALL_CONFIG`. Every field has a default; environment overrides are `CROWD_SNAPSHOT`, `OTP_ENABLED`, `OTP_GRAPHQL_URL`, `CROWD_CLOCK`, `CROWD_WEB_DIR`, `RT_ENABLED`, `LIVE_MODEL` and `LIVE_TREES`. The live feed key is read from `DTS_API_KEY` and is never written to config.
- **Model:** `crowding/config/model.dhall`.

## Operations
- **Health and metrics:** `/health` (liveness), `/ready` (snapshot loaded), and `/metrics` in Prometheus format.
- **Logs:** structured, via `tracing`. Set `RUST_LOG=info`.
- **Footprint:** the service keeps everything in memory, about 1 GB for Delhi, and needs no database or cache. OpenTripPlanner with the Delhi graph needs about 3 GB more.
