//! Minimal static GTFS loader: stops, routes, trips, stop times and calendars, with trips
//! grouped into stop patterns (a route may have several).

use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, NaiveDate};
use csv::{ByteRecord, StringRecord};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::geo::haversine_m;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stop {
    pub stop_id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub route_id: String,
    pub short_name: String,
    pub long_name: String,
    pub agency_id: String,
}

impl Route {
    /// The name riders and tickets use: the short name if set, else the long name.
    pub fn name(&self) -> &str {
        if self.short_name.trim().is_empty() {
            self.long_name.trim()
        } else {
            self.short_name.trim()
        }
    }
}

#[derive(Debug, Clone)]
pub struct Trip {
    pub trip_id: String,
    pub route: u32,
    pub service_id: String,
    pub pattern: u32,
    /// Departure time at each position of the pattern, seconds after midnight of the
    /// service day (may exceed 24 h).
    pub times: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Pattern {
    pub route: u32,
    pub stops: Vec<u32>,
    /// Cumulative straight-line distance along the pattern, metres.
    pub dist_m: Vec<f32>,
    /// Trip indices, sorted by departure from the first stop.
    pub trips: Vec<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Calendar {
    /// service_id -> (Mon..Sun, start, end)
    pub weekly: FxHashMap<String, ([bool; 7], NaiveDate, NaiveDate)>,
    pub added: FxHashMap<String, Vec<NaiveDate>>,
    pub removed: FxHashMap<String, Vec<NaiveDate>>,
}

impl Calendar {
    pub fn active(&self, service_id: &str, d: NaiveDate) -> bool {
        if self.removed.get(service_id).is_some_and(|v| v.contains(&d)) {
            return false;
        }
        if self.added.get(service_id).is_some_and(|v| v.contains(&d)) {
            return true;
        }
        match self.weekly.get(service_id) {
            Some((days, start, end)) => {
                d >= *start && d <= *end && days[d.weekday().num_days_from_monday() as usize]
            }
            None => false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Gtfs {
    pub stops: Vec<Stop>,
    pub stop_idx: FxHashMap<String, u32>,
    pub routes: Vec<Route>,
    pub route_idx: FxHashMap<String, u32>,
    pub trips: Vec<Trip>,
    pub trip_idx: FxHashMap<String, u32>,
    pub patterns: Vec<Pattern>,
    pub patterns_by_route: Vec<Vec<u32>>,
    pub calendar: Calendar,
    pub feed_version: String,
}

struct Cols(StringRecord);

impl Cols {
    fn get(&self, name: &str) -> Result<usize> {
        self.opt(name)
            .ok_or_else(|| anyhow!("missing column {name}"))
    }
    fn opt(&self, name: &str) -> Option<usize> {
        self.0
            .iter()
            .position(|h| h.trim_start_matches('\u{feff}').trim() == name)
    }
}

fn reader(dir: &Path, file: &str) -> Result<Option<(csv::Reader<File>, Cols)>> {
    let p = dir.join(file);
    if !p.exists() {
        return Ok(None);
    }
    let mut r = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_path(&p)
        .with_context(|| format!("open {}", p.display()))?;
    let h = r.headers()?.clone();
    Ok(Some((r, Cols(h))))
}

fn required(dir: &Path, file: &str) -> Result<(csv::Reader<File>, Cols)> {
    reader(dir, file)?.ok_or_else(|| anyhow!("{} has no {file}", dir.display()))
}

/// Parse `HH:MM:SS` (hours may exceed 23).
pub fn parse_time(s: &[u8]) -> Option<u32> {
    let s = std::str::from_utf8(s).ok()?.trim();
    let mut it = s.split(':');
    let h: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let sec: u32 = it.next().unwrap_or("0").parse().ok()?;
    Some(h * 3600 + m * 60 + sec)
}

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y%m%d").with_context(|| format!("bad date {s}"))
}

impl Gtfs {
    pub fn load(dir: &Path) -> Result<Gtfs> {
        let mut g = Gtfs::default();

        // stops
        let (mut r, c) = required(dir, "stops.txt")?;
        let (ci, cn, cla, clo) = (
            c.get("stop_id")?,
            c.get("stop_name")?,
            c.get("stop_lat")?,
            c.get("stop_lon")?,
        );
        for rec in r.records() {
            let rec = rec?;
            let stop = Stop {
                stop_id: rec.get(ci).unwrap_or("").to_string(),
                name: rec.get(cn).unwrap_or("").to_string(),
                lat: rec.get(cla).unwrap_or("0").parse().unwrap_or(0.0),
                lon: rec.get(clo).unwrap_or("0").parse().unwrap_or(0.0),
            };
            g.stop_idx
                .insert(stop.stop_id.clone(), g.stops.len() as u32);
            g.stops.push(stop);
        }

        // routes
        let (mut r, c) = required(dir, "routes.txt")?;
        let ci = c.get("route_id")?;
        let (cs, cl, ca) = (
            c.opt("route_short_name"),
            c.opt("route_long_name"),
            c.opt("agency_id"),
        );
        for rec in r.records() {
            let rec = rec?;
            let f = |i: Option<usize>| i.and_then(|i| rec.get(i)).unwrap_or("").to_string();
            let route = Route {
                route_id: rec.get(ci).unwrap_or("").to_string(),
                short_name: f(cs),
                long_name: f(cl),
                agency_id: f(ca),
            };
            g.route_idx
                .insert(route.route_id.clone(), g.routes.len() as u32);
            g.routes.push(route);
        }

        // trips
        let (mut r, c) = required(dir, "trips.txt")?;
        let (cr, cs, ct) = (c.get("route_id")?, c.get("service_id")?, c.get("trip_id")?);
        for rec in r.records() {
            let rec = rec?;
            let Some(&route) = g.route_idx.get(rec.get(cr).unwrap_or("")) else {
                continue;
            };
            let trip = Trip {
                trip_id: rec.get(ct).unwrap_or("").to_string(),
                route,
                service_id: rec.get(cs).unwrap_or("").to_string(),
                pattern: u32::MAX,
                times: Vec::new(),
            };
            g.trip_idx
                .insert(trip.trip_id.clone(), g.trips.len() as u32);
            g.trips.push(trip);
        }

        // stop_times: (trip, seq, stop, time or MAX)
        let (mut r, c) = required(dir, "stop_times.txt")?;
        let (ct, cs, cq) = (
            c.get("trip_id")?,
            c.get("stop_id")?,
            c.get("stop_sequence")?,
        );
        let (ca, cd) = (c.opt("arrival_time"), c.opt("departure_time"));
        let mut rows: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(1 << 20);
        let mut rec = ByteRecord::new();
        while r.read_byte_record(&mut rec)? {
            let tid = std::str::from_utf8(rec.get(ct).unwrap_or(b"")).unwrap_or("");
            let Some(&trip) = g.trip_idx.get(tid) else {
                continue;
            };
            let sid = std::str::from_utf8(rec.get(cs).unwrap_or(b"")).unwrap_or("");
            let Some(&stop) = g.stop_idx.get(sid) else {
                continue;
            };
            let seq: u32 = std::str::from_utf8(rec.get(cq).unwrap_or(b"0"))
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap_or(0);
            let t = cd
                .and_then(|i| rec.get(i))
                .and_then(parse_time)
                .or_else(|| ca.and_then(|i| rec.get(i)).and_then(parse_time))
                .unwrap_or(u32::MAX);
            rows.push((trip, seq, stop, t));
        }
        rows.sort_unstable();

        let mut pattern_key: FxHashMap<(u32, Vec<u32>), u32> = FxHashMap::default();
        let mut i = 0;
        while i < rows.len() {
            let trip = rows[i].0;
            let mut j = i;
            while j < rows.len() && rows[j].0 == trip {
                j += 1;
            }
            let stops: Vec<u32> = rows[i..j].iter().map(|r| r.2).collect();
            let mut times: Vec<u32> = rows[i..j].iter().map(|r| r.3).collect();
            interpolate_missing(&mut times);
            i = j;
            if stops.len() < 2 || times.contains(&u32::MAX) {
                continue;
            }
            let route = g.trips[trip as usize].route;
            let key = (route, stops);
            let next_id = g.patterns.len() as u32;
            let pid = *pattern_key.entry(key.clone()).or_insert_with(|| {
                let dist_m = cumulative_distance(&g.stops, &key.1);
                g.patterns.push(Pattern {
                    route,
                    stops: key.1.clone(),
                    dist_m,
                    trips: Vec::new(),
                });
                next_id
            });
            g.patterns[pid as usize].trips.push(trip);
            let t = &mut g.trips[trip as usize];
            t.pattern = pid;
            t.times = times;
        }
        for p in &mut g.patterns {
            let trips = &g.trips;
            p.trips.sort_by_key(|&t| trips[t as usize].times[0]);
        }
        g.patterns_by_route = vec![Vec::new(); g.routes.len()];
        for (pid, p) in g.patterns.iter().enumerate() {
            g.patterns_by_route[p.route as usize].push(pid as u32);
        }

        // calendars
        if let Some((mut r, c)) = reader(dir, "calendar.txt")? {
            let days = [
                "monday",
                "tuesday",
                "wednesday",
                "thursday",
                "friday",
                "saturday",
                "sunday",
            ];
            let di: Vec<usize> = days.iter().map(|d| c.get(d)).collect::<Result<_>>()?;
            let (cs, c0, c1) = (
                c.get("service_id")?,
                c.get("start_date")?,
                c.get("end_date")?,
            );
            for rec in r.records() {
                let rec = rec?;
                let mut mask = [false; 7];
                for (k, &i) in di.iter().enumerate() {
                    mask[k] = rec.get(i).unwrap_or("0").trim() == "1";
                }
                g.calendar.weekly.insert(
                    rec.get(cs).unwrap_or("").to_string(),
                    (
                        mask,
                        parse_date(rec.get(c0).unwrap_or(""))?,
                        parse_date(rec.get(c1).unwrap_or(""))?,
                    ),
                );
            }
        }
        if let Some((mut r, c)) = reader(dir, "calendar_dates.txt")? {
            let (cs, cd, ce) = (
                c.get("service_id")?,
                c.get("date")?,
                c.get("exception_type")?,
            );
            for rec in r.records() {
                let rec = rec?;
                let sid = rec.get(cs).unwrap_or("").to_string();
                let d = parse_date(rec.get(cd).unwrap_or(""))?;
                match rec.get(ce).unwrap_or("").trim() {
                    "1" => g.calendar.added.entry(sid).or_default().push(d),
                    "2" => g.calendar.removed.entry(sid).or_default().push(d),
                    _ => {}
                }
            }
        }

        if let Some((mut r, c)) = reader(dir, "feed_info.txt")? {
            if let (Some(rec), Some(i)) = (r.records().next(), c.opt("feed_version")) {
                g.feed_version = rec?.get(i).unwrap_or("").to_string();
            }
        }

        tracing::info!(
            stops = g.stops.len(),
            routes = g.routes.len(),
            trips = g.trips.len(),
            patterns = g.patterns.len(),
            "loaded GTFS"
        );
        Ok(g)
    }

    pub fn route_of_pattern(&self, p: u32) -> &Route {
        &self.routes[self.patterns[p as usize].route as usize]
    }

    /// Index of the scheduled trip on `pattern` whose time at `pos` is nearest `t`.
    pub fn nearest_trip(&self, pattern: u32, pos: usize, t: u32) -> Option<u32> {
        let p = &self.patterns[pattern as usize];
        if p.trips.is_empty() {
            return None;
        }
        let key = |ti: u32| self.trips[ti as usize].times[pos];
        let i = p.trips.partition_point(|&ti| key(ti) < t);
        let mut best = None;
        let mut best_d = u32::MAX;
        for j in [i.saturating_sub(1), i.min(p.trips.len() - 1)] {
            let ti = p.trips[j];
            let d = key(ti).abs_diff(t);
            if d < best_d {
                best_d = d;
                best = Some(ti);
            }
        }
        best
    }
}

fn interpolate_missing(times: &mut [u32]) {
    let n = times.len();
    let mut last: Option<usize> = None;
    for i in 0..n {
        if times[i] == u32::MAX {
            continue;
        }
        if let Some(l) = last {
            if i > l + 1 {
                let (a, b) = (times[l] as f64, times[i] as f64);
                for k in (l + 1)..i {
                    let f = (k - l) as f64 / (i - l) as f64;
                    times[k] = (a + f * (b - a)).round() as u32;
                }
            }
        }
        last = Some(i);
    }
}

fn cumulative_distance(stops: &[Stop], seq: &[u32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(seq.len());
    let mut acc = 0.0f64;
    out.push(0.0);
    for w in seq.windows(2) {
        let (a, b) = (&stops[w[0] as usize], &stops[w[1] as usize]);
        acc += haversine_m(a.lat, a.lon, b.lat, b.lon);
        out.push(acc as f32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit as fixture;

    #[test]
    fn loads_fixture_into_patterns() {
        let dir = fixture::write_feed();
        let g = Gtfs::load(&dir).unwrap();
        assert_eq!(g.routes.len(), 2);
        assert_eq!(g.patterns.len(), 2);
        let p = &g.patterns[g.patterns_by_route[0][0] as usize];
        assert_eq!(p.stops.len(), 5);
        assert_eq!(p.trips.len(), 6);
        assert!(p.dist_m[4] > 9_000.0);
        let t = &g.trips[p.trips[0] as usize];
        assert_eq!(t.times[0], 7 * 3600);
        assert_eq!(t.times[2], 7 * 3600 + 20 * 60);
        let d = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        assert!(g.calendar.active("all", d));
        let near = g
            .nearest_trip(g.patterns_by_route[0][0], 1, 8 * 3600 + 12 * 60)
            .unwrap();
        assert_eq!(g.trips[near as usize].times[0], 8 * 3600);
    }

    #[test]
    fn interpolates_missing_times() {
        let mut t = vec![100, u32::MAX, u32::MAX, 400];
        interpolate_missing(&mut t);
        assert_eq!(t, vec![100, 200, 300, 400]);
    }
}
