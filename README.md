# Delhi bus crowding: model, API and crowding-aware journey planner

This project estimates how full Delhi's buses usually are, by route, direction, stop-to-stop stretch, day type and half hour. It starts from October 2026 e-ticket records and the static GTFS, and uses the live GTFS Realtime bus positions to count the buses that actually run. It then shows crowding next to every journey option and on every live bus, and lets the rider trade speed for comfort.

- **Method, assumptions, checks and limits:** [`docs/technical-note.md`](docs/technical-note.md)
- **Validation results:** [`docs/results/summary.md`](docs/results/summary.md), with figures in `docs/figures/`
- **Statistical model or machine learning:** technical note §5, with details in [`docs/results/ml_forecast.md`](docs/results/ml_forecast.md) and [`docs/results/nowcast_backtest.md`](docs/results/nowcast_backtest.md)
- **API:** [`docs/api.md`](docs/api.md)
- **Using it inside an OpenTripPlanner 2.5 stack:** [`docs/integration.md`](docs/integration.md)
- **Demo video (3 min):** [crowding-demo.mp4](../../releases/download/v1.0/crowding-demo.mp4), with scene notes in [`demo/README.md`](demo/README.md)

## Results at a glance
- **Known patterns hold.** Weekday loads peak at 09:00 and 18:30. Travel toward Connaught Place is 1.55× heavier than away in the morning, and away is 1.77× heavier in the evening. At 09:00, 41% of riders have a seat, 30% stand and 29% are in a crowded bus.
- **Stable week to week.** A model fitted on Oct 1–24 predicts Oct 25–31 with rank correlation 0.91, and the same crowding level on 81% of stretch-half-hours with at least 10 test tickets.
- **Statistical model or machine learning.** We tested the statistical estimate against gradient-boosted trees, ARIMA and the pretrained Chronos models:
  - Forecasting a whole week, it ties with the trees and gets the week's total riding right (1.01× actual, against 0.94× for the trees).
  - For the next hours, its city-wide live adjustment is the most accurate for the city's total.
  - Route by route, the trees forecast best, so they are available as an opt-in live model (`LIVE_MODEL=trees`).

## Layout
```
crowding/        Rust workspace
  crates/crowd-core       GTFS and GTFS Realtime, ticket matching, running load, supply, capacity, levels, live adjustment, planner
  crates/crowd-pipeline   tickets + GTFS -> snapshot + Parquet tables
  crates/crowd-service    API, live buses and live adjustment, GTFS-RT occupancy, OTP crowding index, journeys
  crates/crowd-replay     replay a day of tickets as a live stream (HTTP or Kafka)
  config/*.dhall          every model assumption and service setting
otp-crowding/    OpenTripPlanner 2.5 patch (crowding cost in RAPTOR, Leg.crowding) and Delhi configs
analysis/        Python validation: patterns, week-to-week, sensitivity, coverage, statistical model vs ML
web/             React + MapLibre: planner, network crowding map, operator dashboard
docs/            technical note, API, integration, results and figures
deploy/          Kubernetes manifest for the single image
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
just validate               # every model variant plus the validation suite (writes docs/results and the live trees)
just replay                 # stream Wed 28 Oct from 07:00 at 120x into the live adjustment
```
`LIVE_MODEL=trees just serve` uses the machine-learning live model instead of the statistical rule.

Crowding-aware routing in OpenTripPlanner:
```
otp-crowding/apply-patch.sh /path/to/OpenTripPlanner
mvn -Dprettier.skip=true -DskipTests package
java -Xmx5g -jar target/otp-2.5.0-shaded.jar --build --save <dir with otp-crowding/delhi/*.json, GTFS, OSM>
OTP_CROWDING_INDEX_URL=http://localhost:8090/v1/otp/crowding-index java -Xmx3g -jar ... --load --serve <dir>
OTP_ENABLED=True just serve                               # /v1/journeys then plans through OTP
```

## One image
The root `Dockerfile` builds a single image. It holds the crowding service, the web app, the model and the patched OpenTripPlanner with the Delhi graph. Build it after `just profiles` and `just validate`:
```
docker compose up --build                                  # http://localhost:8090
docker buildx build --platform linux/amd64 -t <registry>/delhi-transport-crowding .   # for a cluster
```
`deploy/kubernetes.yaml` runs it on Kubernetes as a Deployment with a Service and an Istio VirtualService that serves it under a sub-path (`/delhi-crowding/`) of an existing host. The pod needs about 4.5 GB of memory: OpenTripPlanner takes 3 GB and the service 1 GB.

## Tests
- `cd crowding && cargo test`:
  - unit tests: name matching, running load, shrinkage, capture correction, live adjustment (rule and trees), live feed decoding, bus placement and calibration, levels, comfort cost, planner;
  - API integration tests for every endpoint, on a fixture feed.
- `mvn -Dtest=CrowdingCostCalculatorTest test`, inside the patched OTP: the crowding cost on a crowded middle stretch, boarding after it, admissibility, and the index format.
