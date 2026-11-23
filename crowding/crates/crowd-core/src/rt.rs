//! Live bus positions (GTFS Realtime VehiclePositions) and what they say about bus supply.
//!
//! The Delhi feed reports every bus on the road every few seconds: its registration number,
//! route, the start time of its current trip and its position. It carries no occupancy.
//! It is used twice:
//! 1. Calibration (pipeline). Snapshots collected over some hours show how many buses
//!    actually run each route at each hour. Compared with the trips the timetable has under
//!    way at the same moments, this gives a delivery factor per route and hour, which
//!    scales the timetable to the buses that really run.
//! 2. Live (service). Where each bus is now, which stretch it is on, and how many buses run
//!    each route right now compared with usual.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Timelike};
use prost::Message;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::daytype::{DayCalendar, N_DAYTYPES};
use crate::gtfs::{parse_time, Gtfs, Stop};
use crate::tickets::ist;

// ---------- GTFS Realtime messages (the subset read and written here) ----------

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
    #[prost(message, optional, tag = "4")]
    pub vehicle: Option<VehiclePosition>,
}

#[derive(Clone, PartialEq, Message)]
pub struct VehiclePosition {
    #[prost(message, optional, tag = "1")]
    pub trip: Option<TripDescriptor>,
    #[prost(message, optional, tag = "2")]
    pub position: Option<Position>,
    #[prost(uint32, optional, tag = "3")]
    pub current_stop_sequence: Option<u32>,
    #[prost(uint64, optional, tag = "5")]
    pub timestamp: Option<u64>,
    #[prost(string, optional, tag = "7")]
    pub stop_id: Option<String>,
    #[prost(message, optional, tag = "8")]
    pub vehicle: Option<VehicleDescriptor>,
    #[prost(int32, optional, tag = "9")]
    pub occupancy_status: Option<i32>,
    #[prost(uint32, optional, tag = "10")]
    pub occupancy_percentage: Option<u32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct TripDescriptor {
    #[prost(string, optional, tag = "1")]
    pub trip_id: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub start_time: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub start_date: Option<String>,
    #[prost(int32, optional, tag = "4")]
    pub schedule_relationship: Option<i32>,
    #[prost(string, optional, tag = "5")]
    pub route_id: Option<String>,
    #[prost(uint32, optional, tag = "6")]
    pub direction_id: Option<u32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Position {
    #[prost(float, required, tag = "1")]
    pub latitude: f32,
    #[prost(float, required, tag = "2")]
    pub longitude: f32,
    #[prost(float, optional, tag = "3")]
    pub bearing: Option<f32>,
    #[prost(float, optional, tag = "5")]
    pub speed: Option<f32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct VehicleDescriptor {
    #[prost(string, optional, tag = "1")]
    pub id: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub label: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub license_plate: Option<String>,
}

// ---------- decoded feed ----------

#[derive(Debug, Clone, Serialize)]
pub struct LiveVehicle {
    /// Feed entity id.
    pub id: String,
    /// Registration number (the same as `vehicle_number` on tickets).
    pub label: String,
    pub route_id: String,
    pub trip_id: String,
    /// Start of the bus's current trip, seconds after midnight.
    pub trip_start: Option<u32>,
    pub lat: f64,
    pub lon: f64,
    /// Metres per second.
    pub speed: f32,
    pub bearing: Option<f32>,
    /// Unix time of the position.
    pub ts: i64,
}

#[derive(Debug, Clone)]
pub struct Feed {
    /// Unix time of the feed.
    pub ts: i64,
    pub vehicles: Vec<LiveVehicle>,
    pub message: FeedMessage,
}

/// Decode a VehiclePositions feed (plain or gzip-compressed).
pub fn decode(bytes: &[u8]) -> Result<Feed> {
    let unzipped;
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut v = Vec::new();
        flate2::read::GzDecoder::new(bytes).read_to_end(&mut v)?;
        unzipped = v;
        &unzipped[..]
    } else {
        bytes
    };
    let message = FeedMessage::decode(bytes).context("decode GTFS Realtime feed")?;
    let ts = message.header.timestamp.unwrap_or(0) as i64;
    let vehicles = message
        .entity
        .iter()
        .filter_map(|e| {
            let v = e.vehicle.as_ref()?;
            let p = v.position.as_ref()?;
            let t = v.trip.as_ref();
            let label = v
                .vehicle
                .as_ref()
                .and_then(|d| d.label.clone().or_else(|| d.id.clone()))
                .unwrap_or_else(|| e.id.clone());
            Some(LiveVehicle {
                id: e.id.clone(),
                label,
                route_id: t.and_then(|t| t.route_id.clone()).unwrap_or_default(),
                trip_id: t.and_then(|t| t.trip_id.clone()).unwrap_or_default(),
                trip_start: t
                    .and_then(|t| t.start_time.as_deref())
                    .and_then(|s| parse_time(s.as_bytes())),
                lat: p.latitude as f64,
                lon: p.longitude as f64,
                speed: p.speed.unwrap_or(0.0),
                bearing: p.bearing,
                ts: v.timestamp.map(|x| x as i64).unwrap_or(ts),
            })
        })
        .collect();
    Ok(Feed {
        ts,
        vehicles,
        message,
    })
}

/// Feed snapshot files (`*.pb`, `*.pb.gz`) under a directory, sorted by path.
pub fn snapshot_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).with_context(|| format!("read {}", d.display()))? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".pb") || n.ends_with(".pb.gz"))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

