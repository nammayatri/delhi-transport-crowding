//! REST handlers.

use std::sync::Arc;

use actix_web::{get, post, web, HttpResponse};
use crowd_core::daytype::DayType;
use crowd_core::levels::{CellView, Confidence, Level, ScenarioPick};
use crowd_core::names::norm_stop;
use crowd_core::snapshot::LegCrowding;
use crowd_core::tickets::{ist, Interner, RawTicket};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::state::{AppState, Loaded, When};

pub type State = web::Data<AppState>;

#[derive(Debug, Deserialize, Default)]
pub struct TimeQuery {
    pub at: Option<String>,
    pub date: Option<String>,
    pub time: Option<String>,
    pub daytype: Option<String>,
    pub scenario: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<usize>,
    pub q: Option<String>,
    pub format: Option<String>,
}

pub fn scenario(s: Option<&str>, default: &str) -> AppResult<ScenarioPick> {
    match s.unwrap_or(default) {
        "low" => Ok(ScenarioPick::Low),
        "mid" => Ok(ScenarioPick::Mid),
        "high" => Ok(ScenarioPick::High),
        o => Err(AppError::BadRequest(format!(
            "scenario must be low, mid or high, not {o}"
        ))),
    }
}

fn daytype(q: &TimeQuery, when: &When) -> AppResult<DayType> {
    match q.daytype.as_deref() {
        Some(d) => {
            DayType::parse(d).ok_or_else(|| AppError::BadRequest(format!("bad daytype {d}")))
        }
        None => Ok(when.dt),
    }
}

fn hhmm_to_secs(s: &str) -> AppResult<u32> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| AppError::BadRequest(format!("bad time {s}")))?;
    let h: u32 = h
        .parse()
        .map_err(|_| AppError::BadRequest(format!("bad time {s}")))?;
    let m: u32 = m
        .parse()
        .map_err(|_| AppError::BadRequest(format!("bad time {s}")))?;
    Ok(h * 3600 + m * 60)
}

pub fn hhmm(secs: u32) -> String {
    let s = secs % 86_400;
    format!("{:02}:{:02}", s / 3600, (s / 60) % 60)
}

/// One-line description a commuter can read in a second.
pub fn describe(level: Level, usually: bool) -> &'static str {
    match (level, usually) {
        (Level::Seats, true) => "Seats usually available",
        (Level::Seats, false) => "Seats often available",
        (Level::Standing, true) => "Usually standing room only",
        (Level::Standing, false) => "Often standing room only",
        (Level::Crowded, true) => "Usually crowded",
        (Level::Crowded, false) => "Often crowded",
    }
}

#[derive(Serialize)]
pub struct LegSummary {
    pub summary: String,
    pub level: Option<Level>,
    #[serde(flatten)]
    pub detail: LegCrowding,
    pub live_factor: f64,
}

pub fn summarise_leg(l: LegCrowding, live_factor: f64) -> LegSummary {
    let summary = match (l.worst_level, &l.worst_from, &l.worst_to) {
        (None, _, _) => "No crowding data for this bus".to_string(),
        (Some(Level::Seats), _, _) => "Seats likely for the whole ride".to_string(),
        (Some(lvl), Some(a), Some(b)) => {
            let word = if lvl == Level::Crowded {
                "Crowded"
            } else {
                "Standing room"
            };
            if l.level_at_boarding == Some(Level::Seats) && lvl == Level::Crowded {
                format!("{word} {a} → {b}; seats when you board")
            } else {
                format!("{word} {a} → {b}")
            }
        }
        (Some(lvl), _, _) => lvl.label().to_string(),
    };
    LegSummary {
        summary,
        level: l.worst_level,
        detail: l,
        live_factor,
    }
}

/// Skip the second operator's copy of a route number; the model counts it on the first.
fn is_copy(l: &Loaded, pattern: usize) -> bool {
    let r = l.snap.patterns[pattern].route as usize;
    l.route_canon[r] as usize != r
}

#[get("/health")]
pub async fn health() -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "ok"}))
}

