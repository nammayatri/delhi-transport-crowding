//! Route groups: the same route number run by two operators.
//!
//! The timetable lists several route names twice, once per operator, each with its own
//! stop ids. Riders and tickets do not tell them apart, so a ticket on "106DOWN" may come
//! from either operator's bus. We merge the copies onto one canonical pattern (the one
//! with most trips): every other copy's stops are mapped to the nearest canonical stop
//! along the route, and both its tickets and its buses are counted on the canonical
//! pattern. Copies whose stops do not line up (median distance above a threshold) stay
//! separate.

use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::geo::haversine_m;
use crate::gtfs::Gtfs;
use crate::names::norm_route;

#[derive(Debug, Clone)]
pub struct Groups {
    /// Canonical pattern of each pattern (itself if not merged).
    pub canon: Vec<u32>,
    /// Position on the canonical pattern of each stop position.
    pub map: Vec<Vec<u16>>,
    pub report: GroupReport,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GroupReport {
    pub names_with_copies: usize,
    pub patterns_merged: usize,
    pub patterns_kept_apart: usize,
    pub median_match_m: f64,
}

impl Groups {
    pub fn identity(g: &Gtfs) -> Self {
        Self {
            canon: (0..g.patterns.len() as u32).collect(),
            map: g
                .patterns
                .iter()
                .map(|p| (0..p.stops.len() as u16).collect())
                .collect(),
            report: GroupReport::default(),
        }
    }

    pub fn build(g: &Gtfs, max_median_m: f64) -> Self {
        let mut s = Self::identity(g);
        let mut by_name: FxHashMap<String, Vec<u32>> = FxHashMap::default();
        for (pid, p) in g.patterns.iter().enumerate() {
            by_name
                .entry(norm_route(g.routes[p.route as usize].name()))
                .or_default()
                .push(pid as u32);
        }
        let mut medians = Vec::new();
        for pats in by_name.values().filter(|v| v.len() > 1) {
            s.report.names_with_copies += 1;
            let c = *pats
                .iter()
                .max_by_key(|&&p| {
                    (
                        g.patterns[p as usize].trips.len(),
                        g.patterns[p as usize].stops.len(),
                    )
                })
                .unwrap();
            for &p in pats.iter().filter(|&&p| p != c) {
                let (m, d) = monotone_nearest(g, p, c);
                let mut ds = d.clone();
                ds.sort_by(f64::total_cmp);
                let med = ds[ds.len() / 2];
                if med <= max_median_m {
                    s.canon[p as usize] = c;
                    s.map[p as usize] = m;
                    s.report.patterns_merged += 1;
                    medians.push(med);
                } else {
                    s.report.patterns_kept_apart += 1;
                }
            }
        }
        medians.sort_by(f64::total_cmp);
        s.report.median_match_m = medians.get(medians.len() / 2).copied().unwrap_or(0.0);
        s
    }

    pub fn is_canonical(&self, p: u32) -> bool {
        self.canon[p as usize] == p
    }

    /// Canonical segment that represents segment `k` of pattern `p` for read-out.
    pub fn canon_segment(&self, p: u32, k: usize, canon_nseg: usize) -> usize {
        let m = &self.map[p as usize];
        (m[k] as usize).min(canon_nseg - 1)
    }
}

/// Map each stop of `from` to the nearest stop of `to`, never going backwards.
fn monotone_nearest(g: &Gtfs, from: u32, to: u32) -> (Vec<u16>, Vec<f64>) {
    let (a, b) = (&g.patterns[from as usize], &g.patterns[to as usize]);
    let mut prev = 0usize;
    let mut map = Vec::with_capacity(a.stops.len());
    let mut dist = Vec::with_capacity(a.stops.len());
    for &s in &a.stops {
        let sa = &g.stops[s as usize];
        let mut best = (f64::INFINITY, prev);
        for j in prev..b.stops.len().min(prev + 25) {
            let sb = &g.stops[b.stops[j] as usize];
            let d = haversine_m(sa.lat, sa.lon, sb.lat, sb.lon);
            if d < best.0 {
                best = (d, j);
            }
        }
        prev = best.1;
        map.push(best.1 as u16);
        dist.push(best.0);
    }
    (map, dist)
}
