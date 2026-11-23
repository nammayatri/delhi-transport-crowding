//! Onboard load profiles.
//!
//! Tickets cover a small share of riders, so a single bus trip usually carries one or two
//! tickets. Instead of reading loads off individual trips, we keep the running onboard
//! count for every ticket (it is on board on each segment between its boarding and
//! alighting stop) and pool it over all scheduled trips that pass the segment in the same
//! day type and time bin:
//!
//! ```text
//! rate(segment, daytype, bin) = ticketed riders crossing the segment in the bin
//!                               / scheduled bus passes of the segment in the bin
//! ```
//!
//! summed over all days of the day type. That is the average per-trip running count. The
//! expected load per bus is `rate * expansion`. Bins are smoothed with a small kernel and
//! sparse bins are shrunk toward a wider time window of the same segment (then all day
//! types pooled), so a bin with few tickets borrows strength from its neighbours.

use chrono::NaiveDate;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::assign::Assigned;
use crate::config::ModelConfig;
use crate::daytype::{DayCalendar, DayType, N_DAYTYPES};
use crate::groups::Groups;
use crate::gtfs::Gtfs;
use crate::tickets::Ticket;

/// Index helpers for dense `[pos][daytype][bin]` arrays.
#[derive(Debug, Clone, Copy)]
pub struct Grid {
    pub nb: usize,
}

impl Grid {
    #[inline]
    pub fn idx(&self, pos: usize, dt: usize, bin: usize) -> usize {
        (pos * N_DAYTYPES + dt) * self.nb + bin
    }
    pub fn len(&self, n_pos: usize) -> usize {
        n_pos * N_DAYTYPES * self.nb
    }
}

/// Raw sums for one pattern.
#[derive(Debug, Clone, Default)]
pub struct PatternAccum {
    /// Ticketed riders crossing each segment, `[segment][dt][bin]`.
    pub cross: Vec<f32>,
    /// Ticketed boardings and alightings at each stop, `[stop pos][dt][bin]`.
    pub board: Vec<f32>,
    pub alight: Vec<f32>,
    pub tickets: f32,
    pub ac_tickets: f32,
    pub ac_known: f32,
}

pub struct Accumulator {
    pub grid: Grid,
    pub pats: Vec<Option<PatternAccum>>,
    pub dropped_out_of_hours: f64,
}