#[get("/ready")]
pub async fn ready(st: State) -> HttpResponse {
    match st.loaded.load_full() {
        Some(_) => HttpResponse::Ok().json(json!({"status": "ready"})),
        None => HttpResponse::ServiceUnavailable().json(json!({"status": "loading"})),
    }
}

#[get("/metrics")]
pub async fn metrics(st: State) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4")
        .body(st.metrics.render())
}

#[get("/v1/meta")]
pub async fn meta(st: State) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let s = &l.snap;
    let now = st.now();
    let live = st.live.lock();
    Ok(HttpResponse::Ok().json(json!({
        "meta": s.meta,
        "bins": {"minutes": s.config.bin_minutes, "first": hhmm(s.config.first_minute * 60), "count": s.config.n_bins()},
        "capacity_classes": s.config.capacity_classes,
        "multipliers": s.config.multipliers,
        "levels": [
            {"level": "seats", "label": Level::Seats.label()},
            {"level": "standing", "label": Level::Standing.label()},
            {"level": "crowded", "label": Level::Crowded.label()}
        ],
        "routes_with_data": s.patterns.iter().filter(|p| p.cells.is_some()).count(),
        "loaded_at": l.loaded_at,
        "now": now,
        "live": {"enabled": st.cfg.live.enabled, "clock": st.cfg.live.clock, "tickets": live.observed_total, "routes": live.routes_seen(), "latest_ts": live.latest_ts},
        "otp": {"enabled": st.cfg.otp.enabled, "crowding_patch": st.cfg.otp.crowding_patch},
    })))
}

#[get("/v1/routes")]
pub async fn routes(st: State, q: web::Query<TimeQuery>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let s = &l.snap;
    let needle = q.q.as_deref().map(|x| x.trim().to_ascii_uppercase());
    let limit = q.limit.unwrap_or(5000);
    let mut out = Vec::new();
    for (ri, r) in s.routes.iter().enumerate() {
        if needle
            .as_ref()
            .is_some_and(|n| !r.name().to_ascii_uppercase().contains(n.as_str()))
        {
            continue;
        }
        let pats: Vec<Value> = l.patterns_by_route[ri]
            .iter()
            .map(|&p| {
                let sp = &s.patterns[p as usize];
                json!({
                    "pattern": p,
                    "stops": sp.stops.len(),
                    "first_stop": s.stops[sp.stops[0] as usize].name,
                    "last_stop": s.stops[*sp.stops.last().unwrap() as usize].name,
                    "trips": sp.trips.len(),
                    "bus_class": sp.class,
                    "ticketed_riders": sp.tickets,
                    "has_data": sp.cells.is_some(),
                })
            })
            .collect();
        let has = l.patterns_by_route[ri]
            .iter()
            .any(|&p| s.patterns[p as usize].cells.is_some());
        out.push(json!({"route_id": r.route_id, "name": r.name(), "agency": r.agency_id, "has_data": has, "patterns": pats}));
        if out.len() >= limit {
            break;
        }
    }
    Ok(HttpResponse::Ok().json(out))
}

