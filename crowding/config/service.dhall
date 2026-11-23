-- crowd-service configuration. Every field has a default; see src/config.rs.
{ bind = "0.0.0.0:8090"
, snapshot_path = env:CROWD_SNAPSHOT as Text ? "../data/out/full/snapshot.msgpack.zst"
, reload_secs = 30
, default_scenario = "mid"
, otp =
  { enabled = env:OTP_ENABLED ? False
  , graphql_url = env:OTP_GRAPHQL_URL as Text ? "http://localhost:8080/otp/gtfs/v1"
  , timeout_ms = 8000
  -- True when OTP carries the crowding patch (accepts crowdingReluctance).
  , crowding_patch = True
  , num_itineraries = 5
  }
, live =
  { enabled = True
  , window_secs = 3600
  , prior_tickets = 6.0
  , min_factor = 0.5
  , max_factor = 2.5
  -- "wall" or "replay" (now follows the latest ingested ticket, for demos).
  , clock = env:CROWD_CLOCK as Text ? "replay"
  , kafka = { enabled = False, brokers = "localhost:9092", topic = "tickets", group_id = "crowd-service" }
  }
, planner =
  { max_transfers = 2
  , max_access_m = 900.0
  , max_transfer_m = 350.0
  , later_departures_min = [ 10, 20, 30 ]
  , max_reluctance = 2.5
  , transfer_penalty_min = 5.0
  }
, cors_allow_any = True
, web_dir = env:CROWD_WEB_DIR as Text ? ""
}