impl Accumulator {
    pub fn new(g: &Gtfs, cfg: &ModelConfig) -> Self {
        Self {
            grid: Grid { nb: cfg.n_bins() },
            pats: vec![None; g.patterns.len()],
            dropped_out_of_hours: 0.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        g: &Gtfs,
        cfg: &ModelConfig,
        cal: &DayCalendar,
        groups: &Groups,
        t: &Ticket,
        a: &Assigned,
        weight: f32,
    ) {
        let c = groups.canon[a.pattern as usize] as usize;
        let map = &groups.map[a.pattern as usize];
        let n = g.patterns[c].stops.len();
        let grid = self.grid;
        let acc = self.pats[c].get_or_insert_with(|| PatternAccum {
            cross: vec![0.0; grid.len(n - 1)],
            board: vec![0.0; grid.len(n)],
            alight: vec![0.0; grid.len(n)],
            ..Default::default()
        });
        let dt = cal.day_type(t.date).idx();
        let times = &g.trips[a.trip as usize].times;
        let b = a.board as usize;
        let at = |k: usize| t.secs + times[k].saturating_sub(times[b]);
        let pax = a.pax * weight;
        acc.tickets += pax;
        if let Some(ac) = t.is_ac {
            acc.ac_known += pax;
            if ac {
                acc.ac_tickets += pax;
            }
        }
        match cfg.bin_of(t.secs) {
            Some(bin) => acc.board[grid.idx(map[b] as usize, dt, bin)] += pax,
            None => self.dropped_out_of_hours += pax as f64,
        }
        for &(al, w) in &a.alight {
            let al = al as usize;
            let px = pax * w;
            for k in b..al {
                if let Some(bin) = cfg.bin_of(at(k)) {
                    for j in map[k] as usize..map[k + 1] as usize {
                        acc.cross[grid.idx(j, dt, bin)] += px;
                    }
                }
            }
            if let Some(bin) = cfg.bin_of(at(al)) {
                acc.alight[grid.idx(map[al] as usize, dt, bin)] += px;
            }
        }
    }
}

/// Scheduled bus passes per segment, `[segment][dt][bin]`, summed over the days of each
/// day type in the analysis period ("trip-days").
pub fn supply(
    g: &Gtfs,
    cfg: &ModelConfig,
    dates: &[Vec<NaiveDate>; N_DAYTYPES],
    groups: &Groups,
) -> Vec<Vec<f32>> {
    let grid = Grid { nb: cfg.n_bins() };
    let mut active: FxHashMap<&str, [f32; N_DAYTYPES]> = FxHashMap::default();
    let mut out: Vec<Vec<f32>> = g
        .patterns
        .iter()
        .enumerate()
        .map(|(pid, p)| {
            if groups.is_canonical(pid as u32) {
                vec![0.0f32; grid.len(p.stops.len() - 1)]
            } else {
                Vec::new()
            }
        })
        .collect();
    for (pid, p) in g.patterns.iter().enumerate() {
        let c = groups.canon[pid] as usize;
        let map = &groups.map[pid];
        for &ti in &p.trips {
            let trip = &g.trips[ti as usize];
            let days = *active.entry(trip.service_id.as_str()).or_insert_with(|| {
                let mut d = [0.0; N_DAYTYPES];
                for (dt, ds) in dates.iter().enumerate() {
                    d[dt] = ds
                        .iter()
                        .filter(|&&x| g.calendar.active(&trip.service_id, x))
                        .count() as f32;
                }
                d
            });
            for k in 0..p.stops.len() - 1 {
                if let Some(bin) = cfg.bin_of(trip.times[k]) {
                    for j in map[k] as usize..map[k + 1] as usize {
                        for (dt, &cnt) in days.iter().enumerate() {
                            out[c][grid.idx(j, dt, bin)] += cnt;
                        }
                    }
                }
            }
        }
    }
    out
}

/// How many buses actually run, compared with the timetable.
///
/// A bus that runs all day sells about 8 vehicle-tagged tickets, so the distinct vehicle
/// numbers seen per day are, in practice, the buses in service. The timetable's own peak
/// (most trips in progress at once) is the fleet it needs. Their ratio, per day type, scales
/// the timetable supply: an old or aspirational timetable that lists more trips than are run
/// would otherwise spread riders over buses that do not exist.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct FleetCalibration {
    /// Distinct vehicles selling a tagged ticket on a modelled route, per day.
    pub vehicles_seen: [f64; N_DAYTYPES],
    /// Most timetabled trips in progress at once (05:00-22:00), per day.
    pub timetable_peak: [f64; N_DAYTYPES],
    /// Factor applied to timetable supply (1 = timetable as is).
    pub scale: [f64; N_DAYTYPES],
}

