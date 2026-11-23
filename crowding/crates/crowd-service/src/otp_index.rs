//! Crowding index for the OpenTripPlanner updater.
//!
//! Binary layout (big-endian, gzip-compressed), version 1:
//!
//! ```text
//! magic "CRWD" | u16 version | u16 flags (0) | i32 service date yyyymmdd | i64 generated unix s
//! u32 trip count, then per trip:
//!   u16 trip_id byte length | trip_id UTF-8 | u16 stop count n
//!   n x u32 cumulative crowding-extra seconds (C[0] = 0)
//!   (n-1) x u8 commuter level per segment (0 seats, 1 standing, 2 crowded, 255 no data)
//! ```
//!
//! Riding a trip from stop position i to j costs `C[j] - C[i]` extra seconds; the planner
//! multiplies that by the request's crowding reluctance.

use std::io::Write;

use actix_web::{get, web, HttpResponse};
use chrono::Datelike;
use crowd_core::levels::ScenarioPick;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::json;

use crate::api::{scenario, State, TimeQuery};
use crate::error::AppResult;
use crate::state::{AppState, Loaded, When};

pub struct TripEntry {
    pub trip_id: String,
    pub cum: Vec<u32>,
    pub levels: Vec<u8>,
}

pub fn entries(st: &AppState, l: &Loaded, when: &When, scen: ScenarioPick) -> Vec<TripEntry> {
    let active = l.active(when.date);
    let mut factors = rustc_hash::FxHashMap::default();
    let mut out = Vec::new();
    for (i, t) in l.snap.trips.iter().enumerate() {
        if !active[i]
            || t.pattern == u32::MAX
            || l.snap.patterns[t.pattern as usize].cells.is_none()
        {
            continue;
        }
        let route = l.snap.patterns[t.pattern as usize].route;
        let f = *factors
            .entry(route)
            .or_insert_with(|| st.live_factor(l, route, when).factor);
        let (cum, levels) = l.snap.trip_cumulative(i as u32, when.dt, scen, f);
        if levels.iter().all(|&x| x == 255) {
            continue;
        }
        out.push(TripEntry {
            trip_id: t.trip_id.clone(),
            cum,
            levels,
        });
    }
    out
}

pub fn encode(when: &When, entries: &[TripEntry]) -> std::io::Result<Vec<u8>> {
    let mut raw = Vec::with_capacity(entries.len() * 200);
    raw.extend_from_slice(b"CRWD");
    raw.extend_from_slice(&1u16.to_be_bytes());
    raw.extend_from_slice(&0u16.to_be_bytes());
    let d = when.date;
    let ymd = d.year() * 10_000 + d.month() as i32 * 100 + d.day() as i32;
    raw.extend_from_slice(&ymd.to_be_bytes());
    raw.extend_from_slice(&chrono::Utc::now().timestamp().to_be_bytes());
    raw.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for e in entries {
        let id = e.trip_id.as_bytes();
        raw.extend_from_slice(&(id.len() as u16).to_be_bytes());
        raw.extend_from_slice(id);
        raw.extend_from_slice(&(e.cum.len() as u16).to_be_bytes());
        for c in &e.cum {
            raw.extend_from_slice(&c.to_be_bytes());
        }
        raw.extend_from_slice(&e.levels);
    }
    let mut z = GzEncoder::new(Vec::new(), Compression::fast());
    z.write_all(&raw)?;
    z.finish()
}

#[get("/v1/otp/crowding-index")]
pub async fn crowding_index(st: State, q: web::Query<TimeQuery>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let when = st.resolve(&l, q.at.as_deref(), q.date.as_deref(), q.time.as_deref())?;
    let scen = scenario(q.scenario.as_deref(), &st.cfg.default_scenario)?;
    let st2 = st.clone();
    let l2 = l.clone();
    let es = web::block(move || entries(&st2, &l2, &when, scen))
        .await
        .map_err(anyhow::Error::from)?;
    if q.format.as_deref() == Some("json") {
        let limit = q.limit.unwrap_or(50);
        let sample: Vec<_> = es
            .iter()
            .take(limit)
            .map(|e| json!({"trip_id": e.trip_id, "cum": e.cum, "levels": e.levels}))
            .collect();
        return Ok(HttpResponse::Ok().json(
            json!({"date": when.date, "daytype": when.dt, "trips": es.len(), "sample": sample}),
        ));
    }
    let body = encode(&when, &es).map_err(anyhow::Error::from)?;
    Ok(HttpResponse::Ok()
        .content_type("application/octet-stream")
        .insert_header(("X-Crowding-Trips", es.len().to_string()))
        .insert_header(("X-Crowding-Date", when.date.to_string()))
        .body(body))
}
