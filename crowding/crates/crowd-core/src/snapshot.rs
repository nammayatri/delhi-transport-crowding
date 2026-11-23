//! The model output that services load: GTFS geometry and schedule, inferred bus classes
//! and per-pattern sufficient statistics. Crowding views are derived on read, so the same
//! snapshot can be read under the low, mid or high expansion scenario.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::config::{CapacityClass, ModelConfig};
use crate::daytype::{DayCalendar, DayType};
use crate::gtfs::{Calendar, Route, Stop};
use crate::levels::{
    evaluate, expected_multiplier, CellView, Confidence, Expansion, Level, ScenarioPick,
};
use crate::profile::{Cells, Grid};

pub const FORMAT_VERSION: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub format_version: u32,
    pub generated_at: String,
    pub date_from: NaiveDate,
    pub date_to: NaiveDate,
    pub n_days: [u32; 3],
    pub expansion: Expansion,
    /// Ticketed riders on an average weekday (all tickets, matched or not).
    pub ticketed_pax_per_weekday: f64,
    pub feed_version: String,
    pub tickets_total: u64,
    pub tickets_assigned: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapPattern {
    pub route: u32,
    pub stops: Vec<u32>,
    pub dist_m: Vec<f32>,
    pub trips: Vec<u32>,
    pub class: String,
    pub ac_share: Option<f32>,
    pub tickets: f32,
    /// Route-specific multiplier on the city-wide expansion (capacity consistency), <= 1.
    pub expansion_adj: f32,
    /// Pattern that carries this one's estimates (itself, or the full route a short-turn
    /// pattern is merged onto).
    pub canon: u32,
    pub cells: Option<Cells>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapTrip {
    pub trip_id: String,
    pub pattern: u32,
    pub service_id: String,
    pub times: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub meta: Meta,
    pub config: ModelConfig,
    pub stops: Vec<Stop>,
    pub routes: Vec<Route>,
    pub patterns: Vec<SnapPattern>,
    pub trips: Vec<SnapTrip>,
    pub calendar: Calendar,
    /// Buses usually running each route, `[route][day type * bins + bin]`.
    #[serde(default)]
    pub usual_buses: Vec<Vec<f32>>,
    /// Live feed calibration behind the bus supply, if any.
    #[serde(default)]
    pub rt: Option<crate::rt::RtCalibration>,
}

/// Crowding over one ride: from a boarding position to an alighting position on a trip.
#[derive(Debug, Clone, Serialize)]
pub struct LegCrowding {
    pub level_at_boarding: Option<Level>,
    pub worst_level: Option<Level>,
    /// Where the worst level holds longest: from and to stop names.
    pub worst_from: Option<String>,
    pub worst_to: Option<String>,
    pub confidence: Confidence,
    pub ride_secs: u32,
    /// Ride time weighted by crowding multipliers minus the plain ride time.
    pub crowding_extra_secs: f64,
    pub segments: Vec<SegmentCrowding>,
    pub has_data: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentCrowding {
    pub from_stop: String,
    pub to_stop: String,
    pub depart_secs: u32,
    pub ride_secs: u32,
    #[serde(flatten)]
    pub view: Option<CellView>,
}

impl Snapshot {
    pub fn save(&self, path: &Path) -> Result<()> {
        let f =
            std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
        let mut z = zstd::Encoder::new(BufWriter::new(f), 9)?;
        let bytes = rmp_serde::to_vec_named(self)?;
        z.write_all(&bytes)?;
        z.finish()?.flush()?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Snapshot> {
        let f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut buf = Vec::new();
        zstd::Decoder::new(BufReader::new(f))?.read_to_end(&mut buf)?;
        let s: Snapshot = rmp_serde::from_slice(&buf).context("decode snapshot")?;
        anyhow::ensure!(
            s.meta.format_version == FORMAT_VERSION,
            "snapshot format mismatch"
        );
        Ok(s)
    }

    pub fn grid(&self) -> Grid {
        Grid {
            nb: self.config.n_bins(),
        }
    }

    pub fn day_calendar(&self) -> DayCalendar {
        DayCalendar::new(&self.config.holidays).unwrap_or_default()
    }

    pub fn class_of(&self, pattern: u32) -> &CapacityClass {
        let name = &self.patterns[pattern as usize].class;
        self.config
            .class(name)
            .unwrap_or(&self.config.capacity_classes[0])
    }

    /// Buses usually running a route at a time of day.
    pub fn usual_buses(&self, route: u32, dt: DayType, secs: u32) -> Option<f64> {
        let v = self.usual_buses.get(route as usize)?;
        let bin = self.config.bin_of(secs % 86_400)?;
        v.get(dt.idx() * self.grid().nb + bin).map(|&x| x as f64)
    }

    pub fn route_of(&self, pattern: u32) -> &Route {
        &self.routes[self.patterns[pattern as usize].route as usize]
    }

    /// Crowding on a segment in a bin, optionally scaled by a live factor.
    pub fn cell(
        &self,
        pattern: u32,
        seg: usize,
        dt: DayType,
        bin: usize,
        scen: ScenarioPick,
        live_factor: f64,
    ) -> Option<CellView> {
        let p = &self.patterns[pattern as usize];
        let c = p.cells.as_ref()?;
        let i = self.grid().idx(seg, dt.idx(), bin);
        let rate = *c.rate.get(i)?;
        if rate.is_nan() {
            return None;
        }
        let mut v = evaluate(
            &self.config,
            self.class_of(pattern),
            &self.meta.expansion,
            scen,
            rate as f64 * p.expansion_adj as f64 * live_factor,
            c.tickets[i] as f64,
            c.buses[i] as f64,
        );
        // Confidence also reflects how well we know the number of buses.
        let f = c.flags.get(i).copied().unwrap_or(0);
        if f != 0 {
            let cap = if f.count_ones() >= 2 {
                Confidence::Low
            } else {
                Confidence::Medium
            };
            v.confidence = v.confidence.min(cap);
        }
        Some(v)
    }

    /// Like [`Snapshot::cell`], looking the bin up from a time of day. Times past midnight
    /// of the service day wrap around.
    pub fn cell_at(
        &self,
        pattern: u32,
        seg: usize,
        dt: DayType,
        secs: u32,
        scen: ScenarioPick,
        live_factor: f64,
    ) -> Option<CellView> {
        let bin = self.config.bin_of(secs % 86_400)?;
        self.cell(pattern, seg, dt, bin, scen, live_factor)
    }

    /// Crowding for riding `trip` from position `board` to `alight`.
    pub fn leg(
        &self,
        trip: u32,
        board: usize,
        alight: usize,
        dt: DayType,
        scen: ScenarioPick,
        live_factor: f64,
    ) -> LegCrowding {
        let t = &self.trips[trip as usize];
        let p = &self.patterns[t.pattern as usize];
        let name = |pos: usize| self.stops[p.stops[pos] as usize].name.clone();
        let mut segs = Vec::new();
        let mut extra = 0.0;
        let mut worst: Option<(Level, usize)> = None;
        let mut min_conf = Confidence::High;
        let mut any = false;
        for k in board..alight.min(p.stops.len() - 1) {
            let ride = t.times[k + 1].saturating_sub(t.times[k]);
            let v = self.cell_at(t.pattern, k, dt, t.times[k], scen, live_factor);
            if let Some(v) = &v {
                any = true;
                extra += ride as f64 * (expected_multiplier(&self.config, v) - 1.0);
                min_conf = min_conf.min(v.confidence);
                if worst.is_none_or(|(l, _)| v.level > l) {
                    worst = Some((v.level, k));
                }
            }
            segs.push(SegmentCrowding {
                from_stop: name(k),
                to_stop: name(k + 1),
                depart_secs: t.times[k],
                ride_secs: ride,
                view: v,
            });
        }
        // Longest run of segments at the worst level, for "crowded Ashram -> Moolchand".
        let (worst_from, worst_to) = match worst {
            Some((lvl, _)) => {
                let mut best: Option<(usize, usize, u32)> = None;
                let mut run: Option<(usize, u32)> = None;
                for (i, s) in segs.iter().enumerate() {
                    if s.view.is_some_and(|v| v.level == lvl) {
                        let r = run.get_or_insert((i, 0));
                        r.1 += s.ride_secs.max(1);
                        if best.is_none_or(|b| r.1 > b.2) {
                            best = Some((r.0, i, r.1));
                        }
                    } else {
                        run = None;
                    }
                }
                best.map(|(a, b, _)| {
                    (
                        Some(segs[a].from_stop.clone()),
                        Some(segs[b].to_stop.clone()),
                    )
                })
                .unwrap_or((None, None))
            }
            None => (None, None),
        };
        LegCrowding {
            level_at_boarding: segs.first().and_then(|s| s.view.map(|v| v.level)),
            worst_level: worst.map(|w| w.0),
            worst_from,
            worst_to,
            confidence: if any { min_conf } else { Confidence::Low },
            ride_secs: t.times[alight.min(t.times.len() - 1)].saturating_sub(t.times[board]),
            crowding_extra_secs: extra,
            segments: segs,
            has_data: any,
        }
    }

    /// Cumulative crowding-extra seconds at each stop of a trip: C[0] = 0 and
    /// C[k+1] = C[k] + ride(k) * (multiplier(k) - 1). Riding from i to j costs C[j] - C[i].
    /// Also returns the commuter level per segment (255 = no data).
    pub fn trip_cumulative(
        &self,
        trip: u32,
        dt: DayType,
        scen: ScenarioPick,
        live_factor: f64,
    ) -> (Vec<u32>, Vec<u8>) {
        let t = &self.trips[trip as usize];
        let n = t.times.len();
        if n < 2 || t.pattern == u32::MAX {
            return (vec![0; n], vec![255; n.saturating_sub(1)]);
        }
        let mut cum = Vec::with_capacity(n);
        let mut levels = Vec::with_capacity(n.saturating_sub(1));
        let mut acc = 0.0f64;
        cum.push(0);
        for k in 0..n - 1 {
            let ride = t.times[k + 1].saturating_sub(t.times[k]) as f64;
            match self.cell_at(t.pattern, k, dt, t.times[k], scen, live_factor) {
                Some(v) => {
                    acc += ride * (expected_multiplier(&self.config, &v) - 1.0).max(0.0);
                    levels.push(v.level as u8);
                }
                None => levels.push(255),
            }
            cum.push(acc.round() as u32);
        }
        (cum, levels)
    }

    /// Capacity-consistency adjustment for one pattern: the factor (<= 1) that brings the
    /// `quantile` weekday load (bus-weighted, medium or high confidence) down to crush.
    pub fn capacity_adjustment(&self, pattern: u32, quantile: f64) -> f32 {
        if quantile <= 0.0 {
            return 1.0;
        }
        let p = &self.patterns[pattern as usize];
        let Some(c) = &p.cells else { return 1.0 };
        let grid = self.grid();
        let class = self.class_of(pattern);
        let e = self.meta.expansion.mid;
        let mut v: Vec<(f64, f64)> = Vec::new();
        for seg in 0..p.stops.len() - 1 {
            for bin in 0..grid.nb {
                let i = grid.idx(seg, DayType::Weekday.idx(), bin);
                if c.rate[i].is_nan()
                    || (c.tickets[i] as f64) < self.config.confidence.medium_min_tickets
                {
                    continue;
                }
                v.push((c.rate[i] as f64 * e, c.buses[i] as f64));
            }
        }
        if v.is_empty() {
            return 1.0;
        }
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        let total: f64 = v.iter().map(|x| x.1).sum();
        let mut acc = 0.0;
        let mut q = v[v.len() - 1].0;
        for (load, w) in &v {
            acc += w;
            if acc >= quantile * total {
                q = *load;
                break;
            }
        }
        if q > class.crush {
            (class.crush / q) as f32
        } else {
            1.0
        }
    }

    /// Indices of trips active on a date.
    pub fn trips_on(&self, date: NaiveDate) -> Vec<u32> {
        let mut cache = rustc_hash::FxHashMap::default();
        (0..self.trips.len() as u32)
            .filter(|&i| {
                if self.trips[i as usize].pattern == u32::MAX {
                    return false;
                }
                let sid = &self.trips[i as usize].service_id;
                *cache
                    .entry(sid.as_str())
                    .or_insert_with(|| self.calendar.active(sid, date))
            })
            .collect()
    }
}