pub fn fleet_calibration(
    g: &Gtfs,
    tickets: &[Ticket],
    assigned: &[Assigned],
    cal: &DayCalendar,
    dates: &[Vec<NaiveDate>; N_DAYTYPES],
    min_scale: f64,
) -> FleetCalibration {
    let mut out = FleetCalibration::default();
    // Vehicles seen per date.
    let mut per_date: FxHashMap<NaiveDate, rustc_hash::FxHashSet<u32>> = FxHashMap::default();
    for a in assigned {
        let t = &tickets[a.ticket as usize];
        if let Some(v) = t.vehicle {
            per_date.entry(t.date).or_default().insert(v);
        }
    }
    let mut sum = [0.0f64; N_DAYTYPES];
    let mut n = [0.0f64; N_DAYTYPES];
    for (d, vs) in &per_date {
        let dt = cal.day_type(*d).idx();
        sum[dt] += vs.len() as f64;
        n[dt] += 1.0;
    }
    // Timetable peak: trips in progress every 15 minutes, on up to three dates per day type.
    let mut active_cache: FxHashMap<(&str, NaiveDate), bool> = FxHashMap::default();
    for dt in 0..N_DAYTYPES {
        out.vehicles_seen[dt] = if n[dt] > 0.0 { sum[dt] / n[dt] } else { 0.0 };
        let mut peaks = Vec::new();
        for &d in dates[dt].iter().take(3) {
            let mut counts = vec![0u32; (22 - 5) * 4 + 1];
            for t in &g.trips {
                if t.times.len() < 2 {
                    continue;
                }
                let on = *active_cache
                    .entry((t.service_id.as_str(), d))
                    .or_insert_with(|| g.calendar.active(&t.service_id, d));
                if !on {
                    continue;
                }
                let (a, b) = (t.times[0], *t.times.last().unwrap());
                for (i, c) in counts.iter_mut().enumerate() {
                    let x = 5 * 3600 + i as u32 * 900;
                    if a <= x && x <= b {
                        *c += 1;
                    }
                }
            }
            peaks.push(*counts.iter().max().unwrap_or(&0) as f64);
        }
        out.timetable_peak[dt] = if peaks.is_empty() {
            0.0
        } else {
            peaks.iter().sum::<f64>() / peaks.len() as f64
        };
        out.scale[dt] = if out.timetable_peak[dt] > 0.0 && out.vehicles_seen[dt] > 0.0 {
            (out.vehicles_seen[dt] / out.timetable_peak[dt]).clamp(min_scale, 1.0)
        } else {
            1.0
        };
    }
    out
}

