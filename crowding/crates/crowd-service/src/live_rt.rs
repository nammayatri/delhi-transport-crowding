//! Live bus positions from a GTFS Realtime VehiclePositions feed.
//!
//! The feed is polled (or pushed to `POST /v1/live/vehicle-positions`). Each bus is placed
//! on its route and stretch. That gives:
//! - expected crowding on every bus now (`GET /v1/live/vehicles`), and the same feed
//!   re-published with `occupancy_status` filled (`GET /gtfs-rt/vehicle-positions.pb`);
//! - the number of buses running each route now, against the usual number, which scales
//!   riders per bus for trips planned around now (see `AppState::live_factor`).

use std::sync::Arc;
use std::time::Duration;

use actix_web::{get, post, web, HttpResponse};
use crowd_core::levels::OccupancyStatus;
use crowd_core::rt::{self, Feed, FeedMessage};
use prost::Message;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{scenario, State};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, Loaded, When};

/// One bus from the feed, placed on its route.
#[derive(Debug, Clone)]
pub struct LiveBus {
    /// Feed entity id.
    pub id: String,
    pub label: String,
    pub route_id: String,
    /// Route in the snapshot (canonical copy), if the feed's route is known.
    pub route: Option<u32>,
    pub pattern: Option<u32>,
    pub seg: usize,
    /// Running a trip on its route (not waiting at a terminal, not off route, fresh).
    pub running: bool,
    pub lat: f64,
    pub lon: f64,
    pub speed: f32,
    pub ts: i64,
}

pub struct LiveFleet {
    /// Unix time of the feed and of when it was received.
    pub feed_ts: i64,
    pub received: i64,
    pub buses: Vec<LiveBus>,
    /// Buses running each route.
    pub running_by_route: FxHashMap<u32, u32>,
    pub message: FeedMessage,
}

/// Place every bus of a feed on the snapshot's routes.
pub fn build_fleet(l: &Loaded, feed: Feed, received: i64) -> LiveFleet {
    let mut running_by_route: FxHashMap<u32, u32> = FxHashMap::default();
    let buses = feed
        .vehicles
        .into_iter()
        .map(|v| {
            let route = l
                .route_by_id
                .get(v.route_id.as_str())
                .map(|&r| l.route_canon[r as usize]);
            let fresh = feed.ts - v.ts <= rt::MAX_AGE_SECS;
            let loc = route.and_then(|r| {
                let pats = l.patterns_by_route[r as usize]
                    .iter()
                    .map(|&p| (p, l.snap.patterns[p as usize].stops.as_slice()));
                rt::locate(&l.snap.stops, pats, v.lat, v.lon, v.speed)
            });
            let running = fresh && loc.is_some_and(|x| x.in_service);
            if running {
                *running_by_route.entry(route.unwrap()).or_default() += 1;
            }
            LiveBus {
                id: v.id,
                label: v.label,
                route_id: v.route_id,
                route,
                pattern: loc.map(|x| x.pattern),
                seg: loc.map(|x| x.seg).unwrap_or(0),
                running,
                lat: v.lat,
                lon: v.lon,
                speed: v.speed,
                ts: v.ts,
            }
        })
        .collect();
    LiveFleet {
        feed_ts: feed.ts,
        received,
        buses,
        running_by_route,
        message: feed.message,
    }
}

fn install(st: &AppState, bytes: &[u8]) -> anyhow::Result<usize> {
    let l = st.snap().map_err(|e| anyhow::anyhow!("{e}"))?;
    let feed = rt::decode(bytes)?;
    let fleet = build_fleet(&l, feed, chrono::Utc::now().timestamp());
    let n = fleet.buses.len();
    let running = fleet.running_by_route.values().sum::<u32>();
    st.metrics.live_buses.set(running as f64);
    st.fleet.store(Some(Arc::new(fleet)));
    Ok(n)
}

