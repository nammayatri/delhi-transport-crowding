# Data (not committed)

| Path | Source |
|---|---|
| `raw/tickets/October-Delhi.csv` | Hackathon ticket records, 1–31 Oct 2026 (`just fetch` downloads the published zip) |
| `raw/gtfs/delhi-bus.gtfs.zip`, `raw/gtfs/delhi-bus/` | Delhi bus static GTFS from the Delhi Transport Stack platform (`delhi_buses_static_gtfs_v1.zip`, feed version v32, October 2026) |
| `raw/rt/<date>/<HHMMSS>.pb.gz` | Live Delhi bus VehiclePositions (GTFS Realtime) saved by `just rt-collect`. Needs a platform API key in `.env` as `DTS_API_KEY=...` (`.env` is git-ignored) |
| `raw/delhi.osm.pbf` | OpenStreetMap extract for Delhi (OpenTripPlanner street graph only) |
| `out/<run>/` | Pipeline outputs: `snapshot.msgpack.zst`, Parquet tables, `route_summary.csv`, `match_report.json`, `meta.json`, `rt_calibration.json` |