// ---------- placing a bus on its route ----------

/// A bus further than this from its route (straight lines between stops) is not counted
/// as running it.
pub const MAX_ROUTE_DIST_M: f64 = 500.0;
/// Within this of the first stop and not moving: waiting to start. Of the last: done.
const TERMINAL_M: f64 = 300.0;
/// Positions older than this (relative to the feed time) are ignored.
pub const MAX_AGE_SECS: i64 = 600;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Located {
    pub pattern: u32,
    /// Nearest stop on the pattern.
    pub stop_pos: usize,
    /// Stretch the bus is on: from stop `seg` to `seg + 1`.
    pub seg: usize,
    pub dist_m: f64,
    /// Running a trip: on its route, not waiting at the first stop, not done at the last.
    pub in_service: bool,
}

/// Place a bus on the nearest of the given patterns (pattern id, stops): the closest
/// stretch between consecutive stops, measured to the straight line between them.
pub fn locate<'a>(
    stops: &[Stop],
    patterns: impl IntoIterator<Item = (u32, &'a [u32])>,
    lat: f64,
    lon: f64,
    speed: f32,
) -> Option<Located> {
    // Local plane in metres around the bus.
    let kx = 111_320.0 * lat.to_radians().cos();
    let ky = 110_574.0;
    let xy = |s: &Stop| ((s.lon - lon) * kx, (s.lat - lat) * ky);
    let mut best: Option<Located> = None;
    for (pid, ps) in patterns {
        let n = ps.len();
        if n < 2 {
            continue;
        }
        let pts: Vec<(f64, f64)> = ps.iter().map(|&s| xy(&stops[s as usize])).collect();
        let mut seg = (f64::INFINITY, 0usize, 0.0f64);
        for k in 0..n - 1 {
            let ((ax, ay), (bx, by)) = (pts[k], pts[k + 1]);
            let (dx, dy) = (bx - ax, by - ay);
            let l2 = dx * dx + dy * dy;
            let t = if l2 > 0.0 {
                (-(ax * dx + ay * dy) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (px, py) = (ax + t * dx, ay + t * dy);
            let d = (px * px + py * py).sqrt();
            if d < seg.0 {
                seg = (d, k, t);
            }
        }
        let (dm, k, t) = seg;
        if best.as_ref().is_some_and(|b| b.dist_m <= dm) {
            continue;
        }
        let near = |i: usize| (pts[i].0 * pts[i].0 + pts[i].1 * pts[i].1).sqrt() <= TERMINAL_M;
        let waiting = k == 0 && near(0) && speed < 1.0;
        let done = k == n - 2 && near(n - 1);
        best = Some(Located {
            pattern: pid,
            stop_pos: if t < 0.5 { k } else { k + 1 },
            seg: k,
            dist_m: dm,
            in_service: dm <= MAX_ROUTE_DIST_M && !waiting && !done,
        });
    }
    best
}

/// Local (IST) date and seconds after midnight of a unix time.
pub fn local(ts: i64) -> (NaiveDate, u32) {
    let t = DateTime::from_timestamp(ts, 0)
        .unwrap_or_default()
        .with_timezone(&ist());
    (t.date_naive(), t.time().num_seconds_from_midnight())
}

// ---------- calibration of bus supply ----------

pub const HOURS: usize = 24;

/// Buses seen running in the live feed against trips the timetable has under way at the
/// same moments.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RtCalibration {
    pub snapshots: usize,
    pub first: Option<String>,
    pub last: Option<String>,
    pub dates: Vec<NaiveDate>,
    /// Sums over snapshots of buses running / timetabled trips under way, `[dt * 24 + hour]`.
    pub live: Vec<f64>,
    pub sched: Vec<f64>,
    /// All snapshots pooled.
    pub overall: f64,
    /// An hour needs this many timetabled trip-snapshots to get its own city factor.
    pub min_sched: f64,
    /// Per route: buses seen running and the number expected from the city factor (sums
    /// over snapshots), and the route factor relative to the city.
    pub route_live: Vec<f64>,
    pub route_expected: Vec<f64>,
    pub route_rel: Vec<f32>,
    /// Distinct buses seen running a route.
    pub vehicles: usize,
    /// Positions with a route that is in the timetable, and the share of those placed on it.
    pub with_route: u64,
    pub placed_share: f64,
}

impl RtCalibration {
    /// City-wide delivery factor for a day type and hour, falling back to the day type's
    /// observed hours, then to all snapshots.
    pub fn city(&self, dt: usize, h: usize) -> f64 {
        let i = dt * HOURS + h;
        if self.sched[i] >= self.min_sched {
            return self.live[i] / self.sched[i];
        }
        let r = dt * HOURS..(dt + 1) * HOURS;
        let (l, s) = r.fold((0.0, 0.0), |a, i| (a.0 + self.live[i], a.1 + self.sched[i]));
        if s >= self.min_sched {
            return l / s;
        }
        self.overall
    }

    /// Whether the hour had its own snapshots.
    pub fn observed(&self, dt: usize, h: usize) -> bool {
        self.sched[dt * HOURS + h] >= self.min_sched
    }

    /// Factor on the timetable for one route.
    pub fn factor(&self, route: usize, dt: usize, h: usize) -> f64 {
        self.city(dt, h) * self.route_rel.get(route).copied().unwrap_or(1.0) as f64
    }
}

/// Timetabled trips of each route under way at a moment.
pub struct UnderWay<'a> {
    g: &'a Gtfs,
    by_route: Vec<Vec<u32>>,
    active: FxHashMap<NaiveDate, Vec<bool>>,
}

