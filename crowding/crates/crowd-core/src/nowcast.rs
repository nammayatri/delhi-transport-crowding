//! Live adjustment of the historical profile.
//!
//! With tickets covering a small share of riders, a single bus's live ticket count says
//! little, and even a route's count over an hour swings with which buses happened to pass:
//! on busy routes a route's ratio to usual in one hour correlates only about 0.24 with the
//! next hour's. The whole city's ticket rate against usual is far steadier and does carry
//! over (festival days, a quiet Monday). So the factor is the city's ratio, nudged by the
//! route's own tickets with a large pseudo-count, clamped, and faded for hours further
//! ahead. Settings come from replaying October (docs/results/nowcast_backtest.md).

use std::collections::{BTreeMap, VecDeque};

use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::daytype::{DayType, N_DAYTYPES};
use crate::snapshot::Snapshot;

#[derive(Debug, Clone)]
pub struct NowcastConfig {
    pub window_secs: i64,
    /// Pseudo-count of tickets pulling a route's factor toward the anchor.
    pub prior_tickets: f64,
    pub min_factor: f64,
    pub max_factor: f64,
    /// Anchor at the whole city's live ratio (true) or at 1 (false).
    pub city_anchor: bool,
    /// The factor's distance from 1 shrinks by this per whole hour ahead (1 = no fading).
    pub decay_per_hour: f64,
}

impl Default for NowcastConfig {
    fn default() -> Self {
        Self {
            window_secs: 3600,
            prior_tickets: 200.0,
            min_factor: 0.67,
            max_factor: 1.5,
            city_anchor: true,
            decay_per_hour: 0.85,
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
    /// The whole city's live ticketed riders against usual over the same window (clamped).
    pub city_factor: f64,
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
            city_factor: 1.0,
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
    /// Whole city: pax per minute (unix minute).
    city: BTreeMap<i64, f64>,
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
        *self.city.entry(ts.div_euclid(60)).or_default() += pax as f64;
        self.latest_ts = self.latest_ts.max(ts);
        self.observed_total += 1;
        let cutoff = self.latest_ts - 2 * self.cfg.window_secs;
        while q.front().is_some_and(|f| f.0 < cutoff) {
            q.pop_front();
        }
        while self
            .city
            .first_key_value()
            .is_some_and(|(m, _)| *m < cutoff.div_euclid(60))
        {
            self.city.pop_first();
        }
    }

    /// Live factor for a route at local time `secs` on a day of type `dt`, where `now` is
    /// the unix time matching `secs`, for a trip `hours_ahead` whole hours from now.
    pub fn factor(
        &self,
        usual: &UsualBoardings,
        route: u32,
        dt: DayType,
        now: i64,
        secs: u32,
        hours_ahead: u32,
    ) -> LiveFactor {
        let w = self.cfg.window_secs;
        let from = secs.saturating_sub(w as u32);
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
        let expected = usual.window(Some(route), dt, from, secs);
        let (lo, hi) = (self.cfg.min_factor, self.cfg.max_factor);
        let city_factor = if self.cfg.city_anchor {
            let obs: f64 = self
                .city
                .range((now - w).div_euclid(60) + 1..=now.div_euclid(60))
                .map(|(_, v)| *v)
                .sum();
            let exp = usual.window(None, dt, from, secs);
            if exp > 0.0 {
                (obs / exp).clamp(lo, hi)
            } else {
                1.0
            }
        } else {
            1.0
        };
        if expected <= 0.0 && observed <= 0.0 && !self.cfg.city_anchor {
            return LiveFactor::neutral();
        }
        let k = self.cfg.prior_tickets;
        let f = if expected + k > 0.0 {
            (observed + k * city_factor) / (expected + k)
        } else {
            city_factor
        };
        let f = f.clamp(lo, hi);
        let fade = self.cfg.decay_per_hour.powi(hours_ahead.min(24) as i32);
        LiveFactor {
            factor: f.powf(fade),
            observed,
            expected,
            city_factor,
            ..LiveFactor::neutral()
        }
    }

    pub fn routes_seen(&self) -> usize {
        self.recent.len()
    }
}

/// Usual ticketed boardings per day, per live route key and for the whole city, by day type
/// and time bin. Counted on canonical patterns only: the other patterns of a route number
/// (short workings, a second operator's copy) carry copies of the same riders.
#[derive(Debug, Clone, Default)]
pub struct UsualBoardings {
    starts: Vec<u32>,
    bin_secs: u32,
    route: FxHashMap<u32, Vec<f64>>,
    city: Vec<f64>,
}

impl UsualBoardings {
    /// `key` maps a route index to the key live tickets are counted under.
    pub fn new(snap: &Snapshot, key: impl Fn(u32) -> u32) -> Self {
        let grid = snap.grid();
        let nb = grid.nb;
        let starts = (0..nb)
            .map(|b| snap.config.bin_start_minute(b) * 60)
            .collect();
        let mut route: FxHashMap<u32, Vec<f64>> = FxHashMap::default();
        let mut city = vec![0.0; N_DAYTYPES * nb];
        for (pid, p) in snap.patterns.iter().enumerate() {
            if p.canon as usize != pid {
                continue;
            }
            let Some(c) = &p.cells else { continue };
            let r = route
                .entry(key(p.route))
                .or_insert_with(|| vec![0.0; N_DAYTYPES * nb]);
            for pos in 0..p.stops.len() {
                for dt in 0..N_DAYTYPES {
                    for b in 0..nb {
                        let v = c.board[grid.idx(pos, dt, b)] as f64;
                        r[dt * nb + b] += v;
                        city[dt * nb + b] += v;
                    }
                }
            }
        }
        Self {
            starts,
            bin_secs: snap.config.bin_minutes * 60,
            route,
            city,
        }
    }