/// Poll the configured feed until the process ends.
pub async fn run(st: Arc<AppState>) {
    let c = st.cfg.rt.clone();
    if !c.enabled || c.url.is_empty() {
        return;
    }
    let key = if c.api_key_env.is_empty() {
        None
    } else {
        match std::env::var(&c.api_key_env) {
            Ok(k) if !k.trim().is_empty() => Some(k.trim().to_string()),
            _ => {
                tracing::warn!(
                    var = %c.api_key_env,
                    "live bus positions off: API key variable not set"
                );
                return;
            }
        }
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
        .expect("http client");
    let mut tick = tokio::time::interval(Duration::from_secs(c.poll_secs.max(5)));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if st.loaded.load().is_none() {
            continue;
        }
        let mut req = http.get(&c.url);
        if let Some(k) = &key {
            req = req.header("x-api-key", k);
        }
        let bytes = match req.send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => match r.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(error = %e, "live feed read failed");
                    continue;
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "live feed request failed");
                continue;
            }
        };
        let st2 = st.clone();
        match tokio::task::spawn_blocking(move || install(&st2, &bytes)).await {
            Ok(Ok(n)) => tracing::debug!(buses = n, "live feed updated"),
            Ok(Err(e)) => tracing::warn!(error = %e, "live feed not usable"),
            Err(e) => tracing::warn!(error = %e, "live feed task failed"),
        }
    }
}

/// Push a VehiclePositions feed (protobuf body), e.g. from another poller or a test.
#[post("/v1/live/vehicle-positions")]
pub async fn post_vehicle_positions(st: State, body: web::Bytes) -> AppResult<HttpResponse> {
    let n = install(&st, &body).map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(HttpResponse::Ok().json(json!({ "buses": n })))
}

#[derive(Debug, Deserialize, Default)]
pub struct LiveQuery {
    pub scenario: Option<String>,
    pub route: Option<String>,
    pub format: Option<String>,
}

struct BusView {
    level: Option<&'static str>,
    status: OccupancyStatus,
    load: Option<f64>,
    crush: f64,
    props: Value,
}

fn view_bus(
    st: &AppState,
    l: &Loaded,
    f: &LiveFleet,
    b: &LiveBus,
    when: &When,
    scen: crowd_core::levels::ScenarioPick,
) -> BusView {
    let route_name = b
        .route
        .map(|r| l.snap.routes[r as usize].name().to_string());
    let lf = b.route.map(|r| st.live_factor(l, r, when));
    let cell = match (b.running, b.pattern) {
        (true, Some(p)) => l.snap.cell_at(
            p,
            b.seg,
            when.dt,
            when.secs,
            scen,
            lf.map(|x| x.factor).unwrap_or(1.0),
        ),
        _ => None,
    };
    let stretch = b.pattern.map(|p| {
        let s = &l.snap.patterns[p as usize].stops;
        let n = |i: usize| l.snap.stops[s[i.min(s.len() - 1)] as usize].name.clone();
        (n(b.seg), n(b.seg + 1))
    });
    let crush = b
        .pattern
        .map(|p| l.snap.class_of(p).crush)
        .unwrap_or(l.snap.config.capacity_classes[0].crush);
    let seats = b.pattern.map(|p| l.snap.class_of(p).seats);
    let props = json!({
        "vehicle": b.label,
        "route": route_name,
        "route_id": b.route_id,
        "running": b.running,
        "level": cell.map(|c| c.level.as_str()),
        "load": cell.map(|c| (c.load * 10.0).round() / 10.0),
        "seats": seats,
        "p_seats": cell.map(|c| c.p_seats),
        "p_standing": cell.map(|c| c.p_standing),
        "p_crowded": cell.map(|c| c.p_crowded),
        "confidence": cell.map(|c| c.confidence.as_str()),
        "from": stretch.as_ref().map(|s| s.0.clone()),
        "to": stretch.as_ref().map(|s| s.1.clone()),
        "speed_kmh": (b.speed as f64 * 3.6).round(),
        "age_s": (f.feed_ts - b.ts).max(0),
        "buses_running_on_route": lf.and_then(|x| x.buses_live),
        "buses_usual_on_route": lf.and_then(|x| x.buses_usual).map(|u| (u * 10.0).round() / 10.0),
        "live_factor": lf.map(|x| x.factor),
    });
    BusView {
        level: cell.map(|c| c.level.as_str()),
        status: cell
            .map(|c| c.status)
            .unwrap_or(OccupancyStatus::NoDataAvailable),
        load: cell.map(|c| c.load),
        crush,
        props,
    }
}

