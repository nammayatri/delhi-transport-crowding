//! GTFS Realtime TripUpdates carrying occupancy per stop (no delays), for any consumer
//! that reads standard feeds. Only the message subset we fill is declared.

use actix_web::{get, web, HttpResponse};
use crowd_core::levels::OccupancyStatus;
use prost::Message;

use crate::api::{scenario, State, TimeQuery};
use crate::error::AppResult;

#[derive(Clone, PartialEq, Message)]
pub struct FeedMessage {
    #[prost(message, required, tag = "1")]
    pub header: FeedHeader,
    #[prost(message, repeated, tag = "2")]
    pub entity: Vec<FeedEntity>,
}

#[derive(Clone, PartialEq, Message)]
pub struct FeedHeader {
    #[prost(string, required, tag = "1")]
    pub gtfs_realtime_version: String,
    #[prost(int32, optional, tag = "2")]
    pub incrementality: Option<i32>,
    #[prost(uint64, optional, tag = "3")]
    pub timestamp: Option<u64>,
}

#[derive(Clone, PartialEq, Message)]
pub struct FeedEntity {
    #[prost(string, required, tag = "1")]
    pub id: String,
    #[prost(message, optional, tag = "3")]
    pub trip_update: Option<TripUpdate>,
}

#[derive(Clone, PartialEq, Message)]
pub struct TripUpdate {
    #[prost(message, required, tag = "1")]
    pub trip: TripDescriptor,
    #[prost(message, repeated, tag = "2")]
    pub stop_time_update: Vec<StopTimeUpdate>,
    #[prost(uint64, optional, tag = "4")]
    pub timestamp: Option<u64>,
}

#[derive(Clone, PartialEq, Message)]
pub struct TripDescriptor {
    #[prost(string, optional, tag = "1")]
    pub trip_id: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub start_date: Option<String>,
    #[prost(int32, optional, tag = "4")]
    pub schedule_relationship: Option<i32>,
    #[prost(string, optional, tag = "5")]
    pub route_id: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct StopTimeUpdate {
    #[prost(uint32, optional, tag = "1")]
    pub stop_sequence: Option<u32>,
    #[prost(string, optional, tag = "4")]
    pub stop_id: Option<String>,
    #[prost(int32, optional, tag = "5")]
    pub schedule_relationship: Option<i32>,
    /// `VehiclePosition.OccupancyStatus` for the departure from this stop.
    #[prost(int32, optional, tag = "7")]
    pub departure_occupancy_status: Option<i32>,
}

/// TripUpdates for trips running within the next `horizon` (default 2 h) of `at`/now.
#[get("/gtfs-rt/trip-updates.pb")]
pub async fn trip_updates(st: State, q: web::Query<TimeQuery>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let s = &l.snap;
    let when = st.resolve(&l, q.at.as_deref(), q.date.as_deref(), q.time.as_deref())?;
    let scen = scenario(q.scenario.as_deref(), &st.cfg.default_scenario)?;
    let active = l.active(when.date);
    let (t0, t1) = (when.secs.saturating_sub(30 * 60), when.secs + 2 * 3600);
    let date = when.date.format("%Y%m%d").to_string();
    let mut entity = Vec::new();
    let limit = q.limit.unwrap_or(usize::MAX);
    for (i, t) in s.trips.iter().enumerate() {
        if entity.len() >= limit {
            break;
        }
        if !active[i] || t.pattern == u32::MAX || t.times.is_empty() {
            continue;
        }
        let (first, last) = (t.times[0], *t.times.last().unwrap());
        if last < t0 || first > t1 {
            continue;
        }
        let p = &s.patterns[t.pattern as usize];
        if p.cells.is_none() {
            continue;
        }
        let lf = st.live_factor(&l, p.route, &when).factor;
        let mut stu = Vec::with_capacity(p.stops.len());
        for k in 0..p.stops.len() - 1 {
            let status = s
                .cell_at(t.pattern, k, when.dt, t.times[k], scen, lf)
                .map(|v| v.status)
                .unwrap_or(OccupancyStatus::NoDataAvailable);
            stu.push(StopTimeUpdate {
                stop_sequence: Some(k as u32 + 1),
                stop_id: Some(s.stops[p.stops[k] as usize].stop_id.clone()),
                schedule_relationship: Some(0),
                departure_occupancy_status: Some(status as i32),
            });
        }
        entity.push(FeedEntity {
            id: t.trip_id.clone(),
            trip_update: Some(TripUpdate {
                trip: TripDescriptor {
                    trip_id: Some(t.trip_id.clone()),
                    start_date: Some(date.clone()),
                    schedule_relationship: Some(0),
                    route_id: Some(s.routes[p.route as usize].route_id.clone()),
                },
                stop_time_update: stu,
                timestamp: Some(when.ts as u64),
            }),
        });
    }
    let msg = FeedMessage {
        header: FeedHeader {
            gtfs_realtime_version: "2.0".into(),
            incrementality: Some(0),
            timestamp: Some(when.ts as u64),
        },
        entity,
    };
    if q.format.as_deref() == Some("json") {
        let n = msg.entity.len();
        let sample: Vec<_> = msg
            .entity
            .iter()
            .take(5)
            .map(|e| {
                let tu = e.trip_update.as_ref().unwrap();
                serde_json::json!({
                    "trip_id": tu.trip.trip_id, "route_id": tu.trip.route_id,
                    "stops": tu.stop_time_update.iter().map(|u| (u.stop_id.clone(), u.departure_occupancy_status)).collect::<Vec<_>>(),
                })
            })
            .collect();
        return Ok(HttpResponse::Ok().json(serde_json::json!({"entities": n, "sample": sample})));
    }
    Ok(HttpResponse::Ok()
        .content_type("application/x-protobuf")
        .body(msg.encode_to_vec()))
}
