//! Assign tickets to GTFS: route, stop pattern, boarding and alighting positions, and the
//! nearest scheduled trip.
//!
//! Order of attempts for each ticket:
//! 1. The ticket's route, with both stops found in order on one of its patterns.
//! 2. The same route in the other direction, when the stops appear in reverse order
//!    (direction label wrong on the ticket).
//! 3. The ticket's route with only the boarding stop found. The alighting stop is then
//!    imputed: candidates are the downstream stops whose distance from the boarding stop
//!    fits the distance range observed for the same fare, weighted by where riders who
//!    boarded at that stop were seen to get off.

use std::collections::BTreeMap;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

use crate::config::ModelConfig;
use crate::geo::haversine_m;
use crate::gtfs::Gtfs;
use crate::names::{
    alternates, norm_route, norm_stop, opposite_direction, route_candidates, score,
};
use crate::tickets::{Interner, Ticket};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Both stops matched by name on the ticket's route.
    Exact,
    /// Both stops matched on the opposite direction of the ticket's route.
    DirectionFixed,
    /// Boarding stop matched; alighting stop imputed from fare and history.
    Imputed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reject {
    RouteNotInGtfs,
    StartStopNotOnRoute,
    NoDownstreamStop,
    NoTrip,
}

/// A stop of a pattern that matches a ticket's stop name.
#[derive(Debug, Clone, Copy)]
struct Hit {
    pos: u16,
    score: f32,
    geo: bool,
}

#[derive(Debug, Clone)]
pub struct Assigned {
    /// Index of the ticket in the input slice.
    pub ticket: u32,
    pub pattern: u32,
    pub board: u16,
    /// Alighting positions with weights summing to one.
    pub alight: Vec<(u16, f32)>,
    pub pax: f32,
    pub trip: u32,
    pub method: Method,
    pub score: f32,
    pub geo: bool,
}

