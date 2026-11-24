//! Shared state: the loaded snapshot with derived indexes (hot-swappable), the live
//! nowcast, the HTTP client and metrics.

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use arc_swap::ArcSwapOption;
use chrono::{DateTime, FixedOffset, NaiveDate, Timelike, Utc};
use crowd_core::daytype::{DayCalendar, DayType};
use crowd_core::live_trees::LiveTrees;
use crowd_core::names::{norm_route, norm_stop, route_candidates};
use crowd_core::nowcast::{LiveFactor, Nowcast, NowcastConfig, UsualBoardings};
use crowd_core::planner::{active_mask, Network, PlannerConfig};
use crowd_core::snapshot::Snapshot;
use crowd_core::tickets::{ist, parse_ts};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::config::ServiceConfig;
use crate::error::{AppError, AppResult};
use crate::live_rt::LiveFleet;
use crate::metrics::Metrics;

pub struct Loaded {
    pub snap: Snapshot,
    pub cal: DayCalendar,
    pub net: Network,
    pub trip_by_id: FxHashMap<String, u32>,
    pub stop_by_id: FxHashMap<String, u32>,
    pub route_by_id: FxHashMap<String, u32>,
    pub route_by_name: FxHashMap<String, u32>,
    /// Route used for live counts: copies of one route number share the first copy.
    pub route_canon: Vec<u32>,
    /// Usual ticketed boardings per live route key and for the city, for live tickets.
    pub usual: UsualBoardings,
    pub patterns_by_route: Vec<Vec<u32>>,
    /// Normalised stop names for search, with stop index.
    pub stop_search: Vec<(String, u32)>,
    pub stop_has_data: Vec<bool>,
    pub mtime: Option<SystemTime>,
    pub loaded_at: DateTime<Utc>,
    active_cache: Mutex<FxHashMap<NaiveDate, Arc<Vec<bool>>>>,
    pub map_cache: Mutex<FxHashMap<(u8, u16, u8), Arc<serde_json::Value>>>,
}

impl Loaded {
    pub fn new(snap: Snapshot, planner: PlannerConfig, mtime: Option<SystemTime>) -> Self {
        let trip_by_id = snap
            .trips
            .iter()
            .enumerate()
            .map(|(i, t)| (t.trip_id.clone(), i as u32))
            .collect();
        let stop_by_id = snap
            .stops
            .iter()
            .enumerate()
            .map(|(i, s)| (s.stop_id.clone(), i as u32))
            .collect();
        let route_by_id = snap
            .routes
            .iter()
            .enumerate()
            .map(|(i, r)| (r.route_id.clone(), i as u32))
            .collect();
        // For a route number listed once per operator, use the copy with most trips: the
        // model counts both copies' tickets and buses on that one.
        let mut route_by_name: FxHashMap<String, u32> = FxHashMap::default();
        let weight = |ri: usize| {
            snap.patterns
                .iter()
                .filter(|p| p.route as usize == ri)
                .map(|p| (p.trips.len(), p.stops.len()))
                .max()
                .unwrap_or((0, 0))
        };
        let weights: Vec<(usize, usize)> = (0..snap.routes.len()).map(weight).collect();
        for (i, r) in snap.routes.iter().enumerate() {
            let e = route_by_name
                .entry(norm_route(r.name()))
                .or_insert(i as u32);
            if weights[i] > weights[*e as usize] {
                *e = i as u32;
            }
        }
        let route_canon: Vec<u32> = snap
            .routes
            .iter()
            .map(|r| route_by_name[&norm_route(r.name())])
            .collect();
        let usual = UsualBoardings::new(&snap, |r| route_canon[r as usize]);
        let mut patterns_by_route = vec![Vec::new(); snap.routes.len()];
        let mut stop_has_data = vec![false; snap.stops.len()];
        for (pid, p) in snap.patterns.iter().enumerate() {
            patterns_by_route[p.route as usize].push(pid as u32);
            if p.cells.is_some() {
                for &s in &p.stops {
                    stop_has_data[s as usize] = true;
                }
            }
        }
        let stop_search = snap
            .stops
            .iter()
            .enumerate()
            .map(|(i, s)| (norm_stop(&s.name), i as u32))
            .collect();
        let net = Network::build(&snap, planner);
        let cal = snap.day_calendar();
        Self {
            snap,
            cal,
            net,
            trip_by_id,
            stop_by_id,
            route_by_id,
            route_by_name,
            route_canon,
            usual,
            patterns_by_route,
            stop_search,
            stop_has_data,
            mtime,
            loaded_at: Utc::now(),
            active_cache: Mutex::new(FxHashMap::default()),
            map_cache: Mutex::new(FxHashMap::default()),
        }
    }