impl<'a> UnderWay<'a> {
    pub fn new(g: &'a Gtfs) -> Self {
        let mut by_route = vec![Vec::new(); g.routes.len()];
        for (i, t) in g.trips.iter().enumerate() {
            if t.times.len() >= 2 {
                by_route[t.route as usize].push(i as u32);
            }
        }
        Self {
            g,
            by_route,
            active: FxHashMap::default(),
        }
    }

    fn ensure(&mut self, d: NaiveDate) {
        let g = self.g;
        self.active.entry(d).or_insert_with(|| {
            let mut cache: FxHashMap<&str, bool> = FxHashMap::default();
            g.trips
                .iter()
                .map(|t| {
                    *cache
                        .entry(t.service_id.as_str())
                        .or_insert_with(|| g.calendar.active(&t.service_id, d))
                })
                .collect()
        });
    }

    /// Trips of `route` running at `secs` on `date` (including last night's trips that run
    /// past midnight).
    pub fn count(&mut self, route: usize, date: NaiveDate, secs: u32) -> u32 {
        let prev = date - Duration::days(1);
        self.ensure(date);
        self.ensure(prev);
        let (today, yday) = (&self.active[&date], &self.active[&prev]);
        let mut n = 0;
        for &ti in &self.by_route[route] {
            let t = &self.g.trips[ti as usize];
            let (a, b) = (t.times[0], *t.times.last().unwrap());
            if (a <= secs && secs <= b && today[ti as usize])
                || (a <= secs + 86_400 && secs + 86_400 <= b && yday[ti as usize])
            {
                n += 1;
            }
        }
        n
    }
}