/// Inverse capture probability of a bus trip in the ticket data.
///
/// A trip appears in the vehicle-tagged tickets only if it carries at least one such
/// ticket. With `o` trips seen per day carrying `t` tagged tickets, and tickets falling on
/// trips as a Poisson process, the number of trips that ran, `n`, solves
/// `o = n (1 - exp(-t / n))`. Returns `n / o`, capped (a cap is needed when nearly every
/// seen trip carries a single ticket and `n` is not identified).
pub fn inverse_capture(o: f64, t: f64, cap: f64) -> f64 {
    if o <= 0.0 || t <= 0.0 {
        return 1.0;
    }
    let r = (o / t).min(1.0);
    if r >= 0.999 {
        return cap;
    }
    let f = |x: f64| (1.0 - (-x).exp()) / x;
    let (mut lo, mut hi) = (1e-6, 60.0);
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if f(mid) > r {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let n = t / (0.5 * (lo + hi));
    (n / o).clamp(1.0, cap)
}

/// Bus passes per segment implied by trips rebuilt from tickets, `[segment][dt][bin]` in
/// trip-days, corrected for trips that carried no tagged ticket. Each rebuilt trip is
/// placed along the pattern using the running times of its nearest scheduled trip.
pub fn observed_supply(
    g: &Gtfs,
    cfg: &ModelConfig,
    cal: &DayCalendar,
    recon: &[crate::trips::ReconTrip],
    cap: f64,
    groups: &Groups,
) -> (Vec<Option<Vec<f32>>>, Vec<[f32; N_DAYTYPES]>) {
    let grid = Grid { nb: cfg.n_bins() };
    // A bus sells more tickets at the peak than at midday, so the chance that a trip goes
    // unseen depends on the time of day. Capture correction per (pattern, day type, 2-hour
    // band), falling back to (pattern, day type), then to the city for that band.
    const BANDS: usize = 12;
    let band = |secs: u32| ((secs / 7200) as usize).min(BANDS - 1);
    let np = g.patterns.len();
    let mut seen_b = vec![[[0.0f64; BANDS]; N_DAYTYPES]; np];
    let mut tix_b = vec![[[0.0f64; BANDS]; N_DAYTYPES]; np];
    let mut seen_all = [[0.0f64; BANDS]; N_DAYTYPES];
    let mut tix_all = [[0.0f64; BANDS]; N_DAYTYPES];
    for r in recon {
        let dt = cal.day_type(r.date).idx();
        let c = groups.canon[r.pattern as usize] as usize;
        let b = band(r.anchor_secs);
        seen_b[c][dt][b] += 1.0;
        tix_b[c][dt][b] += r.tickets as f64;
        seen_all[dt][b] += 1.0;
        tix_all[dt][b] += r.tickets as f64;
    }
    let city = |dt: usize, b: usize| -> f64 {
        if seen_all[dt][b] >= 50.0 {
            inverse_capture(seen_all[dt][b], tix_all[dt][b], cap)
        } else {
            let (o, t): (f64, f64) = (seen_all[dt].iter().sum(), tix_all[dt].iter().sum());
            inverse_capture(o, t, cap)
        }
    };
    let inv_band = |p: usize, dt: usize, b: usize| -> f64 {
        if seen_b[p][dt][b] >= 20.0 {
            return inverse_capture(seen_b[p][dt][b], tix_b[p][dt][b], cap);
        }
        let (o, t): (f64, f64) = (seen_b[p][dt].iter().sum(), tix_b[p][dt].iter().sum());
        if o >= 30.0 {
            // Route level, adjusted by how this band compares with the city's day.
            inverse_capture(o, t, cap)
        } else {
            city(dt, b)
        }
    };
    // Reported per (pattern, day type): the trip-weighted mean correction.
    let inv: Vec<[f32; N_DAYTYPES]> = (0..np)
        .map(|p| {
            let mut out = [1.0f32; N_DAYTYPES];
            for (dt, o) in out.iter_mut().enumerate() {
                let n: f64 = seen_b[p][dt].iter().sum();
                if n > 0.0 {
                    let w: f64 = (0..BANDS)
                        .map(|b| seen_b[p][dt][b] * inv_band(p, dt, b))
                        .sum();
                    *o = (w / n) as f32;
                }
            }
            out
        })
        .collect();
    let mut obs: Vec<Option<Vec<f32>>> = vec![None; g.patterns.len()];
    for r in recon {
        let p = &g.patterns[r.pattern as usize];
        let c = groups.canon[r.pattern as usize] as usize;
        let map = &groups.map[r.pattern as usize];
        let nc = g.patterns[c].stops.len();
        let dt = cal.day_type(r.date).idx();
        let w = inv_band(c, dt, band(r.anchor_secs)) as f32;
        let times = &g.trips[r.anchor_trip as usize].times;
        let base = times[r.anchor_board as usize] as i64;
        let o = obs[c].get_or_insert_with(|| vec![0.0; grid.len(nc - 1)]);
        for k in 0..p.stops.len() - 1 {
            let vt = r.anchor_secs as i64 + times[k] as i64 - base;
            if vt < 0 {
                continue;
            }
            if let Some(bin) = cfg.bin_of(vt as u32) {
                for j in map[k] as usize..map[k + 1] as usize {
                    o[grid.idx(j, dt, bin)] += w;
                }
            }
        }
    }
    (obs, inv)
}

/// Sufficient statistics for one pattern; everything else is derived at read time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cells {
    /// Shrunk ticketed riders on board per bus, `[segment][dt][bin]`. NaN where no bus runs.
    pub rate: Vec<f32>,
    /// Kernel-weighted ticketed riders behind the rate.
    pub tickets: Vec<f32>,
    /// Scheduled buses per day.
    pub buses: Vec<f32>,
    /// Ticketed boardings and alightings per day, `[stop pos][dt][bin]`.
    pub board: Vec<f32>,
    pub alight: Vec<f32>,
    /// Per segment cell: bit 0 = no timetabled bus in the bin (supply from tickets only),
    /// bit 1 = thin supply (under `THIN_SUPPLY` trip-days behind the bin).
    #[serde(default)]
    pub flags: Vec<u8>,
}

pub const FLAG_TICKET_SUPPLY_ONLY: u8 = 1;
pub const FLAG_THIN_SUPPLY: u8 = 2;
/// Kernel-weighted trip-days below which the bus count is considered thin.
pub const THIN_SUPPLY: f64 = 10.0;