impl Assigned {
    /// The single alighting position if not imputed.
    pub fn alight_exact(&self) -> Option<u16> {
        (self.alight.len() == 1 && self.method != Method::Imputed).then(|| self.alight[0].0)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FareBand {
    pub n: u64,
    pub p05_km: f32,
    pub p25_km: f32,
    pub median_km: f32,
    pub p75_km: f32,
    pub p95_km: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MatchReport {
    pub tickets: u64,
    pub pax: f64,
    pub assigned: u64,
    pub assigned_pax: f64,
    pub by_method: BTreeMap<String, u64>,
    pub rejected: BTreeMap<String, u64>,
    pub geo_matches: u64,
    /// Trip distance by fare paid per passenger, from tickets with both stops matched.
    /// Distance rising with fare is evidence that stop matching is right.
    pub fare_bands: BTreeMap<String, FareBand>,
    /// Ticket routes that are not in the GTFS feed, with ticket counts (top 50).
    pub unknown_routes: Vec<(String, u64)>,
}

/// Matches ticket names against GTFS. Name matches are computed once per
/// (pattern, name) pair, in parallel.
pub struct Matcher<'g> {
    g: &'g Gtfs,
    min_score: f64,
    geo_radius_m: f64,
    routes_by_name: FxHashMap<String, Vec<u32>>,
    stop_alts: Vec<Vec<String>>,
    global_by_name: FxHashMap<String, Vec<u32>>,
}

impl<'g> Matcher<'g> {
    pub fn new(g: &'g Gtfs, cfg: &ModelConfig) -> Self {
        let mut routes_by_name: FxHashMap<String, Vec<u32>> = FxHashMap::default();
        for (i, r) in g.routes.iter().enumerate() {
            routes_by_name
                .entry(norm_route(r.name()))
                .or_default()
                .push(i as u32);
        }
        let stop_alts: Vec<Vec<String>> = g.stops.par_iter().map(|s| alternates(&s.name)).collect();
        let mut global_by_name: FxHashMap<String, Vec<u32>> = FxHashMap::default();
        for (i, alts) in stop_alts.iter().enumerate() {
            for a in alts {
                global_by_name.entry(a.clone()).or_default().push(i as u32);
            }
        }
        Self {
            g,
            min_score: cfg.matching.min_score,
            geo_radius_m: cfg.matching.geo_radius_m,
            routes_by_name,
            stop_alts,
            global_by_name,
        }
    }

    /// GTFS route indices for a ticket route name.
    pub fn routes_for(&self, ticket_route: &str) -> Option<(String, &[u32])> {
        for cand in route_candidates(ticket_route) {
            if let Some(v) = self.routes_by_name.get(&cand) {
                return Some((cand, v.as_slice()));
            }
        }
        None
    }

    fn patterns_for_name(&self, route_name: &str) -> Vec<u32> {
        self.routes_by_name
            .get(route_name)
            .map(|rs| {
                rs.iter()
                    .flat_map(|&r| self.g.patterns_by_route[r as usize].iter().copied())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn hits(&self, pattern: u32, name_norm: &str) -> Vec<Hit> {
        let p = &self.g.patterns[pattern as usize];
        let mut out: Vec<Hit> = p
            .stops
            .iter()
            .enumerate()
            .filter_map(|(pos, &s)| {
                let sc = score(name_norm, &self.stop_alts[s as usize]);
                (sc >= self.min_score).then_some(Hit {
                    pos: pos as u16,
                    score: sc as f32,
                    geo: false,
                })
            })
            .collect();
        if out.is_empty() {
            // Geographic fallback: a stop elsewhere in the feed with exactly this name,
            // close to a stop of this pattern.
            if let Some(cands) = self.global_by_name.get(name_norm) {
                let mut best: Option<(f64, usize)> = None;
                for &c in cands {
                    let cs = &self.g.stops[c as usize];
                    for (pos, &s) in p.stops.iter().enumerate() {
                        let ps = &self.g.stops[s as usize];
                        let d = haversine_m(cs.lat, cs.lon, ps.lat, ps.lon);
                        if d <= self.geo_radius_m && best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, pos));
                        }
                    }
                }
                if let Some((_, pos)) = best {
                    out.push(Hit {
                        pos: pos as u16,
                        score: self.min_score as f32,
                        geo: true,
                    });
                }
            }
        }
        out
    }
}

type HitCache = FxHashMap<(u32, u32), Vec<Hit>>;
/// Patterns of a ticket's route, and of the same route in the other direction.
type RoutePatterns = (Vec<u32>, Vec<u32>);

#[derive(Debug, Clone, Copy)]
struct Resolution {
    pattern: u32,
    board: u16,
    alight: Option<u16>,
    score: f32,
    geo: bool,
    method: Method,
}

fn best_pair(hs: &[Hit], he: &[Hit]) -> Option<(Hit, Hit)> {
    let mut best: Option<(f32, i32, Hit, Hit)> = None;
    for b in hs {
        for a in he.iter().filter(|a| a.pos > b.pos) {
            let key = (b.score.min(a.score), -((a.pos - b.pos) as i32));
            if best.is_none_or(|(s, l, _, _)| (key.0, key.1) > (s, l)) {
                best = Some((key.0, key.1, *b, *a));
            }
        }
    }
    best.map(|(_, _, b, a)| (b, a))
}

/// Assign all tickets. Returns assignments (in ticket order) and a report.
pub fn assign_all(
    g: &Gtfs,
    cfg: &ModelConfig,
    tickets: &[Ticket],
    names: &Interner,
) -> (Vec<Assigned>, MatchReport) {
    let m = Matcher::new(g, cfg);
    let mut report = MatchReport {
        tickets: tickets.len() as u64,
        ..Default::default()
    };

    // Route name -> (forward patterns, opposite patterns), per distinct ticket route.
    let mut route_pats: FxHashMap<u32, Option<RoutePatterns>> = FxHashMap::default();
    for t in tickets {
        route_pats.entry(t.route).or_insert_with(|| {
            let (name, _) = m.routes_for(names.get(t.route))?;
            let fwd = m.patterns_for_name(&name);
            let opp = opposite_direction(&name)
                .map(|o| m.patterns_for_name(&o))
                .unwrap_or_default();
            Some((fwd, opp))
        });
    }

    // All (pattern, name) pairs we need, computed in parallel.
    let mut need: FxHashSet<(u32, u32)> = FxHashSet::default();
    for t in tickets {
        if let Some(Some((fwd, opp))) = route_pats.get(&t.route) {
            for &p in fwd.iter().chain(opp.iter()) {
                need.insert((p, t.start));
                if let Some(e) = t.end {
                    need.insert((p, e));
                }
            }
        }
    }
    let mut norm_cache: FxHashMap<u32, String> = FxHashMap::default();
    for &(_, n) in &need {
        norm_cache
            .entry(n)
            .or_insert_with(|| norm_stop(names.get(n)));
    }
    let need: Vec<(u32, u32)> = need.into_iter().collect();
    let cache: HitCache = need
        .par_iter()
        .map(|&(p, n)| ((p, n), m.hits(p, &norm_cache[&n])))
        .collect();
    let hits = |p: u32, n: u32| cache.get(&(p, n)).map(Vec::as_slice).unwrap_or(&[]);

    // First pass: resolve stops.
    let resolve = |t: &Ticket| -> Result<Resolution, Reject> {
        let Some(Some((fwd, opp))) = route_pats.get(&t.route) else {
            return Err(Reject::RouteNotInGtfs);
        };
        let pick = |pats: &[u32], method: Method| -> Option<Resolution> {
            let e = t.end?;
            pats.iter()
                .filter_map(|&p| {
                    best_pair(hits(p, t.start), hits(p, e)).map(|(b, a)| Resolution {
                        pattern: p,
                        board: b.pos,
                        alight: Some(a.pos),
                        score: b.score.min(a.score),
                        geo: b.geo || a.geo,
                        method,
                    })
                })
                .max_by(|x, y| {
                    let tx = g.patterns[x.pattern as usize].trips.len();
                    let ty = g.patterns[y.pattern as usize].trips.len();
                    x.score.total_cmp(&y.score).then(tx.cmp(&ty))
                })
        };
        if let Some(r) = pick(fwd, Method::Exact) {
            return Ok(r);
        }
        if let Some(r) = pick(opp, Method::DirectionFixed) {
            return Ok(r);
        }
        // Boarding stop only: choose the earliest strong match with downstream stops left.
        let mut best: Option<Resolution> = None;
        for &p in fwd {
            let n = g.patterns[p as usize].stops.len() as u16;
            for h in hits(p, t.start).iter().filter(|h| h.pos + 1 < n) {
                if best.is_none_or(|b| h.score > b.score) {
                    best = Some(Resolution {
                        pattern: p,
                        board: h.pos,
                        alight: None,
                        score: h.score,
                        geo: h.geo,
                        method: Method::Imputed,
                    });
                }
            }
        }
        best.ok_or(Reject::StartStopNotOnRoute)
    };
    let resolved: Vec<Result<Resolution, Reject>> = tickets.par_iter().map(resolve).collect();

    // Learn fare -> distance bands and per-boarding-stop alighting histograms from exact
    // matches.
    let mut fare_d: FxHashMap<(bool, i32), Vec<f32>> = FxHashMap::default();
    let mut alight_hist: FxHashMap<(u32, u16), FxHashMap<u16, f32>> = FxHashMap::default();
    let mut alight_total: FxHashMap<u32, FxHashMap<u16, f32>> = FxHashMap::default();
    for (t, r) in tickets.iter().zip(&resolved) {
        let Ok(r) = r else { continue };
        let Some(a) = r.alight else { continue };
        let p = &g.patterns[r.pattern as usize];
        let km = (p.dist_m[a as usize] - p.dist_m[r.board as usize]) / 1000.0;
        let key = (t.is_ac.unwrap_or(true), (t.fare_per_pax() * 2.0) as i32);
        fare_d.entry(key).or_default().push(km);
        *alight_hist
            .entry((r.pattern, r.board))
            .or_default()
            .entry(a)
            .or_default() += t.pax;
        *alight_total
            .entry(r.pattern)
            .or_default()
            .entry(a)
            .or_default() += t.pax;
    }
    let mut bands: FxHashMap<(bool, i32), FareBand> = FxHashMap::default();
    for (k, v) in fare_d.iter_mut() {
        v.sort_by(f32::total_cmp);
        let q = |f: f64| v[((v.len() - 1) as f64 * f).round() as usize];
        let band = FareBand {
            n: v.len() as u64,
            p05_km: q(0.05),
            p25_km: q(0.25),
            median_km: q(0.5),
            p75_km: q(0.75),
            p95_km: q(0.95),
        };
        if band.n >= 200 {
            let label = format!(
                "{} fare {:.1}",
                if k.0 { "AC" } else { "non-AC" },
                k.1 as f32 / 2.0
            );
            report.fare_bands.insert(label, band.clone());
        }
        bands.insert(*k, band);
    }

    // Second pass: build assignments.
    let spread = cfg.alight_spread.clamp(0.0, 1.0) as f32;
    let mut out = Vec::with_capacity(tickets.len());
    let mut unknown: FxHashMap<u32, u64> = FxHashMap::default();
    for (i, (t, r)) in tickets.iter().zip(resolved).enumerate() {
        report.pax += t.pax as f64;
        let r = match r {
            Ok(r) => r,
            Err(e) => {
                if e == Reject::RouteNotInGtfs {
                    *unknown.entry(t.route).or_default() += 1;
                }
                *report.rejected.entry(format!("{e:?}")).or_default() += 1;
                continue;
            }
        };
        let p = &g.patterns[r.pattern as usize];
        let n = p.stops.len() as u16;
        let alight: Vec<(u16, f32)> = match r.alight {
            Some(a) if spread > 0.0 => {
                let mut v = vec![(a, 1.0 - spread)];
                let lo = if a > r.board + 1 { Some(a - 1) } else { None };
                let hi = if a + 1 < n { Some(a + 1) } else { None };
                match (lo, hi) {
                    (Some(l), Some(h)) => {
                        v.push((l, spread / 2.0));
                        v.push((h, spread / 2.0));
                    }
                    (Some(x), None) | (None, Some(x)) => v.push((x, spread)),
                    (None, None) => v[0].1 = 1.0,
                }
                v
            }
            Some(a) => vec![(a, 1.0)],
            None => {
                let key = (t.is_ac.unwrap_or(true), (t.fare_per_pax() * 2.0) as i32);
                let band = bands.get(&key).filter(|b| b.n >= 30);
                let d0 = p.dist_m[r.board as usize];
                let cands: Vec<u16> = ((r.board + 1)..n)
                    .filter(|&a| {
                        let km = (p.dist_m[a as usize] - d0) / 1000.0;
                        band.is_none_or(|b| km >= b.p05_km && km <= b.p95_km)
                    })
                    .collect();
                let cands = if cands.is_empty() {
                    // Fall back to the stop closest to the band's median distance.
                    let target = band.map(|b| b.median_km).unwrap_or(3.0);
                    ((r.board + 1)..n)
                        .min_by(|&a, &b| {
                            let da = ((p.dist_m[a as usize] - d0) / 1000.0 - target).abs();
                            let db = ((p.dist_m[b as usize] - d0) / 1000.0 - target).abs();
                            da.total_cmp(&db)
                        })
                        .into_iter()
                        .collect()
                } else {
                    cands
                };
                if cands.is_empty() {
                    *report
                        .rejected
                        .entry(format!("{:?}", Reject::NoDownstreamStop))
                        .or_default() += 1;
                    continue;
                }
                let hist = alight_hist.get(&(r.pattern, r.board));
                let total = alight_total.get(&r.pattern);
                let tot_sum: f32 = total.map(|m| m.values().sum()).unwrap_or(0.0).max(1.0);
                let mut w: Vec<(u16, f32)> = cands
                    .iter()
                    .map(|&a| {
                        let h = hist.and_then(|m| m.get(&a)).copied().unwrap_or(0.0);
                        let prior = total.and_then(|m| m.get(&a)).copied().unwrap_or(0.0) / tot_sum;
                        (a, h + 1.0 * prior + 0.01)
                    })
                    .collect();
                let s: f32 = w.iter().map(|x| x.1).sum();
                for x in &mut w {
                    x.1 /= s;
                }
                w
            }
        };
        let Some(trip) = g.nearest_trip(r.pattern, r.board as usize, t.secs) else {
            *report
                .rejected
                .entry(format!("{:?}", Reject::NoTrip))
                .or_default() += 1;
            continue;
        };
        report.assigned += 1;
        report.assigned_pax += t.pax as f64;
        *report
            .by_method
            .entry(format!("{:?}", r.method))
            .or_default() += 1;
        if r.geo {
            report.geo_matches += 1;
        }
        out.push(Assigned {
            ticket: i as u32,
            pattern: r.pattern,
            board: r.board,
            alight,
            pax: t.pax,
            trip,
            method: r.method,
            score: r.score,
            geo: r.geo,
        });
    }
    let mut unk: Vec<(String, u64)> = unknown
        .into_iter()
        .map(|(r, n)| (names.get(r).to_string(), n))
        .collect();
    unk.sort_by_key(|x| std::cmp::Reverse(x.1));
    unk.truncate(50);
    report.unknown_routes = unk;
    (out, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit as fixture;
    use crate::tickets::{ist, RawTicket};

    fn raw(route: &str, a: &str, b: Option<&str>, at: &str, amount: f64) -> RawTicket {
        RawTicket {
            passenger_count: Some(1.0),
            amount: Some(amount),
            vehicle_number: None,
            ticket_type_id: Some(1),
            start_stop_name: a.into(),
            end_stop_name: b.map(str::to_string),
            route: route.into(),
            created_at: format!("2026-10-05 {at}+05:30"),
            is_ac: Some(true),
        }
    }

    #[test]
    fn exact_direction_fix_imputation_and_rejects() {
        let g = Gtfs::load(&fixture::write_feed()).unwrap();
        let cfg = ModelConfig::default();
        let mut names = Interner::default();
        let raws = [
            raw("34UP", "Anand Vihar Isbt", Some("Ito"), "08:01:00", 13.5),
            raw("34UP", "ITO", Some("Laxmi Nagar Xing"), "18:30:00", 9.0),
            raw("34 UP", "Laxmi Nagar Crossing", None, "09:12:00", 9.0),
            raw("99UP", "ITO", Some("Mandi House"), "09:00:00", 9.0),
            raw("34UP", "Nowhere", Some("ITO"), "09:00:00", 9.0),
        ];
        let tickets: Vec<Ticket> = raws
            .iter()
            .map(|r| r.parse(&mut names, ist()).unwrap())
            .collect();
        let (a, rep) = assign_all(&g, &cfg, &tickets, &names);
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].method, Method::Exact);
        assert_eq!((a[0].board, a[0].alight_exact()), (0, Some(2)));
        assert_eq!(a[1].method, Method::DirectionFixed);
        assert_eq!(g.route_of_pattern(a[1].pattern).long_name, "34DOWN");
        assert_eq!(a[2].method, Method::Imputed);
        assert!(a[2].alight.iter().all(|(p, _)| *p > a[2].board));
        let w: f32 = a[2].alight.iter().map(|x| x.1).sum();
        assert!((w - 1.0).abs() < 1e-5);
        assert_eq!(rep.rejected.get("RouteNotInGtfs"), Some(&1));
        assert_eq!(rep.rejected.get("StartStopNotOnRoute"), Some(&1));
        // The 08:01 ticket boards the 08:00 trip.
        assert_eq!(g.trips[a[0].trip as usize].times[0], 8 * 3600);
    }
}