    pub fn active(&self, date: NaiveDate) -> Arc<Vec<bool>> {
        let mut c = self.active_cache.lock();
        if let Some(a) = c.get(&date) {
            return a.clone();
        }
        if c.len() > 16 {
            c.clear();
        }
        let a = Arc::new(active_mask(&self.snap, date));
        c.insert(date, a.clone());
        a
    }

    /// Route index for a ticket's route name.
    pub fn route_for_ticket(&self, name: &str) -> Option<u32> {
        route_candidates(name)
            .iter()
            .find_map(|c| self.route_by_name.get(c).copied())
    }

    /// Strip a feed prefix such as `1:` from an OTP id.
    pub fn local_id(id: &str) -> &str {
        id.split_once(':').map(|(_, b)| b).unwrap_or(id)
    }
}

pub struct AppState {
    pub cfg: ServiceConfig,
    pub loaded: ArcSwapOption<Loaded>,
    pub live: Mutex<Nowcast>,
    /// Latest live bus positions, placed on routes.
    pub fleet: ArcSwapOption<LiveFleet>,
    pub http: reqwest::Client,
    pub metrics: Metrics,
}

/// A point in time resolved for a request.
#[derive(Debug, Clone, Copy)]
pub struct When {
    pub date: NaiveDate,
    pub secs: u32,
    pub ts: i64,
    pub dt: DayType,
}