fn fleet_when(l: &Loaded, f: &LiveFleet) -> When {
    let (date, secs) = rt::local(f.feed_ts);
    When {
        date,
        secs,
        ts: f.feed_ts,
        dt: l.cal.day_type(date),
    }
}

/// Every bus in the latest feed with its expected crowding now, as GeoJSON points.
#[get("/v1/live/vehicles")]
pub async fn live_vehicles(st: State, q: web::Query<LiveQuery>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let scen = scenario(q.scenario.as_deref(), &st.cfg.default_scenario)?;
    let Some(f) = st.fleet.load_full() else {
        return Ok(HttpResponse::Ok().json(json!({
            "type": "FeatureCollection", "features": [], "feed_time": null,
            "note": "no live feed yet",
        })));
    };
    let when = fleet_when(&l, &f);
    let mut features = Vec::with_capacity(f.buses.len());
    let mut counts: FxHashMap<&str, u32> = FxHashMap::default();
    for b in &f.buses {
        if q.route.as_deref().is_some_and(|r| {
            r != b.route_id
                && b.route
                    .is_none_or(|ri| l.snap.routes[ri as usize].name() != r)
        }) {
            continue;
        }
        let v = view_bus(&st, &l, &f, b, &when, scen);
        *counts
            .entry(
                v.level
                    .unwrap_or(if b.running { "no_data" } else { "not_running" }),
            )
            .or_default() += 1;
        features.push(json!({
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [b.lon, b.lat]},
            "properties": v.props,
        }));
    }
    let feed_time = chrono::DateTime::from_timestamp(f.feed_ts, 0)
        .map(|t| t.with_timezone(&crowd_core::tickets::ist()).to_rfc3339());
    Ok(HttpResponse::Ok().json(json!({
        "type": "FeatureCollection",
        "feed_time": feed_time,
        "daytype": when.dt,
        "buses": f.buses.len(),
        "running": f.running_by_route.values().sum::<u32>(),
        "by_level": counts,
        "features": features,
    })))
}

/// The live VehiclePositions feed with `occupancy_status` and `occupancy_percentage`
/// filled from the model for every running bus.
#[get("/gtfs-rt/vehicle-positions.pb")]
pub async fn vehicle_positions(st: State, q: web::Query<LiveQuery>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let scen = scenario(q.scenario.as_deref(), &st.cfg.default_scenario)?;
    let f = st
        .fleet
        .load_full()
        .ok_or_else(|| AppError::NotFound("no live feed yet".into()))?;
    let when = fleet_when(&l, &f);
    let by_id: FxHashMap<&str, &LiveBus> = f.buses.iter().map(|b| (b.id.as_str(), b)).collect();
    let mut msg = f.message.clone();
    let mut filled = 0usize;
    for e in &mut msg.entity {
        let (Some(v), Some(b)) = (e.vehicle.as_mut(), by_id.get(e.id.as_str())) else {
            continue;
        };
        let bv = view_bus(&st, &l, &f, b, &when, scen);
        if let Some(load) = bv.load {
            v.occupancy_status = Some(bv.status as i32);
            v.occupancy_percentage =
                Some((100.0 * load / bv.crush).round().clamp(0.0, 100.0) as u32);
            filled += 1;
        }
    }
    if q.format.as_deref() == Some("json") {
        return Ok(HttpResponse::Ok().json(json!({
            "entities": msg.entity.len(), "with_occupancy": filled,
            "feed_timestamp": msg.header.timestamp,
        })));
    }
    Ok(HttpResponse::Ok()
        .content_type("application/x-protobuf")
        .body(msg.encode_to_vec()))
}