/// City-wide time-of-day shape of the ticketed rate per bus, `[dt][bin]`, normalised so
/// the supply-weighted mean of each day type is 1. Used as the outermost shrinkage prior.
pub fn city_shape(
    cfg: &ModelConfig,
    accs: &[Option<PatternAccum>],
    sups: &[Vec<f32>],
) -> Vec<Vec<f64>> {
    let grid = Grid { nb: cfg.n_bins() };
    let nb = grid.nb;
    let mut x = vec![vec![0.0f64; nb]; N_DAYTYPES];
    let mut s = vec![vec![0.0f64; nb]; N_DAYTYPES];
    for (acc, sup) in accs.iter().zip(sups) {
        let Some(acc) = acc else { continue };
        let nseg = acc.cross.len() / (N_DAYTYPES * nb);
        for seg in 0..nseg {
            for dt in 0..N_DAYTYPES {
                for bin in 0..nb {
                    let i = grid.idx(seg, dt, bin);
                    x[dt][bin] += acc.cross[i] as f64;
                    s[dt][bin] += sup[i] as f64;
                }
            }
        }
    }
    (0..N_DAYTYPES)
        .map(|dt| {
            let (tx, ts): (f64, f64) = (x[dt].iter().sum(), s[dt].iter().sum());
            let overall = if ts > 0.0 { tx / ts } else { 0.0 };
            (0..nb)
                .map(|b| {
                    if s[dt][b] > 0.0 && overall > 0.0 {
                        (x[dt][b] / s[dt][b] / overall).max(0.05)
                    } else {
                        1.0
                    }
                })
                .collect()
        })
        .collect()
}

/// Build the per-cell statistics for one pattern with nested shrinkage:
///
/// 1. route level: ticketed riders per bus over the whole day, adjusted for time of day
///    with the city shape;
/// 2. segment level: the segment's whole-day rate, shrunk toward the route level;
/// 3. window level: a +/- `prior_window_bins` window of the segment, shrunk toward
///    (segment level x city shape of the bin);
/// 4. bin level: the kernel-smoothed bin, shrunk toward the window level.
///
/// Each step uses `shrink_pseudo_trips` trip-days of prior weight.
pub fn build_cells(
    cfg: &ModelConfig,
    acc: &PatternAccum,
    sup: &[f32],
    n_stops: usize,
    n_days: &[u32; N_DAYTYPES],
    shape: &[Vec<f64>],
    sched: Option<&[f32]>,
) -> Cells {
    let grid = Grid { nb: cfg.n_bins() };
    let nb = grid.nb;
    let nseg = n_stops - 1;
    let ksum: f64 = cfg.kernel.iter().sum();
    let kern: Vec<f64> = cfg.kernel.iter().map(|k| k / ksum).collect();
    let half = (kern.len() / 2) as isize;
    let w = cfg.prior_window_bins as isize;
    let m = cfg.shrink_pseudo_trips;
    let mut c = Cells {
        rate: vec![f32::NAN; grid.len(nseg)],
        tickets: vec![0.0; grid.len(nseg)],
        buses: vec![0.0; grid.len(nseg)],
        board: vec![0.0; grid.len(n_stops)],
        alight: vec![0.0; grid.len(n_stops)],
        flags: vec![0; grid.len(nseg)],
    };
    let at = |arr: &[f32], seg: usize, dt: usize, b: isize| -> f64 {
        if b >= 0 && (b as usize) < nb {
            arr[grid.idx(seg, dt, b as usize)] as f64
        } else {
            0.0
        }
    };
    for dt in 0..N_DAYTYPES {
        if n_days[dt] == 0 {
            continue;
        }
        let sh = &shape[dt];
        // Route level, in shape-normalised units.
        let (mut px, mut pes) = (0.0, 0.0);
        for seg in 0..nseg {
            for bin in 0..nb {
                let i = grid.idx(seg, dt, bin);
                px += acc.cross[i] as f64;
                pes += sup[i] as f64 * sh[bin];
            }
        }
        if pes <= 0.0 {
            continue;
        }
        let route_level = px / pes;
        for seg in 0..nseg {
            let (mut sx, mut ses) = (0.0, 0.0);
            for bin in 0..nb {
                let i = grid.idx(seg, dt, bin);
                sx += acc.cross[i] as f64;
                ses += sup[i] as f64 * sh[bin];
            }
            let seg_level = (sx + m * route_level) / (ses + m);
            for bin in 0..nb {
                let i = grid.idx(seg, dt, bin);
                let (mut x, mut s) = (0.0, 0.0);
                for (j, k) in kern.iter().enumerate() {
                    let b = bin as isize + j as isize - half;
                    x += k * at(&acc.cross, seg, dt, b);
                    s += k * at(sup, seg, dt, b);
                }
                // No bus seen on this stretch around this time: no estimate.
                if s <= 0.0 {
                    continue;
                }
                let (mut xw, mut sw) = (0.0, 0.0);
                for j in -w..=w {
                    xw += at(&acc.cross, seg, dt, bin as isize + j);
                    sw += at(sup, seg, dt, bin as isize + j);
                }
                let prior2 = seg_level * sh[bin];
                let prior1 = (xw + m * prior2) / (sw + m);
                c.rate[i] = ((x + m * prior1) / (s + m)) as f32;
                c.tickets[i] = x as f32;
                // Buses per day, smoothed like the riders.
                c.buses[i] = (s / n_days[dt] as f64) as f32;
                if sched.is_some_and(|sc| sc[i] <= 0.0) {
                    c.flags[i] |= FLAG_TICKET_SUPPLY_ONLY;
                }
                if s < THIN_SUPPLY {
                    c.flags[i] |= FLAG_THIN_SUPPLY;
                }
            }
        }
    }
    for pos in 0..n_stops {
        for dt in 0..N_DAYTYPES {
            if n_days[dt] == 0 {
                continue;
            }
            for bin in 0..nb {
                let i = grid.idx(pos, dt, bin);
                c.board[i] = acc.board[i] / n_days[dt] as f32;
                c.alight[i] = acc.alight[i] / n_days[dt] as f32;
            }
        }
    }
    c
}

