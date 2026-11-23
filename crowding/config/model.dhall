-- Crowding model assumptions. Every number the model relies on is here.
-- Change a value, rerun `just profiles`, and compare.
let class =
      \(name : Text) ->
      \(label : Text) ->
      \(seats : Double) ->
      \(comfortable : Double) ->
      \(crush : Double) ->
        { name, label, seats, comfortable, crush }

let rule =
      \(route_prefix : Text) ->
      \(ac : Text) ->
      \(class : Text) ->
        { route_prefix, ac, class }

in  { bin_minutes = 30
    , first_minute = 240
    , last_minute = 1440
    -- Smoothing across neighbouring 30-minute bins.
    , kernel = [ 0.25, 0.5, 0.25 ]
    -- Sparse bins shrink toward a +/- 2 hour window of the same segment.
    , prior_window_bins = 4
    , shrink_pseudo_trips = 8.0
    -- Gandhi Jayanti and Dussehra 2026; both show Sunday-like ticket volumes.
    , holidays = [ "2026-10-02", "2026-10-20" ]
    -- Ticketed riders are expanded to all riders so that an average weekday matches
    -- the published city bus ridership. Low / mid / high bracket the uncertainty.
    , ridership =
      { weekday_boardings = { low = 3000000.0, mid = 4000000.0, high = 5000000.0 }
      , fixed_expansion = 0.0
      }
    -- Spread of load between buses of the same bin (coefficient of variation).
    , trip_cv = 0.35
    , confidence = { high_min_tickets = 40.0, medium_min_tickets = 12.0 }
    , capacity_classes =
      [ class "12m-ac" "12 m AC bus" 37.0 62.0 85.0
      , class "12m-nonac" "12 m non-AC bus" 40.0 65.0 90.0
      , class "9m-ev" "9 m electric bus" 23.0 38.0 55.0
      ]
    -- First matching rule wins.
    , capacity_rules =
      [ rule "D-" "any" "9m-ev", rule "" "nonac" "12m-nonac", rule "" "any" "12m-ac" ]
    -- In-vehicle time multipliers by crowding level.
    , multipliers = { seated = 1.0, standing = 1.4, crowded = 1.9 }
    , usually = 0.6
    , matching = { min_score = 82.0, geo_radius_m = 400.0 }
    -- Share of riders assumed to get off one stop early or late (sensitivity runs).
    , alight_spread = 0.0
    , trip_split_gap_minutes = 90
    -- Buses are counted from the tickets only: trips rebuilt from vehicle numbers,
    -- corrected for trips that sold no tagged ticket. The timetable only gives stop order
    -- and running times. "timetable" or "max" switch back to timetable supply.
    , supply_source = "tickets"
    , capture_cap = 3.0
    -- A bus cannot carry more than crush load. Where the city-wide expansion implies
    -- more on a route (its riders use tickets more than average), lower that route's
    -- expansion so its 99th percentile weekday load stays at crush. Loads shown are
    -- capped at crush capacity and flagged over_capacity.
    , capacity_check_quantile = 0.99
    -- Several route numbers are run by two operators, listed as two routes with their
    -- own stops. Merge them so tickets and buses of both are counted together.
    , merge_route_copies = True
    , merge_max_median_m = 300.0
    -- Tickets on a covered route whose boarding stop could not be placed still rode
    -- that route: scale the route up for them (at most 1.5x).
    , unmatched_correction_cap = 1.5
    -- The timetable needs ~8,900 buses at once at its peak; the tickets show ~5,400
    -- different buses running on these routes all day. Scale the timetable to the buses
    -- actually seen, per day type (ticket-based supply still applies where higher).
    , fleet_calibration = True
    , fleet_min_scale = 0.3
    }