#[get("/v1/routes/{route_id}/profile")]
pub async fn route_profile(
    st: State,
    path: web::Path<String>,
    q: web::Query<TimeQuery>,
) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let s = &l.snap;
    let ri = *l
        .route_by_id
        .get(path.as_str())
        .ok_or_else(|| AppError::NotFound(format!("route {path}")))?;
    let when = st.resolve(&l, q.at.as_deref(), q.date.as_deref(), q.time.as_deref())?;
    let dt = daytype(&q, &when)?;
    let scen = scenario(q.scenario.as_deref(), &st.cfg.default_scenario)?;
    let nb = s.config.n_bins();
    let bins: Vec<String> = (0..nb)
        .map(|b| hhmm(s.config.bin_start_minute(b) * 60))
        .collect();
    let grid = s.grid();
    let mut pats = Vec::new();
    for &pid in &l.patterns_by_route[ri as usize] {
        let p = &s.patterns[pid as usize];
        let stops: Vec<Value> = p
            .stops
            .iter()
            .zip(&p.dist_m)
            .map(|(&i, d)| {
                let x = &s.stops[i as usize];
                json!({"stop_id": x.stop_id, "name": x.name, "lat": x.lat, "lon": x.lon, "km": d / 1000.0})
            })
            .collect();
        let mut load = Vec::new();
        let mut level = Vec::new();
        let mut conf = Vec::new();
        let mut buses = Vec::new();
        let mut pcrowd = Vec::new();
        for seg in 0..p.stops.len() - 1 {
            let mut rl = Vec::with_capacity(nb);
            let mut rv = Vec::with_capacity(nb);
            let mut rc = Vec::with_capacity(nb);
            let mut rb = Vec::with_capacity(nb);
            let mut rp = Vec::with_capacity(nb);
            for bin in 0..nb {
                match s.cell(pid, seg, dt, bin, scen, 1.0) {
                    Some(v) => {
                        rl.push(Some((v.load * 10.0).round() / 10.0));
                        rv.push(Some(v.level as u8));
                        rc.push(Some(v.confidence as u8));
                        rb.push(Some((v.buses_per_day * 10.0).round() / 10.0));
                        rp.push(Some((v.p_crowded * 100.0).round() / 100.0));
                    }
                    None => {
                        rl.push(None);
                        rv.push(None);
                        rc.push(None);
                        rb.push(None);
                        rp.push(None);
                    }
                }
            }
            load.push(rl);
            level.push(rv);
            conf.push(rc);
            buses.push(rb);
            pcrowd.push(rp);
        }
        let e = s.meta.expansion.pick(scen) * p.expansion_adj as f64;
        let boardings: Vec<Vec<f32>> = p
            .cells
            .as_ref()
            .map(|c| {
                (0..p.stops.len())
                    .map(|pos| {
                        (0..nb)
                            .map(|b| (c.board[grid.idx(pos, dt.idx(), b)] as f64 * e) as f32)
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let class = s.class_of(pid);
        pats.push(json!({
            "pattern": pid, "bus_class": class, "expansion_adjustment": p.expansion_adj,
            "ticketed_riders": p.tickets, "stops": stops, "has_data": p.cells.is_some(),
            "load": load, "level": level, "confidence": conf, "buses_per_day": buses, "p_crowded": pcrowd,
            "boardings_per_day": boardings,
        }));
    }
    let r = &s.routes[ri as usize];
    Ok(HttpResponse::Ok().json(json!({
        "route_id": r.route_id, "name": r.name(), "daytype": dt, "scenario": scen, "bins": bins,
        "level_codes": {"0": "seats", "1": "standing", "2": "crowded"},
        "confidence_codes": {"0": "low", "1": "medium", "2": "high"},
        "patterns": pats,
    })))
}

/// Well-known places whose common name differs from any stop name in the feed.
const LANDMARKS: &[(&str, f64, f64)] = &[
    ("Connaught Place", 28.6315, 77.2167),
    ("Rajiv Chowk", 28.6328, 77.2197),
    ("New Delhi Railway Station", 28.6430, 77.2194),
    ("India Gate", 28.6129, 77.2295),
    ("AIIMS", 28.5672, 77.2100),
    ("Nehru Place", 28.5494, 77.2519),
    ("Kashmere Gate ISBT", 28.6675, 77.2282),
    ("Anand Vihar ISBT", 28.6469, 77.3159),
    ("Karol Bagh", 28.6519, 77.1909),
    ("Lajpat Nagar", 28.5677, 77.2433),
];

#[get("/v1/stops")]
pub async fn search_stops(st: State, q: web::Query<TimeQuery>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let s = &l.snap;
    let needle = norm_stop(q.q.as_deref().unwrap_or(""));
    let limit = q.limit.unwrap_or(10).min(100);
    if needle.is_empty() {
        return Ok(HttpResponse::Ok().json(Vec::<Value>::new()));
    }
    let mut hits: Vec<(i32, u32)> = l
        .stop_search
        .iter()
        .filter_map(|(n, i)| {
            let score = if n == &needle {
                0
            } else if n.starts_with(&needle) {
                1
            } else if n.split(' ').any(|w| w.starts_with(&needle)) {
                2
            } else if n.contains(&needle) {
                3
            } else {
                return None;
            };
            let bonus = if l.stop_has_data[*i as usize] { 0 } else { 10 };
            Some((score + bonus, *i))
        })
        .collect();
    hits.sort_by_key(|h| (h.0, s.stops[h.1 as usize].name.len()));
    let mut out: Vec<Value> = LANDMARKS
        .iter()
        .filter(|(n, _, _)| norm_stop(n).contains(&needle))
        .map(|(n, lat, lon)| json!({"stop_id": null, "name": n, "lat": lat, "lon": lon, "has_data": true, "kind": "place"}))
        .collect();
    // One entry per name: stops on both sides of a road share a name.
    let mut seen = rustc_hash::FxHashSet::default();
    let stops_out: Vec<Value> = hits
        .into_iter()
        .filter(|(_, i)| seen.insert(s.stops[*i as usize].name.to_lowercase()))
        .take(limit)
        .map(|(_, i)| {
            let x = &s.stops[i as usize];
            json!({"stop_id": x.stop_id, "name": x.name, "lat": x.lat, "lon": x.lon, "has_data": l.stop_has_data[i as usize], "kind": "stop"})
        })
        .collect();
    out.extend(stops_out);
    out.truncate(limit);
    Ok(HttpResponse::Ok().json(out))
}

#[derive(Serialize)]
struct SegAgg {
    from: u32,
    to: u32,
    load_x_buses: f64,
    buses: f64,
    pc_x_buses: f64,
    ps_x_buses: f64,
    /// Rider-weighted sums: riders per day, and riders standing / in a crowded bus.
    riders: f64,
    riders_standing: f64,
    riders_crowded: f64,
    worst: Level,
    min_conf: Confidence,
    routes: Vec<String>,
}

/// Map layer: every stop-to-stop segment with buses in the chosen bin, aggregated over the
/// routes that use it (bus-weighted load; worst level).
#[get("/v1/map/segments")]
pub async fn map_segments(st: State, q: web::Query<TimeQuery>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let s = &l.snap;
    let when = st.resolve(&l, q.at.as_deref(), q.date.as_deref(), q.time.as_deref())?;
    let dt = daytype(&q, &when)?;
    let scen = scenario(q.scenario.as_deref(), &st.cfg.default_scenario)?;
    let bin = s.config.bin_of(when.secs).unwrap_or(0);
    let key = (dt.idx() as u8, bin as u16, scen as u8);
    if let Some(v) = l.map_cache.lock().get(&key) {
        return Ok(HttpResponse::Ok().json(v.as_ref()));
    }
    let mut agg: FxHashMap<(u32, u32), SegAgg> = FxHashMap::default();
    for (pid, p) in s.patterns.iter().enumerate() {
        if p.cells.is_none() || is_copy(&l, pid) {
            continue;
        }
        let rname = s.routes[p.route as usize].name();
        for seg in 0..p.stops.len() - 1 {
            // Very long stop-to-stop links (express sections) would be drawn as straight lines.
            if p.dist_m[seg + 1] - p.dist_m[seg] > 3000.0 {
                continue;
            }
            let Some(v) = s.cell(pid as u32, seg, dt, bin, scen, 1.0) else {
                continue;
            };
            let k = (p.stops[seg], p.stops[seg + 1]);
            let a = agg.entry(k).or_insert(SegAgg {
                from: k.0,
                to: k.1,
                load_x_buses: 0.0,
                buses: 0.0,
                pc_x_buses: 0.0,
                ps_x_buses: 0.0,
                riders: 0.0,
                riders_standing: 0.0,
                riders_crowded: 0.0,
                worst: Level::Seats,
                min_conf: Confidence::High,
                routes: Vec::new(),
            });
            a.load_x_buses += v.load * v.buses_per_day;
            a.pc_x_buses += v.p_crowded * v.buses_per_day;
            a.ps_x_buses += v.p_standing * v.buses_per_day;
            let r = v.load * v.buses_per_day;
            a.riders += r;
            a.riders_standing += v.p_standing * r;
            a.riders_crowded += v.p_crowded * r;
            a.buses += v.buses_per_day;
            a.worst = a.worst.max(v.level);
            a.min_conf = a.min_conf.min(v.confidence);
            a.routes.push(rname.to_string());
        }
    }
    let features: Vec<Value> = agg
        .into_values()
        .filter(|a| a.buses > 0.0)
        .map(|a| {
            let (f, t) = (&s.stops[a.from as usize], &s.stops[a.to as usize]);
            // Level of the median bus over all routes on this stretch.
            let (pc, ps) = (a.pc_x_buses / a.buses, a.ps_x_buses / a.buses);
            let level = if pc >= 0.5 {
                Level::Crowded
            } else if pc + ps >= 0.5 {
                Level::Standing
            } else {
                Level::Seats
            };
            json!({
                "type": "Feature",
                "geometry": {"type": "LineString", "coordinates": [[f.lon, f.lat], [t.lon, t.lat]]},
                "properties": {
                    "from": f.name, "to": t.name,
                    "load": (a.load_x_buses / a.buses * 10.0).round() / 10.0,
                    "p_crowded": (pc * 100.0).round() / 100.0,
                    "p_standing": (ps * 100.0).round() / 100.0,
                    "riders_per_day": a.riders.round(),
                    "riders_standing": a.riders_standing.round(),
                    "riders_crowded": a.riders_crowded.round(),
                    "worst_route_level": a.worst.as_str(),
                    "buses_per_day": (a.buses * 10.0).round() / 10.0,
                    "level": level.as_str(), "confidence": a.min_conf.as_str(),
                    "routes": a.routes.join(" "),
                }
            })
        })
        .collect();
    let v = Arc::new(
        json!({"type": "FeatureCollection", "daytype": dt, "bin_start": hhmm(s.config.bin_start_minute(bin) * 60), "features": features}),
    );
    l.map_cache.lock().insert(key, v.clone());
    Ok(HttpResponse::Ok().json(v.as_ref()))
}

#[derive(Debug, Deserialize)]
pub struct LegIn {
    pub trip_id: Option<String>,
    pub route_id: Option<String>,
    pub from_stop_id: String,
    pub to_stop_id: String,
    /// Departure from the boarding stop, ISO with offset.
    pub start_time: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AnnotateIn {
    pub legs: Vec<LegIn>,
    pub scenario: Option<String>,
}

/// Locate board/alight positions for a trip.
pub fn positions(l: &Loaded, trip: u32, from: u32, to: u32) -> Option<(usize, usize)> {
    let p = &l.snap.patterns[l.snap.trips[trip as usize].pattern as usize];
    let b = p.stops.iter().position(|&s| s == from)?;
    let a = p.stops[b + 1..].iter().position(|&s| s == to)? + b + 1;
    Some((b, a))
}

pub fn annotate_one(
    st: &AppState,
    l: &Loaded,
    leg: &LegIn,
    scen: ScenarioPick,
) -> AppResult<Value> {
    let from = *l
        .stop_by_id
        .get(Loaded::local_id(&leg.from_stop_id))
        .ok_or_else(|| AppError::NotFound(format!("stop {}", leg.from_stop_id)))?;
    let to = *l
        .stop_by_id
        .get(Loaded::local_id(&leg.to_stop_id))
        .ok_or_else(|| AppError::NotFound(format!("stop {}", leg.to_stop_id)))?;
    let when = st.resolve(l, leg.start_time.as_deref(), None, None)?;
    let trip = match &leg.trip_id {
        Some(t) => l.trip_by_id.get(Loaded::local_id(t)).copied(),
        None => None,
    };
    let trip = match trip {
        Some(t) => t,
        None => {
            // No (known) trip: nearest scheduled trip of the route passing `from` at that time.
            let rid = leg
                .route_id
                .as_deref()
                .map(Loaded::local_id)
                .ok_or_else(|| AppError::BadRequest("need a known trip_id or a route_id".into()))?;
            let ri = *l
                .route_by_id
                .get(rid)
                .ok_or_else(|| AppError::NotFound(format!("route {rid}")))?;
            let mut best: Option<(u32, u32)> = None;
            for &pid in &l.patterns_by_route[ri as usize] {
                let p = &l.snap.patterns[pid as usize];
                let Some(b) = p.stops.iter().position(|&s| s == from) else {
                    continue;
                };
                for &t in &p.trips {
                    let d = l.snap.trips[t as usize].times[b].abs_diff(when.secs);
                    if best.is_none_or(|x| d < x.1) {
                        best = Some((t, d));
                    }
                }
            }
            best.map(|b| b.0)
                .ok_or_else(|| AppError::NotFound("no trip of that route serves the stop".into()))?
        }
    };
    let (b, a) = positions(l, trip, from, to)
        .ok_or_else(|| AppError::BadRequest("stops are not in order on that trip".into()))?;
    let route = l.snap.trips[trip as usize].pattern;
    let route = l.snap.patterns[route as usize].route;
    let lf = st.live_factor(l, route, &when);
    let lc = l.snap.leg(trip, b, a, when.dt, scen, lf.factor);
    Ok(json!({
        "trip_id": l.snap.trips[trip as usize].trip_id,
        "route": l.snap.routes[route as usize].name(),
        "daytype": when.dt,
        "crowding": summarise_leg(lc, lf.factor),
    }))
}

#[post("/v1/legs:annotate")]
pub async fn annotate(st: State, body: web::Json<AnnotateIn>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let scen = scenario(body.scenario.as_deref(), &st.cfg.default_scenario)?;
    let out: Vec<Value> = body
        .legs
        .iter()
        .map(|leg| {
            annotate_one(&st, &l, leg, scen).unwrap_or_else(|e| json!({"error": e.to_string()}))
        })
        .collect();
    Ok(HttpResponse::Ok().json(json!({"legs": out})))
}

fn window(q: &TimeQuery, s: &crowd_core::snapshot::Snapshot) -> AppResult<(usize, usize)> {
    let f = q
        .from
        .as_deref()
        .map(hhmm_to_secs)
        .transpose()?
        .unwrap_or(7 * 3600);
    let t =
        q.to.as_deref()
            .map(hhmm_to_secs)
            .transpose()?
            .unwrap_or(10 * 3600);
    let b0 = s.config.bin_of(f).unwrap_or(0);
    let b1 = s
        .config
        .bin_of(t.saturating_sub(1))
        .unwrap_or(s.config.n_bins() - 1);
    Ok((b0, b1.max(b0)))
}

/// Where extra buses would help most: segments ranked by riders above comfortable load,
/// summed over the window's bins and buses.
#[get("/v1/operator/hotspots")]
pub async fn hotspots(st: State, q: web::Query<TimeQuery>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let s = &l.snap;
    let dt = q
        .daytype
        .as_deref()
        .and_then(DayType::parse)
        .unwrap_or(DayType::Weekday);
    let scen = scenario(q.scenario.as_deref(), &st.cfg.default_scenario)?;
    let (b0, b1) = window(&q, s)?;
    let limit = q.limit.unwrap_or(50).min(500);
    let mut rows = Vec::new();
    for (pid, p) in s.patterns.iter().enumerate() {
        if p.cells.is_none() || is_copy(&l, pid) {
            continue;
        }
        let class = s.class_of(pid as u32);
        for seg in 0..p.stops.len() - 1 {
            let mut excess = 0.0;
            let mut peak: Option<(CellView, usize)> = None;
            for bin in b0..=b1 {
                let Some(v) = s.cell(pid as u32, seg, dt, bin, scen, 1.0) else {
                    continue;
                };
                excess += (v.load - class.comfortable).max(0.0) * v.buses_per_day;
                if peak.is_none_or(|(pv, _)| v.load > pv.load) {
                    peak = Some((v, bin));
                }
            }
            if let Some((v, bin)) = peak {
                if excess > 0.0 {
                    rows.push((excess, pid, seg, v, bin));
                }
            }
        }
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    let out: Vec<Value> = rows
        .into_iter()
        .take(limit)
        .map(|(excess, pid, seg, v, bin)| {
            let p = &s.patterns[pid];
            let class = s.class_of(pid as u32);
            json!({
                "route_id": s.routes[p.route as usize].route_id, "route": s.routes[p.route as usize].name(),
                "from": s.stops[p.stops[seg] as usize].name, "to": s.stops[p.stops[seg + 1] as usize].name,
                "from_lat": s.stops[p.stops[seg] as usize].lat, "from_lon": s.stops[p.stops[seg] as usize].lon,
                "peak_time": hhmm(s.config.bin_start_minute(bin) * 60), "peak_load": (v.load * 10.0).round() / 10.0,
                "peak_level": v.level, "p_crowded": (v.p_crowded * 100.0).round() / 100.0, "confidence": v.confidence,
                "buses_per_day_at_peak": (v.buses_per_day * 10.0).round() / 10.0,
                "excess_riders": excess.round(),
                "extra_buses_at_peak": ((v.load / class.comfortable - 1.0).max(0.0) * v.buses_per_day * 10.0).round() / 10.0,
                "bus_class": class.name,
            })
        })
        .collect();
    Ok(HttpResponse::Ok().json(json!({"daytype": dt, "from": hhmm(s.config.bin_start_minute(b0) * 60), "to": hhmm(s.config.bin_start_minute(b1 + 1) * 60), "hotspots": out})))
}

/// Route table for operators: peak load and where, share of bus-km crowded, and how many
/// extra trips would bring the worst bin down to comfortable standing.
#[get("/v1/operator/routes")]
pub async fn operator_routes(st: State, q: web::Query<TimeQuery>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let s = &l.snap;
    let dt = q
        .daytype
        .as_deref()
        .and_then(DayType::parse)
        .unwrap_or(DayType::Weekday);
    let scen = scenario(q.scenario.as_deref(), &st.cfg.default_scenario)?;
    let (b0, b1) = window(&q, s)?;
    let limit = q.limit.unwrap_or(100).min(3000);
    let mut rows = Vec::new();
    for (pid, p) in s.patterns.iter().enumerate() {
        if p.cells.is_none() || is_copy(&l, pid) {
            continue;
        }
        let class = s.class_of(pid as u32);
        let (mut km_all, mut km_crowd, mut km_stand, mut km_hi) = (0.0, 0.0, 0.0, 0.0);
        let mut peak: Option<(CellView, usize, usize)> = None;
        let mut extra_trips = 0.0;
        for bin in b0..=b1 {
            let mut worst_ratio: f64 = 0.0;
            let mut buses_at = 0.0;
            for seg in 0..p.stops.len() - 1 {
                let Some(v) = s.cell(pid as u32, seg, dt, bin, scen, 1.0) else {
                    continue;
                };
                let km = (p.dist_m[seg + 1] - p.dist_m[seg]) as f64 / 1000.0 * v.buses_per_day;
                km_all += km;
                match v.level {
                    Level::Crowded => km_crowd += km,
                    Level::Standing => km_stand += km,
                    _ => {}
                }
                if v.confidence == Confidence::High {
                    km_hi += km;
                }
                if peak.is_none_or(|(pv, _, _)| v.load > pv.load) {
                    peak = Some((v, seg, bin));
                }
                worst_ratio = worst_ratio.max(v.load / class.comfortable);
                buses_at = v.buses_per_day.max(buses_at);
            }
            extra_trips += (worst_ratio - 1.0).max(0.0) * buses_at;
        }
        let Some((pv, pseg, pbin)) = peak else {
            continue;
        };
        let share = |x: f64| {
            if km_all > 0.0 {
                (x / km_all * 1000.0).round() / 1000.0
            } else {
                0.0
            }
        };
        rows.push(json!({
            "route_id": s.routes[p.route as usize].route_id, "route": s.routes[p.route as usize].name(),
            "pattern": pid, "bus_class": class.name, "ticketed_riders": p.tickets,
            "peak_load": (pv.load * 10.0).round() / 10.0, "peak_level": pv.level,
            "peak_from": s.stops[p.stops[pseg] as usize].name, "peak_to": s.stops[p.stops[pseg + 1] as usize].name,
            "peak_time": hhmm(s.config.bin_start_minute(pbin) * 60),
            "share_crowded": share(km_crowd), "share_standing": share(km_stand), "share_high_confidence": share(km_hi),
            "extra_trips_needed": (extra_trips * 10.0).round() / 10.0,
            "expansion_adjustment": p.expansion_adj,
        }));
    }
    rows.sort_by(|a, b| {
        let k = |v: &Value| {
            v["extra_trips_needed"].as_f64().unwrap_or(0.0)
                + v["peak_load"].as_f64().unwrap_or(0.0) / 1000.0
        };
        k(b).total_cmp(&k(a))
    });
    rows.truncate(limit);
    Ok(HttpResponse::Ok().json(json!({"daytype": dt, "from": hhmm(s.config.bin_start_minute(b0) * 60), "to": hhmm(s.config.bin_start_minute(b1 + 1) * 60), "routes": rows})))
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum TicketsIn {
    Many(Vec<RawTicket>),
    One(RawTicket),
}

/// Ingest one parsed ticket into the nowcast. Returns false if its route is unknown.
pub fn ingest(st: &AppState, l: &Loaded, raw: &RawTicket) -> bool {
    let mut names = Interner::default();
    let Ok(t) = raw.parse(&mut names, ist()) else {
        return false;
    };
    let Some(route) = l.route_for_ticket(&raw.route) else {
        return false;
    };
    st.live.lock().observe(route, t.ts, t.pax);
    true
}

#[post("/v1/tickets")]
pub async fn post_tickets(st: State, body: web::Json<TicketsIn>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let list = match body.into_inner() {
        TicketsIn::Many(v) => v,
        TicketsIn::One(t) => vec![t],
    };
    let (mut ok, mut bad) = (0u64, 0u64);
    for raw in &list {
        if ingest(&st, &l, raw) {
            ok += 1;
        } else {
            bad += 1;
        }
    }
    st.metrics.tickets_ingested.inc_by(ok);
    st.metrics.tickets_rejected.inc_by(bad);
    Ok(HttpResponse::Accepted().json(json!({"accepted": ok, "rejected": bad})))
}

#[get("/v1/live/status")]
pub async fn live_status(st: State) -> AppResult<HttpResponse> {
    let now = st.now();
    let live = st.live.lock();
    Ok(HttpResponse::Ok().json(json!({
        "enabled": st.cfg.live.enabled, "clock": st.cfg.live.clock, "now": now,
        "tickets": live.observed_total, "routes": live.routes_seen(), "latest_ts": live.latest_ts,
        "window_secs": st.cfg.live.window_secs,
    })))
}

#[get("/v1/live/routes/{route_id}")]
pub async fn live_route(st: State, path: web::Path<String>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let ri = *l
        .route_by_id
        .get(path.as_str())
        .ok_or_else(|| AppError::NotFound(format!("route {path}")))?;
    let when = st.resolve(&l, None, None, None)?;
    let f = st.live_factor(&l, ri, &when);
    Ok(HttpResponse::Ok().json(json!({
        "route_id": path.as_str(), "route": l.snap.routes[ri as usize].name(), "now": st.now(), "daytype": when.dt,
        "live_factor": f.factor, "observed_tickets": f.observed, "expected_tickets": f.expected,
        "meaning": "riders now vs the usual for this route, day type and hour (1.0 = usual)",
    })))
}
