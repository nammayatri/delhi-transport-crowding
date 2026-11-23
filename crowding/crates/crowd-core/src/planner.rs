//! A compact RAPTOR journey planner over the snapshot's timetable.
//!
//! OpenTripPlanner is the production planner (see the `otp-crowding` patch). This one lets
//! the service and demo work on their own, and doubles as a reference for the crowding cost:
//! with `crowding_reluctance` > 0, labels are compared on clock time plus reluctance times
//! the crowding-extra seconds of the rides taken (a generalised-cost heuristic; boarding
//! still uses clock time).

use chrono::NaiveDate;
use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::geo::haversine_m;
use crate::levels::ScenarioPick;
use crate::snapshot::Snapshot;

const INF: u32 = u32::MAX / 2;

#[derive(Debug, Clone)]
pub struct PlannerConfig {
    pub max_transfers: usize,
    pub walk_speed_mps: f64,
    /// Straight-line distance is multiplied by this to approximate walking distance.
    pub walk_detour: f64,
    pub max_access_m: f64,
    pub max_transfer_m: f64,
    pub transfer_slack_secs: u32,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            max_transfers: 2,
            walk_speed_mps: 1.2,
            walk_detour: 1.3,
            max_access_m: 900.0,
            max_transfer_m: 350.0,
            transfer_slack_secs: 60,
        }
    }
}

/// Static indexes built once per snapshot.
pub struct Network {
    pub stop_patterns: Vec<Vec<(u32, u16)>>,
    pub footpaths: Vec<Vec<(u32, u32)>>,
    grid: FxHashMap<(i32, i32), Vec<u32>>,
    cfg: PlannerConfig,
}

const CELL: f64 = 0.005;

fn cell(lat: f64, lon: f64) -> (i32, i32) {
    ((lat / CELL).floor() as i32, (lon / CELL).floor() as i32)
}

impl Network {
    pub fn build(s: &Snapshot, cfg: PlannerConfig) -> Self {
        let mut stop_patterns = vec![Vec::new(); s.stops.len()];
        for (pid, p) in s.patterns.iter().enumerate() {
            for (pos, &st) in p.stops.iter().enumerate() {
                stop_patterns[st as usize].push((pid as u32, pos as u16));
            }
        }
        let mut grid: FxHashMap<(i32, i32), Vec<u32>> = FxHashMap::default();
        for (i, st) in s.stops.iter().enumerate() {
            grid.entry(cell(st.lat, st.lon)).or_default().push(i as u32);
        }
        let mut net = Self {
            stop_patterns,
            footpaths: Vec::new(),
            grid,
            cfg,
        };
        let footpaths = (0..s.stops.len())
            .map(|i| {
                let st = &s.stops[i];
                net.nearby(s, st.lat, st.lon, net.cfg.max_transfer_m)
                    .into_iter()
                    .filter(|(j, _)| *j as usize != i)
                    .collect()
            })
            .collect();
        net.footpaths = footpaths;
        net
    }

