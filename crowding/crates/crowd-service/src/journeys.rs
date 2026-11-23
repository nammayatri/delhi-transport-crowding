//! `/v1/journeys`: plan a trip and show crowding next to each option.
//!
//! Candidates come from OpenTripPlanner (when configured) or the built-in RAPTOR planner,
//! at the requested time and a few later departures, with and without crowding
//! reluctance. Each bus leg is annotated from the crowding model; options are scored as
//! `time from the requested departure + reluctance x crowding-extra time + transfer
//! penalty`, and labelled "Fastest", "Most comfortable" and "Leave later, seats likely".

use actix_web::{post, web, HttpResponse};
use chrono::{NaiveDate, TimeZone};
use crowd_core::levels::{Level, ScenarioPick};
use crowd_core::planner::{plan as raptor, Leg, Query};
use crowd_core::tickets::ist;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::{hhmm, positions, scenario, summarise_leg, LegSummary, State};
use crate::error::{AppError, AppResult};
use crate::otp;
use crate::state::{AppState, Loaded, When};

#[derive(Debug, Deserialize)]
pub struct PlaceIn {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub stop_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JourneyIn {
    pub from: PlaceIn,
    pub to: PlaceIn,
    /// ISO date-time with offset; default now.
    pub depart_at: Option<String>,
    /// 0 = fastest only, 1 = strongly prefer comfort.
    pub comfort: Option<f64>,
    pub scenario: Option<String>,
    /// "auto" (OTP if enabled, else built-in), "otp" or "builtin".
    pub engine: Option<String>,
    pub max_options: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaceOut {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub stop_id: Option<String>,
}

#[derive(Serialize)]
pub struct OptLeg {
    pub mode: &'static str,
    pub from: PlaceOut,
    pub to: PlaceOut,
    pub depart: String,
    pub arrive: String,
    pub minutes: f64,
    pub route: Option<String>,
    pub route_id: Option<String>,
    pub trip_id: Option<String>,
    pub stops: usize,
    pub geometry: Vec<[f64; 2]>,
    pub crowding: Option<LegSummary>,
}

#[derive(Serialize)]
pub struct JourneyOption {
    pub labels: Vec<&'static str>,
    pub recommended: bool,
    pub depart: String,
    pub arrive: String,
    pub depart_ts: i64,
    pub arrive_ts: i64,
    pub minutes: f64,
    /// Minutes from the requested departure to arrival (includes waiting to leave later).
    pub minutes_from_request: f64,
    pub transfers: usize,
    pub walk_minutes: f64,
    pub worst_level: Option<Level>,
    pub summary: String,
    pub crowding_extra_minutes: f64,
    /// Ride time as it feels: in-vehicle minutes weighted by crowding.
    pub comfort_minutes: f64,
    pub score: f64,
    pub legs: Vec<OptLeg>,
    pub source: &'static str,
}

fn ts_of(date: NaiveDate, secs: u32) -> i64 {
    ist()
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap()
        .timestamp()
        + secs as i64
}

fn iso(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .unwrap()
        .with_timezone(&ist())
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

fn stop_place(l: &Loaded, i: u32) -> PlaceOut {
    let s = &l.snap.stops[i as usize];
    PlaceOut {
        name: s.name.clone(),
        lat: s.lat,
        lon: s.lon,
        stop_id: Some(s.stop_id.clone()),
    }
}

fn resolve_place(l: &Loaded, p: &PlaceIn, what: &str) -> AppResult<(f64, f64)> {
    if let Some(id) = &p.stop_id {
        let i = l
            .stop_by_id
            .get(Loaded::local_id(id))
            .ok_or_else(|| AppError::NotFound(format!("stop {id}")))?;
        let s = &l.snap.stops[*i as usize];
        return Ok((s.lat, s.lon));
    }
    match (p.lat, p.lon) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err(AppError::BadRequest(format!(
            "{what} needs lat/lon or stop_id"
        ))),
    }
}

/// Annotate a bus leg given its trip and stop indices.
fn bus_crowding(
    st: &AppState,
    l: &Loaded,
    trip: u32,
    b: usize,
    a: usize,
    when: &When,
    scen: ScenarioPick,
) -> LegSummary {
    let route = l.snap.patterns[l.snap.trips[trip as usize].pattern as usize].route;
    let lf = st.live_factor(l, route, when).factor;
    summarise_leg(l.snap.leg(trip, b, a, when.dt, scen, lf), lf)
}

struct Candidate {
    depart: i64,
    arrive: i64,
    legs: Vec<OptLeg>,
    source: &'static str,
    key: String,
}

#[allow(clippy::too_many_arguments)]
fn builtin_candidates(
    st: &AppState,
    l: &Loaded,
    from: (f64, f64),
    to: (f64, f64),
    when: &When,
    rels: &[f64],
    offsets: &[u32],
    scen: ScenarioPick,
) -> Vec<Candidate> {
    let active = l.active(when.date);
    let mut out = Vec::new();
    for &off in offsets {
        for &rel in rels {
            let q = Query {
                from,
                to,
                date: when.date,
                depart: when.secs + off * 60,
                crowding_reluctance: rel,
            };
            for j in raptor(&l.snap, &l.net, &active, &q) {
                let mut legs = Vec::new();
                let mut key = String::new();
                for leg in &j.legs {
                    match leg {
                        Leg::Walk {
                            from: f,
                            to: t,
                            depart,
                            arrive,
                        } => {
                            let place =
                                |p: &crowd_core::planner::Place, fallback: &str| match p.stop {
                                    Some(i) => stop_place(l, i),
                                    None => PlaceOut {
                                        name: fallback.into(),
                                        lat: p.lat,
                                        lon: p.lon,
                                        stop_id: None,
                                    },
                                };
                            if arrive > depart {
                                legs.push(OptLeg {
                                    mode: "walk",
                                    from: place(f, "Start"),
                                    to: place(t, "Destination"),
                                    depart: iso(ts_of(when.date, *depart)),
                                    arrive: iso(ts_of(when.date, *arrive)),
                                    minutes: (arrive - depart) as f64 / 60.0,
                                    route: None,
                                    route_id: None,
                                    trip_id: None,
                                    stops: 0,
                                    geometry: vec![[f.lon, f.lat], [t.lon, t.lat]],
                                    crowding: None,
                                });
                            }
                        }
                        Leg::Bus {
                            trip,
                            board,
                            alight,
                            depart,
                            arrive,
                        } => {
                            let t = &l.snap.trips[*trip as usize];
                            let p = &l.snap.patterns[t.pattern as usize];
                            let r = &l.snap.routes[p.route as usize];
                            key.push_str(&t.trip_id);
                            key.push('|');
                            let geometry = p.stops[*board as usize..=*alight as usize]
                                .iter()
                                .map(|&s| {
                                    [l.snap.stops[s as usize].lon, l.snap.stops[s as usize].lat]
                                })
                                .collect();
                            legs.push(OptLeg {
                                mode: "bus",
                                from: stop_place(l, p.stops[*board as usize]),
                                to: stop_place(l, p.stops[*alight as usize]),
                                depart: iso(ts_of(when.date, *depart)),
                                arrive: iso(ts_of(when.date, *arrive)),
                                minutes: (arrive - depart) as f64 / 60.0,
                                route: Some(r.name().to_string()),
                                route_id: Some(r.route_id.clone()),
                                trip_id: Some(t.trip_id.clone()),
                                stops: (*alight - *board) as usize,
                                geometry,
                                crowding: Some(bus_crowding(
                                    st,
                                    l,
                                    *trip,
                                    *board as usize,
                                    *alight as usize,
                                    when,
                                    scen,
                                )),
                            });
                        }
                    }
                }
                if key.is_empty() {
                    continue;
                }
                out.push(Candidate {
                    depart: ts_of(when.date, j.depart),
                    arrive: ts_of(when.date, j.arrive),
                    legs,
                    source: "builtin",
                    key,
                });
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
async fn otp_candidates(
    st: &AppState,
    l: &Loaded,
    from: (f64, f64),
    to: (f64, f64),
    when: &When,
    rels: &[f64],
    offsets: &[u32],
    scen: ScenarioPick,
) -> AppResult<Vec<Candidate>> {
    let mut out = Vec::new();
    for &off in offsets {
        let t = when.ts + off as i64 * 60;
        let dt = chrono::DateTime::from_timestamp(t, 0)
            .unwrap()
            .with_timezone(&ist());
        let (date, time) = (
            dt.format("%Y-%m-%d").to_string(),
            dt.format("%H:%M").to_string(),
        );
        let rel_list: Vec<Option<f64>> = if st.cfg.otp.crowding_patch {
            rels.iter().map(|r| Some(*r)).collect()
        } else {
            vec![None]
        };
        for rel in rel_list {
            let its = otp::plan(&st.http, &st.cfg.otp, from, to, &date, &time, rel).await?;
            for it in its {
                let mut legs = Vec::new();
                let mut key = String::new();
                for lg in it.legs {
                    let place = |p: &otp::OtpPlace| PlaceOut {
                        name: p.name.clone().unwrap_or_default(),
                        lat: p.lat,
                        lon: p.lon,
                        stop_id: p
                            .stop
                            .as_ref()
                            .map(|s| Loaded::local_id(&s.gtfs_id).to_string()),
                    };
                    let geometry = lg
                        .leg_geometry
                        .as_ref()
                        .map(|g| otp::decode_polyline(&g.points))
                        .unwrap_or_default();
                    let is_bus = lg.mode != "WALK";
                    let mut crowding = None;
                    let mut stops = 0;
                    if is_bus {
                        if let (Some(trip), Some(fs), Some(ts)) =
                            (&lg.trip, &lg.from.stop, &lg.to.stop)
                        {
                            key.push_str(&trip.gtfs_id);
                            key.push('|');
                            let ti = l.trip_by_id.get(Loaded::local_id(&trip.gtfs_id)).copied();
                            let fi = l.stop_by_id.get(Loaded::local_id(&fs.gtfs_id)).copied();
                            let tsi = l.stop_by_id.get(Loaded::local_id(&ts.gtfs_id)).copied();
                            if let (Some(ti), Some(fi), Some(tsi)) = (ti, fi, tsi) {
                                if let Some((b, a)) = positions(l, ti, fi, tsi) {
                                    stops = a - b;
                                    let w = When {
                                        ts: lg.start_time / 1000,
                                        ..*when
                                    };
                                    crowding = Some(bus_crowding(st, l, ti, b, a, &w, scen));
                                }
                            }
                        }
                    }
                    let route = lg.route.as_ref().map(|r| {
                        r.short_name
                            .clone()
                            .filter(|s| !s.is_empty())
                            .or_else(|| r.long_name.clone())
                            .unwrap_or_default()
                    });
                    legs.push(OptLeg {
                        mode: if is_bus { "bus" } else { "walk" },
                        from: place(&lg.from),
                        to: place(&lg.to),
                        depart: iso(lg.start_time / 1000),
                        arrive: iso(lg.end_time / 1000),
                        minutes: (lg.end_time - lg.start_time) as f64 / 60_000.0,
                        route,
                        route_id: lg
                            .route
                            .as_ref()
                            .map(|r| Loaded::local_id(&r.gtfs_id).to_string()),
                        trip_id: lg
                            .trip
                            .as_ref()
                            .map(|t| Loaded::local_id(&t.gtfs_id).to_string()),
                        stops,
                        geometry,
                        crowding,
                    });
                }
                if key.is_empty() {
                    continue;
                }
                out.push(Candidate {
                    depart: it.start_time / 1000,
                    arrive: it.end_time / 1000,
                    legs,
                    source: "otp",
                    key,
                });
            }
        }
    }
    Ok(out)
}

pub async fn plan_journeys(st: &AppState, l: &Loaded, body: &JourneyIn) -> AppResult<Value> {
    let from = resolve_place(l, &body.from, "from")?;
    let to = resolve_place(l, &body.to, "to")?;
    let when = st.resolve(l, body.depart_at.as_deref(), None, None)?;
    let scen = scenario(body.scenario.as_deref(), &st.cfg.default_scenario)?;
    let comfort = body.comfort.unwrap_or(0.5).clamp(0.0, 1.0);
    let lambda = comfort * st.cfg.planner.max_reluctance;
    let rels = [0.0, st.cfg.planner.max_reluctance.max(1.0)];
    let mut offsets = vec![0u32];
    offsets.extend(st.cfg.planner.later_departures_min.iter().copied());
    let engine = body.engine.as_deref().unwrap_or("auto");
    let use_otp = match engine {
        "otp" => true,
        "builtin" => false,
        _ => st.cfg.otp.enabled,
    };
    let mut cands = if use_otp {
        match otp_candidates(st, l, from, to, &when, &rels, &offsets, scen).await {
            Ok(c) => c,
            Err(e) if engine == "auto" => {
                tracing::warn!(error = %e, "OTP failed, falling back to built-in planner");
                builtin_candidates(st, l, from, to, &when, &rels, &offsets, scen)
            }
            Err(e) => return Err(e),
        }
    } else {
        builtin_candidates(st, l, from, to, &when, &rels, &offsets, scen)
    };
    // Deduplicate on the sequence of trips.
    cands.sort_by_key(|c| (c.key.clone(), c.arrive));
    cands.dedup_by(|a, b| a.key == b.key);

    let penalty = st.cfg.planner.transfer_penalty_min * 60.0;
    let mut opts: Vec<JourneyOption> = cands
        .into_iter()
        .map(|c| {
            let transfers = c
                .legs
                .iter()
                .filter(|x| x.mode == "bus")
                .count()
                .saturating_sub(1);
            let walk: f64 = c
                .legs
                .iter()
                .filter(|x| x.mode == "walk")
                .map(|x| x.minutes)
                .sum();
            let extra: f64 = c
                .legs
                .iter()
                .filter_map(|x| x.crowding.as_ref())
                .map(|x| x.detail.crowding_extra_secs)
                .sum::<f64>()
                / 60.0;
            let ride: f64 = c
                .legs
                .iter()
                .filter(|x| x.mode == "bus")
                .map(|x| x.minutes)
                .sum();
            // Unknown if any bus leg has no estimate: such an option is never labelled for comfort.
            let all_known = c
                .legs
                .iter()
                .filter(|x| x.mode == "bus")
                .all(|x| x.crowding.as_ref().is_some_and(|c| c.detail.has_data));
            let worst = if all_known {
                c.legs
                    .iter()
                    .filter_map(|x| x.crowding.as_ref().and_then(|c| c.level))
                    .max()
            } else {
                None
            };
            let from_req = (c.arrive - when.ts) as f64 / 60.0;
            // A bus with no crowding estimate is not assumed empty: for ranking it counts as
            // mildly crowded (multiplier 1.2), between seated and standing.
            let unknown: f64 = c
                .legs
                .iter()
                .filter(|x| {
                    x.mode == "bus" && !x.crowding.as_ref().is_some_and(|c| c.detail.has_data)
                })
                .map(|x| 0.2 * x.minutes)
                .sum();
            let score = from_req + lambda * (extra + unknown) + transfers as f64 * penalty / 60.0;
            let summary = c
                .legs
                .iter()
                .filter_map(|x| {
                    x.crowding.as_ref().map(|cr| {
                        format!("{}: {}", x.route.clone().unwrap_or_default(), cr.summary)
                    })
                })
                .collect::<Vec<_>>()
                .join(" · ");
            JourneyOption {
                labels: Vec::new(),
                recommended: false,
                depart: iso(c.depart),
                arrive: iso(c.arrive),
                depart_ts: c.depart,
                arrive_ts: c.arrive,
                minutes: ((c.arrive - c.depart) as f64 / 60.0).round(),
                minutes_from_request: from_req.round(),
                transfers,
                walk_minutes: walk.round(),
                worst_level: worst,
                summary,
                crowding_extra_minutes: (extra * 10.0).round() / 10.0,
                comfort_minutes: ((ride + extra) * 10.0).round() / 10.0,
                score,
                legs: c.legs,
                source: c.source,
            }
        })
        .collect();
    if opts.is_empty() {
        return Ok(
            json!({"options": [], "message": "No bus journey found near these places at this time."}),
        );
    }
    // Labels.
    let fastest = (0..opts.len())
        .min_by_key(|&i| (opts[i].arrive_ts, opts[i].transfers))
        .unwrap();
    opts[fastest].labels.push("Fastest");
    let limit_arr = opts[fastest].arrive_ts + 25 * 60;
    let fw = opts[fastest].worst_level;
    let fe = opts[fastest].crowding_extra_minutes;
    if let Some(c) = (0..opts.len())
        .filter(|&i| {
            i != fastest && opts[i].depart_ts < when.ts + 10 * 60 && opts[i].arrive_ts <= limit_arr
        })
        .filter(|&i| {
            (opts[i].worst_level.is_some() && opts[i].worst_level < fw)
                || (opts[i].worst_level.is_some() && opts[i].crowding_extra_minutes + 3.0 < fe)
        })
        .min_by(|&a, &b| {
            (opts[a].crowding_extra_minutes + 0.1 * opts[a].minutes_from_request)
                .total_cmp(&(opts[b].crowding_extra_minutes + 0.1 * opts[b].minutes_from_request))
        })
    {
        opts[c].labels.push("Most comfortable");
    }
    // Only worth suggesting a later bus when nothing leaving now has seats.
    let seats_now = opts
        .iter()
        .any(|o| o.depart_ts < when.ts + 10 * 60 && o.worst_level == Some(Level::Seats));
    if fw.is_some_and(|w| w > Level::Seats) && !seats_now {
        if let Some(c) = (0..opts.len())
            .filter(|&i| {
                opts[i].depart_ts >= when.ts + 10 * 60 && opts[i].worst_level == Some(Level::Seats)
            })
            .min_by_key(|&i| opts[i].arrive_ts)
        {
            opts[c].labels.push("Leave later, seats likely");
        }
    }
    opts.sort_by(|a, b| a.score.total_cmp(&b.score));
    if let Some(o) = opts.first_mut() {
        o.recommended = true;
    }
    // Keep labelled options and fill up to max_options by score.
    let max = body.max_options.unwrap_or(4).max(1);
    let mut keep: Vec<JourneyOption> = Vec::new();
    let mut rest: Vec<JourneyOption> = Vec::new();
    for o in opts {
        if o.recommended || !o.labels.is_empty() {
            keep.push(o);
        } else {
            rest.push(o);
        }
    }
    for o in rest {
        if keep.len() >= max {
            break;
        }
        keep.push(o);
    }
    keep.sort_by(|a, b| a.score.total_cmp(&b.score));
    Ok(json!({
        "requested": {"depart": iso(when.ts), "daytype": when.dt, "comfort": comfort, "crowding_reluctance": lambda, "time": hhmm(when.secs)},
        "options": keep,
    }))
}

#[post("/v1/journeys")]
pub async fn journeys(st: State, body: web::Json<JourneyIn>) -> AppResult<HttpResponse> {
    let l = st.snap()?;
    let v = plan_journeys(&st, &l, &body).await?;
    Ok(HttpResponse::Ok().json(v))
}