impl AppState {
    pub fn new(cfg: ServiceConfig) -> Self {
        let trees = match cfg.live.model.as_str() {
            "trees" => match LiveTrees::load(std::path::Path::new(&cfg.live.trees_path)) {
                Ok(t) => {
                    tracing::info!(trees = t.trees.len(), "live model: gradient-boosted trees");
                    Some(Arc::new(t))
                }
                Err(e) => {
                    tracing::error!(error = %e, "live trees not loaded; using the rule");
                    None
                }
            },
            _ => None,
        };
        let live = Nowcast::new(NowcastConfig {
            window_secs: cfg.live.window_secs,
            prior_tickets: cfg.live.prior_tickets,
            min_factor: cfg.live.min_factor,
            max_factor: cfg.live.max_factor,
            city_anchor: cfg.live.city_anchor,
            decay_per_hour: cfg.live.decay_per_hour,
            ..NowcastConfig::default()
        })
        .with_trees(trees);
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(cfg.otp.timeout_ms))
            .build()
            .expect("http client");
        Self {
            cfg,
            loaded: ArcSwapOption::empty(),
            live: Mutex::new(live),
            fleet: ArcSwapOption::empty(),
            http,
            metrics: Metrics::new(),
        }
    }

    pub fn planner_config(&self) -> PlannerConfig {
        PlannerConfig {
            max_transfers: self.cfg.planner.max_transfers as usize,
            max_access_m: self.cfg.planner.max_access_m,
            max_transfer_m: self.cfg.planner.max_transfer_m,
            ..Default::default()
        }
    }

    pub fn load_snapshot(&self, path: &Path) -> anyhow::Result<()> {
        let t0 = std::time::Instant::now();
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let snap = Snapshot::load(path)?;
        let loaded = Loaded::new(snap, self.planner_config(), mtime);
        tracing::info!(
            path = %path.display(),
            patterns = loaded.snap.patterns.len(),
            trips = loaded.snap.trips.len(),
            elapsed_ms = t0.elapsed().as_millis() as u64,
            "snapshot loaded"
        );
        self.metrics
            .snapshot_loaded
            .set(Utc::now().timestamp() as f64);
        self.loaded.store(Some(Arc::new(loaded)));
        Ok(())
    }

    pub fn install(&self, snap: Snapshot) {
        self.loaded.store(Some(Arc::new(Loaded::new(
            snap,
            self.planner_config(),
            None,
        ))));
    }

    pub fn snap(&self) -> AppResult<Arc<Loaded>> {
        self.loaded.load_full().ok_or(AppError::NotReady)
    }

    /// Current time in local (IST) terms; follows ingested tickets in replay mode.
    pub fn now(&self) -> DateTime<FixedOffset> {
        if self.cfg.live.clock == "replay" {
            let ts = self.live.lock().latest_ts;
            if ts > 0 {
                if let Some(t) = DateTime::from_timestamp(ts, 0) {
                    return t.with_timezone(&ist());
                }
            }
        }
        Utc::now().with_timezone(&ist())
    }

    /// Resolve `at` (ISO date-time) or `date` + `time` (HH:MM), defaulting to now.
    pub fn resolve(
        &self,
        l: &Loaded,
        at: Option<&str>,
        date: Option<&str>,
        time: Option<&str>,
    ) -> AppResult<When> {
        let t = match (at, date, time) {
            (Some(a), _, _) => {
                parse_ts(a, ist()).map_err(|e| AppError::BadRequest(e.to_string()))?
            }
            (None, d, t) if d.is_some() || t.is_some() => {
                let now = self.now();
                let d = d
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| now.date_naive().to_string());
                let t = t
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| now.format("%H:%M").to_string());
                let t = if t.len() == 5 { format!("{t}:00") } else { t };
                parse_ts(&format!("{d} {t}"), ist())
                    .map_err(|e| AppError::BadRequest(e.to_string()))?
            }
            _ => self.now(),
        };
        let date = t.date_naive();
        Ok(When {
            date,
            secs: t.time().num_seconds_from_midnight(),
            ts: t.timestamp(),
            dt: l.cal.day_type(date),
        })
    }

    /// Live factor for a route if `when` is close to now and live data is on.
    pub fn live_factor(&self, l: &Loaded, route: u32, when: &When) -> LiveFactor {
        if !self.cfg.live.enabled {
            return LiveFactor::neutral();
        }
        let now = self.now();
        let delta = when.ts - now.timestamp();
        if !(-3600..=3 * 3600).contains(&delta) {
            return LiveFactor::neutral();
        }
        let secs = now.time().num_seconds_from_midnight();
        // Copies of one route number (two operators) share live counts and expectations.
        let route = l.route_canon[route as usize];
        // Live tickets count only while a ticket stream is flowing: without one, no tickets
        // would read as an empty network.
        let mut f = {
            let live = self.live.lock();
            let flowing = live.observed_total > 0
                && now.timestamp() - live.latest_ts <= self.cfg.live.window_secs;
            if flowing {
                let ahead = (delta.max(0) / 3600) as u32;
                live.factor(&l.usual, route, when.dt, now.timestamp(), secs, ahead)
            } else {
                LiveFactor::neutral()
            }
        };
        // Buses running now against the usual number: fewer buses, more riders on each.
        let c = &self.cfg.rt;
        if let Some(fleet) = self.fleet.load_full() {
            let fresh = (now.timestamp() - fleet.feed_ts).abs() <= c.max_age_secs;
            let near = (-1800..=3600).contains(&delta);
            let usual = l.snap.usual_buses(route, when.dt, secs);
            if c.enabled && fresh && near {
                if let Some(u) = usual {
                    let running = fleet.running_by_route.get(&route).copied().unwrap_or(0);
                    if u > 0.0 || running > 0 {
                        let k = c.pseudo_buses;
                        let sf = ((u + k) / (running as f64 + k)).clamp(c.min_factor, c.max_factor);
                        f.factor *= sf;
                        f.supply_factor = sf;
                        f.buses_live = Some(running);
                        f.buses_usual = Some(u);
                    }
                }
            }
        }
        f
    }
}

/// Watch the snapshot file and reload when it changes.
pub async fn reload_loop(state: Arc<AppState>) {
    let secs = state.cfg.reload_secs;
    if secs == 0 {
        return;
    }
    let path = std::path::PathBuf::from(&state.cfg.snapshot_path);
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(secs));
    tick.tick().await;
    loop {
        tick.tick().await;
        let m = tokio::fs::metadata(&path)
            .await
            .and_then(|m| m.modified())
            .ok();
        let Some(cur) = state.loaded.load_full() else {
            continue;
        };
        if m.is_some() && m != cur.mtime {
            let st = state.clone();
            let p = path.clone();
            let res = tokio::task::spawn_blocking(move || st.load_snapshot(&p)).await;
            match res {
                Ok(Ok(())) => tracing::info!("snapshot reloaded"),
                Ok(Err(e)) => tracing::warn!(error = %e, "snapshot reload failed"),
                Err(e) => tracing::warn!(error = %e, "snapshot reload panicked"),
            }
        }
    }
}
