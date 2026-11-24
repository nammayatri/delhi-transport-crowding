# Delhi bus crowding: model, API and crowding-aware journey planner

This project estimates how full Delhi's buses usually are, by route, direction, stop-to-stop stretch, day type and half hour. It starts from October 2026 e-ticket records and the static GTFS, and uses the live GTFS Realtime bus positions to count the buses that actually run. It then shows crowding next to every journey option and on every live bus, and lets the rider trade speed for comfort.

- **Method, assumptions, checks and limits:** [`docs/technical-note.md`](docs/technical-note.md)
- **Validation results:** [`docs/results/summary.md`](docs/results/summary.md), with figures in `docs/figures/`; a machine-learning comparison in [`docs/results/ml_forecast.md`](docs/results/ml_forecast.md)
- **API:** [`docs/api.md`](docs/api.md)
- **Using it inside an OpenTripPlanner 2.5 stack:** [`docs/integration.md`](docs/integration.md)
- **Demo video (3 min):** [crowding-demo.mp4](../../releases/download/v1.0/crowding-demo.mp4), with scene notes in [`demo/README.md`](demo/README.md)

## Layout
```
crowding/        Rust workspace
  crates/crowd-core       GTFS and GTFS Realtime, ticket matching, running load, supply, capacity, levels, planner
  crates/crowd-pipeline   tickets + GTFS -> snapshot + Parquet tables
  crates/crowd-service    API, live buses and nowcast, GTFS-RT occupancy, OTP crowding index, journeys
  crates/crowd-replay     replay a day of tickets as a live stream (HTTP or Kafka)
  config/*.dhall          every model assumption and service setting
otp-crowding/    OpenTripPlanner 2.5 patch (crowding cost in RAPTOR, Leg.crowding), Delhi configs, Dockerfile
analysis/        Python validation suite (patterns, week-to-week, sensitivity, coverage, ML comparison)
web/             React + MapLibre: planner, network crowding map, operator dashboard
docs/            technical note, API, integration, results and figures
scripts/         run every model variant plus validation; collect the live feed
```

## Quick start (local)
Prerequisites: Rust 1.82+, Node 20+, Python 3.11+, Java 21 and Maven (OTP only).

```
just fetch                  # tickets from the published link; put the GTFS zip at data/raw/gtfs/delhi-bus.gtfs.zip
just build                  # Rust release build and web app
echo DTS_API_KEY=... > .env  # Delhi Transport Stack key for the live bus feed (git-ignored)
just rt-collect             # save live bus positions every minute; leave it running
just profiles               # crowding model from all of October, buses calibrated on the live feed (about 20 s)
just serve                  # http://localhost:8090: planner, map with live buses, operator dashboard
just validate               # every model variant plus the validation suite (writes docs/results)
just replay                 # stream Wed 28 Oct from 07:00 at 120x into the live nowcast
```

Crowding-aware routing in OpenTripPlanner:
```
otp-crowding/apply-patch.sh /path/to/OpenTripPlanner     # or use otp-crowding/Dockerfile
mvn -Dprettier.skip=true -DskipTests package
java -Xmx10g -jar target/otp-2.5.0-shaded.jar --build --save <dir with otp-crowding/delhi/*.json, GTFS, OSM>
OTP_CROWDING_INDEX_URL=http://localhost:8090/v1/otp/crowding-index java -Xmx8g -jar ... --load --serve <dir>
OTP_ENABLED=True just serve                               # /v1/journeys then plans through OTP
```

## Docker
```
docker compose --profile build run --rm pipeline   # model into ./data/out/full
docker compose up crowd-service                    # http://localhost:8090
docker compose --profile otp up                    # plus patched OTP (set OTP_ENABLED=True)
docker compose --profile replay up replay          # live ticket replay
```

## Tests
- `cd crowding && cargo test`:
  - unit tests: name matching, running load, shrinkage, capture correction, live feed decoding, bus placement and calibration, levels, comfort cost, planner;
  - API integration tests for every endpoint, on a fixture feed.
- `mvn -Dtest=CrowdingCostCalculatorTest test`, inside the patched OTP: the crowding cost on a crowded middle stretch, boarding after it, admissibility, and the index format.