struct SnapCounts {
    dt: usize,
    h: usize,
    /// (route, buses running, trips under way)
    routes: Vec<(u32, u32, u32)>,
}

/// Calibrate bus supply from live feed snapshots.
pub fn calibrate(
    g: &Gtfs,
    cal: &DayCalendar,
    feeds: impl IntoIterator<Item = Feed>,
    route_pseudo: f64,
    min_sched: f64,
) -> RtCalibration {
    let route_idx: FxHashMap<&str, usize> = g
        .routes
        .iter()
        .enumerate()
        .map(|(i, r)| (r.route_id.as_str(), i))
        .collect();
    let mut under = UnderWay::new(g);
    let mut out = RtCalibration {
        live: vec![0.0; N_DAYTYPES * HOURS],
        sched: vec![0.0; N_DAYTYPES * HOURS],
        min_sched,
        route_live: vec![0.0; g.routes.len()],
        route_expected: vec![0.0; g.routes.len()],
        ..Default::default()
    };
    let mut snaps: Vec<SnapCounts> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();
    let mut dates: FxHashSet<NaiveDate> = FxHashSet::default();
    let (mut with_route, mut placed) = (0u64, 0u64);
    let (mut first, mut last) = (i64::MAX, i64::MIN);
    for f in feeds {
        if f.ts <= 0 {
            continue;
        }
        let (date, secs) = local(f.ts);
        let (dt, h) = (cal.day_type(date).idx(), (secs / 3600) as usize);
        let mut live: FxHashMap<u32, u32> = FxHashMap::default();
        for v in &f.vehicles {
            if f.ts - v.ts > MAX_AGE_SECS {
                continue;
            }
            let Some(&r) = route_idx.get(v.route_id.as_str()) else {
                continue;
            };
            with_route += 1;
            let pats = g.patterns_by_route[r]
                .iter()
                .map(|&p| (p, g.patterns[p as usize].stops.as_slice()));
            let Some(loc) = locate(&g.stops, pats, v.lat, v.lon, v.speed) else {
                continue;
            };
            if loc.dist_m <= MAX_ROUTE_DIST_M {
                placed += 1;
            }
            if loc.in_service {
                *live.entry(r as u32).or_default() += 1;
                seen.insert(v.label.clone());
            }
        }
        let mut routes = Vec::new();
        for r in 0..g.routes.len() {
            let s = under.count(r, date, secs);
            let l = live.get(&(r as u32)).copied().unwrap_or(0);
            if s > 0 || l > 0 {
                routes.push((r as u32, l, s));
                out.live[dt * HOURS + h] += l as f64;
                out.sched[dt * HOURS + h] += s as f64;
            }
        }
        snaps.push(SnapCounts { dt, h, routes });
        dates.insert(date);
        first = first.min(f.ts);
        last = last.max(f.ts);
    }
    out.snapshots = snaps.len();
    let (l, s): (f64, f64) = (out.live.iter().sum(), out.sched.iter().sum());
    out.overall = if s > 0.0 { l / s } else { 1.0 };
    for sc in &snaps {
        let c = out.city(sc.dt, sc.h);
        for &(r, l, s) in &sc.routes {
            out.route_live[r as usize] += l as f64;
            out.route_expected[r as usize] += c * s as f64;
        }
    }
    out.route_rel = out
        .route_live
        .iter()
        .zip(&out.route_expected)
        .map(|(&l, &e)| ((l + route_pseudo) / (e + route_pseudo)).clamp(1.0 / 3.0, 3.0) as f32)
        .collect();
    out.vehicles = seen.len();
    out.with_route = with_route;
    out.placed_share = if with_route > 0 {
        placed as f64 / with_route as f64
    } else {
        0.0
    };
    let fmt = |ts: i64| {
        DateTime::from_timestamp(ts, 0)
            .map(|t| t.with_timezone(&ist()).format("%Y-%m-%d %H:%M").to_string())
    };
    if out.snapshots > 0 {
        out.first = fmt(first);
        out.last = fmt(last);
    }
    let mut d: Vec<NaiveDate> = dates.into_iter().collect();
    d.sort();
    out.dates = d;
    out
}

