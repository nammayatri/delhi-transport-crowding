#!/usr/bin/env bash
# Save the live Delhi bus vehicle positions (GTFS Realtime) every N seconds, default 60,
# as data/raw/rt/<date>/<HHMMSS>.pb.gz. The pipeline uses these snapshots to calibrate
# how many buses actually run on each route (`--rt data/raw/rt`).
# Needs DTS_API_KEY (Delhi Transport Stack platform key); `just rt-collect` reads it from .env.
set -euo pipefail
: "${DTS_API_KEY:?set DTS_API_KEY}"
url="${RT_URL:-https://dts-backend.transportstack.in/api/dataset/otd/get-file?agency=delhi-buses&category=realtime_gtfs&filename=VehiclePositions.pb}"
every="${1:-60}"
out="${RT_DIR:-data/raw/rt}"
while true; do
  d=$(TZ=Asia/Kolkata date +%F)
  t=$(TZ=Asia/Kolkata date +%H%M%S)
  mkdir -p "$out/$d"
  if curl -fsS --max-time 30 -H "x-api-key: $DTS_API_KEY" "$url" -o "$out/$d/$t.pb.part"; then
    gzip -c "$out/$d/$t.pb.part" > "$out/$d/$t.pb.gz"
  fi
  rm -f "$out/$d/$t.pb.part"
  sleep "$every"
done
