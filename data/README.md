# Data (not committed)

| Path | Source |
|---|---|
| `raw/tickets/October-Delhi.csv` | Hackathon ticket records, 1–31 Oct 2026 (`just fetch` downloads the published zip) |
| `raw/gtfs/delhi-bus.gtfs.zip`, `raw/gtfs/delhi-bus/` | Delhi bus static GTFS. Use the feed from the Delhi Transport Stack platform; the runs in `docs/` used a July 2025 feed |
| `raw/delhi.osm.pbf` | OpenStreetMap extract for Delhi (OpenTripPlanner street graph only) |
| `out/<run>/` | Pipeline outputs: `snapshot.msgpack.zst`, Parquet tables, `route_summary.csv`, `match_report.json`, `meta.json` |