    /// Stops within `radius_m`, with walking seconds.
    pub fn nearby(&self, s: &Snapshot, lat: f64, lon: f64, radius_m: f64) -> Vec<(u32, u32)> {
        let (ci, cj) = cell(lat, lon);
        let r = (radius_m / 500.0).ceil() as i32 + 1;
        let mut out = Vec::new();
        for di in -r..=r {
            for dj in -r..=r {
                if let Some(v) = self.grid.get(&(ci + di, cj + dj)) {
                    for &st in v {
                        let p = &s.stops[st as usize];
                        let d = haversine_m(lat, lon, p.lat, p.lon) * self.cfg.walk_detour;
                        if d <= radius_m {
                            out.push((st, (d / self.cfg.walk_speed_mps).round() as u32));
                        }
                    }
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Place {
    pub lat: f64,
    pub lon: f64,
    /// Stop index when the place is a stop.
    pub stop: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Leg {
    Walk {
        from: Place,
        to: Place,
        depart: u32,
        arrive: u32,
    },
    Bus {
        trip: u32,
        board: u16,
        alight: u16,
        depart: u32,
        arrive: u32,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Journey {
    pub depart: u32,
    pub arrive: u32,
    pub transfers: usize,
    pub legs: Vec<Leg>,
    /// Crowding-extra seconds summed over bus legs (mid scenario), used for ranking.
    pub crowding_extra_secs: f64,
}

#[derive(Debug, Clone, Copy)]
enum Parent {
    None,
    Access {
        walk: u32,
    },
    Ride {
        trip: u32,
        board: u16,
        alight: u16,
        from_stop: u32,
    },
    Walk {
        from_stop: u32,
        secs: u32,
    },
}

pub struct Query {
    pub from: (f64, f64),
    pub to: (f64, f64),
    pub date: NaiveDate,
    pub depart: u32,
    pub crowding_reluctance: f64,
}

/// Run RAPTOR and return the best journey per number of transfers (Pareto on transfers and
/// arrival or cost).
pub fn plan(s: &Snapshot, net: &Network, active: &[bool], q: &Query) -> Vec<Journey> {
    let n = s.stops.len();
    let cfg = &net.cfg;
    let dt = s.day_calendar().day_type(q.date);
    let access = net.nearby(s, q.from.0, q.from.1, cfg.max_access_m);
    let egress: FxHashMap<u32, u32> = net
        .nearby(s, q.to.0, q.to.1, cfg.max_access_m)
        .into_iter()
        .collect();
    if access.is_empty() || egress.is_empty() {
        return Vec::new();
    }
    let rounds = cfg.max_transfers + 2;
    // Per round: clock arrival, generalised cost, parent.
    let mut arr = vec![vec![INF; n]; rounds];
    let mut cost = vec![vec![f64::INFINITY; n]; rounds];
    let mut par = vec![vec![Parent::None; n]; rounds];
    let mut best_cost = vec![f64::INFINITY; n];
    let mut marked = vec![false; n];
    for &(st, w) in &access {
        let t = q.depart + w;
        if (t as f64) < cost[0][st as usize] {
            arr[0][st as usize] = t;
            cost[0][st as usize] = t as f64;
            par[0][st as usize] = Parent::Access { walk: w };
            best_cost[st as usize] = t as f64;
            marked[st as usize] = true;
        }
    }
    let mut target_best = f64::INFINITY;
    let crowd = |trip: u32| -> Vec<u32> {
        if q.crowding_reluctance > 0.0 {
            s.trip_cumulative(trip, dt, ScenarioPick::Mid, 1.0).0
        } else {
            Vec::new()
        }
    };
    let mut cum_cache: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut out = Vec::new();
    for k in 1..rounds {
        // Patterns to scan, from their earliest marked position.
        let mut queue: FxHashMap<u32, u16> = FxHashMap::default();
        for (st, m) in marked.iter_mut().enumerate() {
            if !*m {
                continue;
            }
            *m = false;
            for &(p, pos) in &net.stop_patterns[st] {
                let e = queue.entry(p).or_insert(pos);
                *e = (*e).min(pos);
            }
        }
        if queue.is_empty() {
            break;
        }
        let mut new_marked = vec![false; n];
        for (&pid, &start) in &queue {
            let p = &s.patterns[pid as usize];
            let mut cur: Option<(u32, u16, f64)> = None; // (trip, board pos, cost at boarding)
            for pos in start as usize..p.stops.len() {
                let st = p.stops[pos] as usize;
                if let Some((trip, bpos, bcost)) = cur {
                    let times = &s.trips[trip as usize].times;
                    let t_arr = times[pos];
                    let extra = if q.crowding_reluctance > 0.0 {
                        let c = cum_cache.entry(trip).or_insert_with(|| crowd(trip));
                        q.crowding_reluctance * (c[pos] as f64 - c[bpos as usize] as f64)
                    } else {
                        0.0
                    };
                    let c_arr = bcost + (t_arr - times[bpos as usize]) as f64 + extra;
                    if c_arr < best_cost[st].min(target_best) {
                        arr[k][st] = t_arr;
                        cost[k][st] = c_arr;
                        best_cost[st] = c_arr;
                        par[k][st] = Parent::Ride {
                            trip,
                            board: bpos,
                            alight: pos as u16,
                            from_stop: p.stops[bpos as usize],
                        };
                        new_marked[st] = true;
                    }
                }
                // Can we catch an earlier trip here?
                let ready = arr[k - 1][st];
                if ready >= INF || pos + 1 == p.stops.len() {
                    continue;
                }
                let need = if k == 1 {
                    ready
                } else {
                    ready + cfg.transfer_slack_secs
                };
                let cur_dep = cur
                    .map(|(t, _, _)| s.trips[t as usize].times[pos])
                    .unwrap_or(INF);
                if need < cur_dep {
                    let i = p
                        .trips
                        .partition_point(|&t| s.trips[t as usize].times[pos] < need);
                    if let Some(&t) = p.trips[i..].iter().find(|&&t| active[t as usize]) {
                        if s.trips[t as usize].times[pos] < cur_dep {
                            let dep = s.trips[t as usize].times[pos];
                            let c0 = cost[k - 1][st] + (dep - ready) as f64;
                            cur = Some((t, pos as u16, c0));
                        }
                    }
                }
            }
        }
        // Footpaths from stops reached by bus in this round.
        let reached: Vec<usize> = (0..n).filter(|&i| new_marked[i]).collect();
        for st in reached {
            for &(to, w) in &net.footpaths[st] {
                let c = cost[k][st] + w as f64;
                let to = to as usize;
                if c < best_cost[to].min(target_best) && c < cost[k][to] {
                    arr[k][to] = arr[k][st] + w;
                    cost[k][to] = c;
                    best_cost[to] = c;
                    par[k][to] = Parent::Walk {
                        from_stop: st as u32,
                        secs: w,
                    };
                    new_marked[to] = true;
                }
            }
        }
        marked = new_marked;
        // Best arrival at the destination this round.
        let mut best: Option<(f64, u32, u32)> = None;
        for (&st, &w) in &egress {
            let c = cost[k][st as usize];
            if c.is_finite() && best.is_none_or(|b| c + (w as f64) < b.0) {
                best = Some((c + w as f64, st, w));
            }
        }
        if let Some((c, st, w)) = best {
            if c < target_best {
                target_best = c;
                if let Some(j) = reconstruct(s, &arr, &par, k, st, w, q) {
                    out.push(j);
                }
            }
        }
    }
    for j in &mut out {
        j.crowding_extra_secs = j
            .legs
            .iter()
            .map(|l| match l {
                Leg::Bus {
                    trip,
                    board,
                    alight,
                    ..
                } => {
                    s.leg(
                        *trip,
                        *board as usize,
                        *alight as usize,
                        dt,
                        ScenarioPick::Mid,
                        1.0,
                    )
                    .crowding_extra_secs
                }
                _ => 0.0,
            })
            .sum();
    }
    out
}

fn reconstruct(
    s: &Snapshot,
    arr: &[Vec<u32>],
    par: &[Vec<Parent>],
    k: usize,
    last_stop: u32,
    egress_walk: u32,
    q: &Query,
) -> Option<Journey> {
    let stop_place = |i: u32| {
        let st = &s.stops[i as usize];
        Place {
            lat: st.lat,
            lon: st.lon,
            stop: Some(i),
        }
    };
    let mut legs = Vec::new();
    let t_end = arr[k][last_stop as usize] + egress_walk;
    legs.push(Leg::Walk {
        from: stop_place(last_stop),
        to: Place {
            lat: q.to.0,
            lon: q.to.1,
            stop: None,
        },
        depart: arr[k][last_stop as usize],
        arrive: t_end,
    });
    let (mut round, mut st) = (k, last_stop);
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 32 {
            return None;
        }
        match par[round][st as usize] {
            Parent::Ride {
                trip,
                board,
                alight,
                from_stop,
            } => {
                let times = &s.trips[trip as usize].times;
                legs.push(Leg::Bus {
                    trip,
                    board,
                    alight,
                    depart: times[board as usize],
                    arrive: times[alight as usize],
                });
                st = from_stop;
                round -= 1;
            }
            Parent::Walk { from_stop, secs } => {
                legs.push(Leg::Walk {
                    from: stop_place(from_stop),
                    to: stop_place(st),
                    depart: arr[round][st as usize] - secs,
                    arrive: arr[round][st as usize],
                });
                st = from_stop;
            }
            Parent::Access { walk } => {
                legs.push(Leg::Walk {
                    from: Place {
                        lat: q.from.0,
                        lon: q.from.1,
                        stop: None,
                    },
                    to: stop_place(st),
                    depart: q.depart,
                    arrive: q.depart + walk,
                });
                break;
            }
            Parent::None => return None,
        }
    }
    legs.reverse();
    // Leave as late as possible: shift the access walk to just before the first bus.
    if let (
        Some(Leg::Walk { depart, arrive, .. }),
        Some(Leg::Bus {
            depart: bus_dep, ..
        }),
    ) = (legs.first().cloned(), legs.get(1).cloned())
    {
        let walk = arrive - depart;
        if let Some(Leg::Walk { depart, arrive, .. }) = legs.first_mut() {
            *arrive = bus_dep;
            *depart = bus_dep.saturating_sub(walk);
        }
    }
    let n_bus = legs.iter().filter(|l| matches!(l, Leg::Bus { .. })).count();
    let depart = match legs.first()? {
        Leg::Walk { depart, .. } | Leg::Bus { depart, .. } => *depart,
    };
    Some(Journey {
        depart,
        arrive: t_end,
        transfers: n_bus.saturating_sub(1),
        legs,
        crowding_extra_secs: 0.0,
    })
}

/// Active-trip mask for a date.
pub fn active_mask(s: &Snapshot, date: NaiveDate) -> Vec<bool> {
    let mut m = vec![false; s.trips.len()];
    for t in s.trips_on(date) {
        m[t as usize] = true;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::fixture_snapshot;

    #[test]
    fn plans_a_direct_bus_ride_and_reports_crowding() {
        let s = fixture_snapshot();
        let net = Network::build(&s, PlannerConfig::default());
        let date = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let active = active_mask(&s, date);
        let q = Query {
            from: (28.6469, 77.3159),
            to: (28.6315, 77.2167),
            date,
            depart: 7 * 3600 + 50 * 60,
            crowding_reluctance: 0.0,
        };
        let js = plan(&s, &net, &active, &q);
        assert!(!js.is_empty());
        let j = &js[0];
        assert_eq!(j.transfers, 0);
        let bus = j
            .legs
            .iter()
            .find(|l| matches!(l, Leg::Bus { .. }))
            .unwrap();
        if let Leg::Bus {
            depart,
            board,
            alight,
            ..
        } = bus
        {
            assert_eq!(*depart, 8 * 3600);
            assert_eq!((*board, *alight), (0, 4));
        }
        // 30 ticketed riders per day on the 08:00 trip between B and D: crowded there.
        assert!(j.crowding_extra_secs > 0.0, "{}", j.crowding_extra_secs);
    }
}