/// Read every snapshot under `dir` and calibrate. Unreadable files are skipped.
pub fn calibrate_dir(
    g: &Gtfs,
    cal: &DayCalendar,
    dir: &Path,
    route_pseudo: f64,
    min_sched: f64,
) -> Result<RtCalibration> {
    let files = snapshot_files(dir)?;
    anyhow::ensure!(
        !files.is_empty(),
        "no live feed snapshots under {}",
        dir.display()
    );
    let mut bad = 0usize;
    let feeds = files.iter().filter_map(|p| {
        match std::fs::read(p)
            .map_err(anyhow::Error::from)
            .and_then(|b| decode(&b))
        {
            Ok(f) if !f.vehicles.is_empty() => Some(f),
            _ => {
                bad += 1;
                None
            }
        }
    });
    let c = calibrate(g, cal, feeds, route_pseudo, min_sched);
    if bad > 0 {
        tracing::warn!(bad, "skipped unreadable or empty live feed snapshots");
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_bytes(vehicles: &[(&str, &str, f32, f32, f32)], ts: u64) -> Vec<u8> {
        FeedMessage {
            header: FeedHeader {
                gtfs_realtime_version: "2.0".into(),
                incrementality: Some(0),
                timestamp: Some(ts),
            },
            entity: vehicles
                .iter()
                .map(|&(id, route, lat, lon, speed)| FeedEntity {
                    id: id.into(),
                    vehicle: Some(VehiclePosition {
                        trip: Some(TripDescriptor {
                            route_id: Some(route.into()),
                            start_time: Some("08:00:00".into()),
                            ..Default::default()
                        }),
                        position: Some(Position {
                            latitude: lat,
                            longitude: lon,
                            bearing: None,
                            speed: Some(speed),
                        }),
                        timestamp: Some(ts),
                        vehicle: Some(VehicleDescriptor {
                            label: Some(id.into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                })
                .collect(),
        }
        .encode_to_vec()
    }

    fn stops() -> Vec<Stop> {
        (0..4)
            .map(|i| Stop {
                stop_id: i.to_string(),
                name: format!("S{i}"),
                lat: 28.6,
                lon: 77.2 + 0.01 * i as f64,
            })
            .collect()
    }

    #[test]
    fn decodes_plain_and_gzip() {
        let b = feed_bytes(&[("DL1PC0001", "7", 28.6, 77.2, 5.0)], 1_795_442_705);
        let f = decode(&b).unwrap();
        assert_eq!(f.vehicles.len(), 1);
        assert_eq!(f.vehicles[0].label, "DL1PC0001");
        assert_eq!(f.vehicles[0].trip_start, Some(8 * 3600));
        let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut z, &b).unwrap();
        let g = decode(&z.finish().unwrap()).unwrap();
        assert_eq!(g.vehicles[0].route_id, "7");
    }

    #[test]
    fn locates_buses_and_spots_layovers() {
        let s = stops();
        let pat: Vec<u32> = vec![0, 1, 2, 3];
        let p = || [(0u32, pat.as_slice())];
        // Between stops 1 and 2, moving.
        let l = locate(&s, p(), 28.6, 77.214, 6.0).unwrap();
        assert_eq!((l.seg, l.in_service), (1, true));
        // Parked at the first stop: waiting, not running.
        let l = locate(&s, p(), 28.6, 77.2001, 0.0).unwrap();
        assert!(!l.in_service);
        // At the last stop: done.
        assert!(!locate(&s, p(), 28.6, 77.23, 4.0).unwrap().in_service);
        // Far from the route.
        assert!(!locate(&s, p(), 28.7, 77.21, 6.0).unwrap().in_service);
    }

    #[test]
    fn route_factor_shrinks_toward_the_city() {
        let c = RtCalibration {
            live: vec![0.0; N_DAYTYPES * HOURS],
            sched: vec![0.0; N_DAYTYPES * HOURS],
            overall: 1.2,
            min_sched: 10.0,
            route_rel: vec![0.5],
            ..Default::default()
        };
        // No hour observed: the overall factor, times the route's.
        assert!((c.factor(0, 0, 8) - 0.6).abs() < 1e-9);
        assert!((c.factor(9, 0, 8) - 1.2).abs() < 1e-9);
    }
}
