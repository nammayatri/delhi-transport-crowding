//! Route groups: patterns that riders and tickets do not tell apart.
//!
//! Two cases share one route name. A route number may be listed twice, once per operator,
//! each copy with its own stop ids. And one route may have several stop patterns, such as
//! full trips and short-turn trips that cover part of it. Tickets name only the route, and
//! a stretch served by both full and short trips carries the riders and the buses of both.
//! So we merge the patterns of a name onto one canonical pattern: the one with most stops
//! (then most trips), which covers the others. Every other pattern's stops are mapped to the
//! nearest canonical stop along the route, and its tickets and buses are counted on the
//! canonical pattern. Patterns whose stops do not line up (median distance above a
//! threshold) stay separate.

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
                        g.patterns[p as usize].stops.len(),
                        g.patterns[p as usize].trips.len(),
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

/// Map each stop of `from` to the nearest stop of `to`, never going backwards. The first
/// stop may match anywhere (a short-turn trip can start mid-route); later stops search a
/// window ahead of the previous match.
fn monotone_nearest(g: &Gtfs, from: u32, to: u32) -> (Vec<u16>, Vec<f64>) {
    let (a, b) = (&g.patterns[from as usize], &g.patterns[to as usize]);
    let mut prev = 0usize;
    let mut map = Vec::with_capacity(a.stops.len());
    let mut dist = Vec::with_capacity(a.stops.len());
    for (i, &s) in a.stops.iter().enumerate() {
        let sa = &g.stops[s as usize];
        let mut best = (f64::INFINITY, prev);
        let end = if i == 0 {
            b.stops.len()
        } else {
            b.stops.len().min(prev + 25)
        };
        for j in prev..end {
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