    /// Built directly from per-bin values (tests and tools).
    pub fn from_parts(
        starts: Vec<u32>,
        bin_secs: u32,
        route: FxHashMap<u32, Vec<f64>>,
        city: Vec<f64>,
    ) -> Self {
        Self {
            starts,
            bin_secs,
            route,
            city,
        }
    }

    /// Usual boardings between two times of day on a route (`None`: the whole city).
    pub fn window(&self, route: Option<u32>, dt: DayType, from: u32, to: u32) -> f64 {
        let v = match route {
            Some(r) => match self.route.get(&r) {
                Some(v) => v,
                None => return 0.0,
            },
            None => &self.city,
        };
        let nb = self.starts.len();
        let bm = self.bin_secs;
        let mut total = 0.0;
        for (b, &b0) in self.starts.iter().enumerate() {
            let overlap = (b0 + bm).min(to).saturating_sub(b0.max(from));
            if overlap > 0 {
                total += v[dt.idx() * nb + b] * overlap as f64 / bm as f64;
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usual() -> UsualBoardings {
        // One 60-minute bin per hour; route 7 usually has 10 riders an hour, the city 1000.
        let nb = 24;
        let mut r = vec![0.0; N_DAYTYPES * nb];
        let mut c = vec![0.0; N_DAYTYPES * nb];
        for b in 0..nb {
            r[b] = 10.0;
            c[b] = 1000.0;
        }
        let mut route = FxHashMap::default();
        route.insert(7, r);
        UsualBoardings::from_parts((0..nb as u32).map(|h| h * 3600).collect(), 3600, route, c)
    }

    fn feed(n: &mut Nowcast, route: u32, now: i64, pax: usize) {
        for i in 0..pax {
            n.observe(route, now - 3000 + i as i64, 1.0);
        }
    }

    #[test]
    fn a_quiet_city_pulls_every_route_down() {
        let mut n = Nowcast::new(NowcastConfig::default());
        let now = 10 * 3600;
        feed(&mut n, 7, now, 10); // the route itself is at usual
        feed(&mut n, 8, now, 790); // the rest of the city: 800 of 1000
        let f = n.factor(&usual(), 7, DayType::Weekday, now, now as u32, 0);
        assert!((f.city_factor - 0.8).abs() < 1e-9);
        // (10 + 200 * 0.8) / (10 + 200)
        assert!((f.factor - 170.0 / 210.0).abs() < 1e-9, "{}", f.factor);
        let later = n.factor(&usual(), 7, DayType::Weekday, now, now as u32, 2);
        assert!((later.factor - f.factor.powf(0.85 * 0.85)).abs() < 1e-9);
        assert!(later.factor > f.factor && later.factor < 1.0);
    }

    #[test]
    fn one_busy_hour_on_a_route_moves_it_little() {
        let mut n = Nowcast::new(NowcastConfig::default());
        let now = 10 * 3600;
        feed(&mut n, 7, now, 30); // three times usual
        feed(&mut n, 8, now, 970);
        let f = n.factor(&usual(), 7, DayType::Weekday, now, now as u32, 0);
        assert!(f.factor > 1.0 && f.factor < 1.15, "{}", f.factor);
    }

    #[test]
    fn route_only_rule_without_city_anchor() {
        let cfg = NowcastConfig {
            prior_tickets: 6.0,
            min_factor: 0.5,
            max_factor: 2.5,
            city_anchor: false,
            decay_per_hour: 1.0,
            ..NowcastConfig::default()
        };
        let mut n = Nowcast::new(cfg);
        let now = 10 * 3600;
        feed(&mut n, 7, now, 20);
        let f = n.factor(&usual(), 7, DayType::Weekday, now, now as u32, 1);
        assert!((f.factor - 26.0 / 16.0).abs() < 1e-9);
        assert_eq!(f.city_factor, 1.0);
    }
}
