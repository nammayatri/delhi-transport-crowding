//! Live adjustment of the historical profile.
//!
//! With tickets covering a small share of riders, a single bus's live ticket count says
//! little. A route's live ticket rate over the last hour, compared with the rate expected
//! for that route, day type and time, says more: it tells us whether today is busier than
//! usual. The factor is shrunk toward 1 with a pseudo-count, clamped, and applied to the
//! historical rate for the hours ahead.

use std::collections::VecDeque;

use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::daytype::DayType;
use crate::snapshot::Snapshot;

#[derive(Debug, Clone)]
pub struct NowcastConfig {
    pub window_secs: i64,
    /// Pseudo-count of tickets pulling the factor toward 1.
    pub prior_tickets: f64,
    pub min_factor: f64,
    pub max_factor: f64,
}

impl Default for NowcastConfig {
    fn default() -> Self {
        Self {
            window_secs: 3600,
            prior_tickets: 6.0,
            min_factor: 0.5,
            max_factor: 2.5,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct LiveFactor {
    /// Applied to the historical riders per bus: busier than usual and fewer buses than
    /// usual both raise it.
    pub factor: f64,
    /// Live and expected ticketed riders on the route in the last window.
    pub observed: f64,
    pub expected: f64,
    /// Buses running the route now (live feed) and usually at this time.
    pub buses_live: Option<u32>,
    pub buses_usual: Option<f64>,
    /// Part of `factor` from the number of buses (usual / running, shrunk and clamped).
    pub supply_factor: f64,
}

impl LiveFactor {
    pub fn neutral() -> Self {
        Self {
            factor: 1.0,
            observed: 0.0,
            expected: 0.0,
            buses_live: None,
            buses_usual: None,
            supply_factor: 1.0,
        }
    }
}

#[derive(Debug, Default)]
pub struct Nowcast {
    pub cfg: NowcastConfig,
    /// Per route: (unix ts, pax), oldest first.
    recent: FxHashMap<u32, VecDeque<(i64, f32)>>,
    pub latest_ts: i64,
    pub observed_total: u64,
}

impl Nowcast {
    pub fn new(cfg: NowcastConfig) -> Self {
        Self {
            cfg,
            ..Default::default()
        }
    }

    pub fn observe(&mut self, route: u32, ts: i64, pax: f32) {
        let q = self.recent.entry(route).or_default();
        q.push_back((ts, pax));
        self.latest_ts = self.latest_ts.max(ts);
        self.observed_total += 1;
        let cutoff = self.latest_ts - 2 * self.cfg.window_secs;
        while q.front().is_some_and(|f| f.0 < cutoff) {
            q.pop_front();
        }
    }

    /// Live factor for a route at local time `secs` on a day of type `dt`, where `now` is
    /// the unix time matching `secs`.
    pub fn factor(
        &self,
        snap: &Snapshot,
        route: u32,
        dt: DayType,
        now: i64,
        secs: u32,
    ) -> LiveFactor {
        let w = self.cfg.window_secs;
        let observed: f64 = self
            .recent
            .get(&route)
            .map(|q| {
                q.iter()
                    .filter(|(t, _)| *t > now - w && *t <= now)
                    .map(|x| x.1 as f64)
                    .sum()
            })
            .unwrap_or(0.0);
        let expected = expected_boardings(snap, route, dt, secs.saturating_sub(w as u32), secs);
        if expected <= 0.0 && observed <= 0.0 {
            return LiveFactor::neutral();
        }
        let k = self.cfg.prior_tickets;
        let f = ((observed + k) / (expected + k)).clamp(self.cfg.min_factor, self.cfg.max_factor);
        LiveFactor {
            factor: f,
            observed,
            expected,
            ..LiveFactor::neutral()
        }
    }

    pub fn routes_seen(&self) -> usize {
        self.recent.len()
    }
}

/// Historical ticketed boardings per day on all patterns of a route between two times.
pub fn expected_boardings(snap: &Snapshot, route: u32, dt: DayType, from: u32, to: u32) -> f64 {
    let grid = snap.grid();
    let bm = snap.config.bin_minutes * 60;
    let mut total = 0.0;
    for p in snap.patterns.iter().filter(|p| p.route == route) {
        let Some(c) = &p.cells else { continue };
        for pos in 0..p.stops.len() {
            for bin in 0..grid.nb {
                let b0 = snap.config.bin_start_minute(bin) * 60;
                let b1 = b0 + bm;
                let overlap = b1.min(to).saturating_sub(b0.max(from));
                if overlap > 0 {
                    total +=
                        c.board[grid.idx(pos, dt.idx(), bin)] as f64 * overlap as f64 / bm as f64;
                }
            }
        }
    }
    total
}