/// Group dates of a period by day type.
pub fn dates_by_daytype(
    cal: &DayCalendar,
    from: NaiveDate,
    to: NaiveDate,
) -> [Vec<NaiveDate>; N_DAYTYPES] {
    let mut out: [Vec<NaiveDate>; N_DAYTYPES] = Default::default();
    let mut d = from;
    while d <= to {
        out[cal.day_type(d).idx()].push(d);
        d = d.succ_opt().expect("date in range");
    }
    out
}

pub fn n_days(dates: &[Vec<NaiveDate>; N_DAYTYPES]) -> [u32; N_DAYTYPES] {
    [
        dates[0].len() as u32,
        dates[1].len() as u32,
        dates[2].len() as u32,
    ]
}

pub fn daytype_label(i: usize) -> &'static str {
    DayType::from_idx(i).as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assign::assign_all;
    use crate::testkit as fixture;
    use crate::tickets::{ist, Interner, RawTicket};

    #[test]
    fn running_count_pools_over_scheduled_trips() {
        let g = Gtfs::load(&fixture::write_feed()).unwrap();
        let cfg = ModelConfig {
            kernel: vec![1.0],
            shrink_pseudo_trips: 0.0,
            ..Default::default()
        };
        let cal = DayCalendar::new(&[]).unwrap();
        let mut names = Interner::default();
        // Two riders A->C and one rider B->E on the 08:00 trip of 34UP, on Monday 5 Oct.
        let mk = |a: &str, b: &str, at: &str| RawTicket {
            passenger_count: Some(1.0),
            amount: Some(9.0),
            vehicle_number: None,
            ticket_type_id: Some(1),
            start_stop_name: a.into(),
            end_stop_name: Some(b.into()),
            route: "34UP".into(),
            created_at: format!("2026-10-05 {at}+05:30"),
            is_ac: Some(true),
        };
        let raws = [
            mk("Anand Vihar ISBT", "ITO", "08:00:10"),
            mk("Anand Vihar ISBT", "ITO", "08:00:20"),
            mk("Laxmi Nagar Crossing", "Connaught Place (T)", "08:10:00"),
        ];
        let tickets: Vec<_> = raws
            .iter()
            .map(|r| r.parse(&mut names, ist()).unwrap())
            .collect();
        let (assigned, _) = assign_all(&g, &cfg, &tickets, &names);
        let mut acc = Accumulator::new(&g, &cfg);
        let groups = Groups::identity(&g);
        for a in &assigned {
            acc.add(&g, &cfg, &cal, &groups, &tickets[a.ticket as usize], a, 1.0);
        }
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let dates = dates_by_daytype(&cal, day, day);
        let sup = supply(&g, &cfg, &dates, &groups);
        let pid = assigned[0].pattern as usize;
        let shape = vec![vec![1.0; cfg.n_bins()]; N_DAYTYPES];
        let cells = build_cells(
            &cfg,
            acc.pats[pid].as_ref().unwrap(),
            &sup[pid],
            5,
            &n_days(&dates),
            &shape,
            None,
        );
        let grid = Grid { nb: cfg.n_bins() };
        let bin8 = cfg.bin_of(8 * 3600).unwrap();
        let wd = DayType::Weekday.idx();
        // Segment A-B: 2 riders; B-C: 3 riders; C-D: 1; D-E: 1 (D-E is crossed at 08:30).
        assert_eq!(cells.rate[grid.idx(0, wd, bin8)], 2.0);
        assert_eq!(cells.rate[grid.idx(1, wd, bin8)], 3.0);
        assert_eq!(cells.rate[grid.idx(2, wd, bin8)], 1.0);
        let bin830 = cfg.bin_of(8 * 3600 + 30 * 60).unwrap();
        assert_eq!(cells.rate[grid.idx(3, wd, bin830)], 1.0);
        assert_eq!(cells.buses[grid.idx(0, wd, bin8)], 1.0);
        // No bus runs at 12:00 in the fixture.
        assert!(cells.rate[grid.idx(0, wd, cfg.bin_of(12 * 3600).unwrap())].is_nan());
        assert_eq!(cells.board[grid.idx(0, wd, bin8)], 2.0);
    }

    #[test]
    fn capture_correction() {
        // Every trip seen with many tickets: nothing missed.
        assert!((inverse_capture(10.0, 100.0, 3.0) - 1.0).abs() < 1e-3);
        // 1.76 tagged tickets per seen trip: about 1.4 trips ran per trip seen.
        let k = inverse_capture(100.0, 176.0, 3.0);
        assert!((1.3..1.5).contains(&k), "{k}");
        // One ticket per seen trip: not identified, capped.
        assert_eq!(inverse_capture(50.0, 50.0, 3.0), 3.0);
    }

    #[test]
    fn shrinkage_pulls_sparse_bins_toward_the_window() {
        let cfg = ModelConfig {
            kernel: vec![1.0],
            prior_window_bins: 2,
            ..Default::default()
        };
        let nb = cfg.n_bins();
        let grid = Grid { nb };
        let n_stops = 2;
        let mut acc = PatternAccum {
            cross: vec![0.0; grid.len(1)],
            board: vec![0.0; grid.len(2)],
            alight: vec![0.0; grid.len(2)],
            ..Default::default()
        };
        let mut sup = vec![0.0; grid.len(1)];
        for b in 10..15 {
            sup[grid.idx(0, 0, b)] = 20.0;
            acc.cross[grid.idx(0, 0, b)] = 20.0; // 1 ticketed rider per bus
        }
        acc.cross[grid.idx(0, 0, 12)] = 100.0; // one noisy bin: 5 per bus
        let shape = vec![vec![1.0; nb]; N_DAYTYPES];
        let c = build_cells(&cfg, &acc, &sup, n_stops, &[20, 4, 6], &shape, None);
        let r = c.rate[grid.idx(0, 0, 12)];
        assert!(r < 5.0 && r > 1.8, "{r}");
    }
}
