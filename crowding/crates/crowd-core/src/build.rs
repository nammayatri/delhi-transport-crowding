//! End-to-end model build: tickets + GTFS -> [`Snapshot`] plus diagnostics.

use anyhow::Result;
use chrono::NaiveDate;

use crate::assign::{assign_all, Assigned, MatchReport};
use crate::capacity::infer_class;
use crate::config::ModelConfig;
use crate::daytype::{DayCalendar, DayType, N_DAYTYPES};
use crate::groups::{GroupReport, Groups};
use crate::gtfs::Gtfs;
use crate::levels::Expansion;
use crate::names::{norm_route, route_candidates};
use crate::profile::{
    build_cells, city_shape, dates_by_daytype, fleet_calibration, n_days, observed_supply, supply,
    Accumulator, FleetCalibration,
};
use crate::profile::{Cells, Grid};
use crate::snapshot::{Meta, SnapPattern, SnapTrip, Snapshot, FORMAT_VERSION};
use crate::tickets::{Interner, Ticket};
use crate::trips::{reconstruct, ReconTrip};
use rustc_hash::FxHashMap;

pub struct BuildOutput {
    pub snapshot: Snapshot,
    pub report: MatchReport,
    pub assigned: Vec<Assigned>,
    pub recon: Vec<ReconTrip>,
    /// Timetable and ticket-based bus passes, `[pattern][segment][dt][bin]` in trip-days.
    pub sup_sched: Vec<Vec<f32>>,
    pub sup_obs: Vec<Option<Vec<f32>>>,
    pub inv_capture: Vec<[f32; N_DAYTYPES]>,
    pub dropped_out_of_hours: f64,
    pub cells_supply_raised: usize,
    pub fleet: crate::profile::FleetCalibration,
    pub groups: GroupReport,
    /// Per canonical pattern: factor applied for tickets that could not be placed.
    pub unmatched_factor: Vec<f32>,
    /// Canonical pattern of each pattern (copies of a route run by a second operator
    /// point at the first).
    pub canon: Vec<u32>,
}

/// Copy a canonical `[segment][dt][bin]` (or `[stop][dt][bin]`) array onto another
/// pattern of the group using its position map.
fn remap(
    src: &[f32],
    map: &[u16],
    n_out: usize,
    n_src: usize,
    nb: usize,
    per_stop: bool,
) -> Vec<f32> {
    let grid = Grid { nb };
    let mut out = vec![0.0; grid.len(n_out)];
    for k in 0..n_out {
        let j = if per_stop {
            map[k] as usize
        } else {
            (map[k] as usize).min(n_src - 1)
        };
        for dt in 0..N_DAYTYPES {
            for b in 0..nb {
                out[grid.idx(k, dt, b)] = src[grid.idx(j.min(n_src - 1), dt, b)];
            }
        }
    }
    out
}

pub fn build(
    g: &Gtfs,
    cfg: &ModelConfig,
    tickets: &[Ticket],
    names: &Interner,
) -> Result<BuildOutput> {
    cfg.validate()?;
    anyhow::ensure!(!tickets.is_empty(), "no tickets");
    let cal = DayCalendar::new(&cfg.holidays)?;
    let date_from: NaiveDate = tickets.iter().map(|t| t.date).min().unwrap();
    let date_to: NaiveDate = tickets.iter().map(|t| t.date).max().unwrap();
    let dates = dates_by_daytype(&cal, date_from, date_to);
    let nd = n_days(&dates);

    // Expansion from ticketed riders to all riders.
    let wd_pax: f64 = tickets
        .iter()
        .filter(|t| cal.day_type(t.date) == DayType::Weekday)
        .map(|t| t.pax as f64)
        .sum();
    let ticketed_per_weekday = wd_pax / nd[0].max(1) as f64;
    let wb = cfg.ridership.weekday_boardings;
    let expansion = if cfg.ridership.fixed_expansion > 0.0 || ticketed_per_weekday <= 0.0 {
        let e = cfg.ridership.fixed_expansion.max(1.0);
        Expansion {
            low: e,
            mid: e,
            high: e,
        }
    } else {
        Expansion {
            low: wb.low / ticketed_per_weekday,
            mid: wb.mid / ticketed_per_weekday,
            high: wb.high / ticketed_per_weekday,
        }
    };
    tracing::info!(ticketed_per_weekday, ?expansion, "expansion factor");

    let (assigned, report) = assign_all(g, cfg, tickets, names);
    tracing::info!(assigned = assigned.len(), ?report.by_method, ?report.rejected, "assigned");

    let groups = if cfg.merge_route_copies {
        Groups::build(g, cfg.merge_max_median_m)
    } else {
        Groups::identity(g)
    };
    tracing::info!(report = ?groups.report, "route groups");

    // Tickets whose boarding stop could not be placed, per canonical pattern.
    let mut canon_by_name: FxHashMap<String, u32> = FxHashMap::default();
    for (ri, r) in g.routes.iter().enumerate() {
        if let Some(&p) = g.patterns_by_route[ri].first() {
            canon_by_name
                .entry(norm_route(r.name()))
                .or_insert(groups.canon[p as usize]);
        }
    }
    let mut route_canon: FxHashMap<u32, Option<u32>> = FxHashMap::default();
    let mut total = vec![0.0f64; g.patterns.len()];
    for t in tickets {
        let c = *route_canon.entry(t.route).or_insert_with(|| {
            route_candidates(names.get(t.route))
                .iter()
                .find_map(|n| canon_by_name.get(n).copied())
        });
        if let Some(c) = c {
            total[c as usize] += t.pax as f64;
        }
    }
    let mut placed = vec![0.0f64; g.patterns.len()];
    for a in &assigned {
        placed[groups.canon[a.pattern as usize] as usize] += a.pax as f64;
    }
    let unmatched_factor: Vec<f32> = (0..g.patterns.len())
        .map(|c| {
            if placed[c] > 0.0 {
                (total[c] / placed[c]).clamp(1.0, cfg.unmatched_correction_cap.max(1.0)) as f32
            } else {
                1.0
            }
        })
        .collect();

    let mut acc = Accumulator::new(g, cfg);
    for a in &assigned {
        let w = unmatched_factor[groups.canon[a.pattern as usize] as usize];
        acc.add(g, cfg, &cal, &groups, &tickets[a.ticket as usize], a, w);
    }
    let mut sup_sched = supply(g, cfg, &dates, &groups);
    let fleet = if cfg.fleet_calibration && cfg.supply_source != "tickets" {
        fleet_calibration(g, tickets, &assigned, &cal, &dates, cfg.fleet_min_scale)
    } else {
        FleetCalibration {
            scale: [1.0; N_DAYTYPES],
            ..Default::default()
        }
    };
    tracing::info!(?fleet, "fleet calibration");
    {
        let grid = Grid { nb: cfg.n_bins() };
        for arr in sup_sched.iter_mut().filter(|a| !a.is_empty()) {
            let nseg = arr.len() / grid.len(1);
            for seg in 0..nseg {
                for (dt, &f) in fleet.scale.iter().enumerate() {
                    for b in 0..grid.nb {
                        arr[grid.idx(seg, dt, b)] *= f as f32;
                    }
                }
            }
        }
    }
    let recon = reconstruct(g, tickets, &assigned, names, cfg.trip_split_gap_minutes);
    let (sup_obs, inv_capture) = observed_supply(g, cfg, &cal, &recon, cfg.capture_cap, &groups);
    let mut raised = 0usize;
    let sup: Vec<Vec<f32>> = sup_sched
        .iter()
        .zip(&sup_obs)
        .map(|(s, o)| match (cfg.supply_source.as_str(), o) {
            ("timetable", _) => s.clone(),
            ("max", Some(o)) => s
                .iter()
                .zip(o)
                .map(|(a, b)| {
                    if b > a {
                        raised += 1;
                    }
                    a.max(*b)
                })
                .collect(),
            ("max", None) => s.clone(),
            // "tickets": buses seen in the tickets only.
            (_, Some(o)) => o.clone(),
            (_, None) => vec![0.0; s.len()],
        })
        .collect();
    tracing::info!(
        source = %cfg.supply_source,
        recon_trips = recon.len(),
        cells_raised = raised,
        "bus supply"
    );

    let shape = city_shape(cfg, &acc.pats, &sup);
    let nb = cfg.n_bins();
    let canon_cells: Vec<Option<Cells>> = g
        .patterns
        .iter()
        .enumerate()
        .map(|(pid, p)| {
            let a = acc.pats[pid].as_ref()?;
            Some(build_cells(
                cfg,
                a,
                &sup[pid],
                p.stops.len(),
                &nd,
                &shape,
                (cfg.supply_source == "max").then(|| sup_sched[pid].as_slice()),
            ))
        })
        .collect();
    let patterns = g
        .patterns
        .iter()
        .enumerate()
        .map(|(pid, p)| {
            let route = &g.routes[p.route as usize];
            let c = groups.canon[pid] as usize;
            let pa = acc.pats[c].as_ref();
            let ac_share = pa
                .filter(|a| a.ac_known > 0.0)
                .map(|a| a.ac_tickets / a.ac_known);
            let cells = if c == pid {
                canon_cells[pid].clone()
            } else {
                canon_cells[c].as_ref().map(|cc| {
                    let (n, nc) = (p.stops.len(), g.patterns[c].stops.len());
                    let m = &groups.map[pid];
                    Cells {
                        rate: remap(&cc.rate, m, n - 1, nc - 1, nb, false),
                        tickets: remap(&cc.tickets, m, n - 1, nc - 1, nb, false),
                        buses: remap(&cc.buses, m, n - 1, nc - 1, nb, false),
                        board: remap(&cc.board, m, n, nc, nb, true),
                        alight: remap(&cc.alight, m, n, nc, nb, true),
                        flags: {
                            let f: Vec<f32> = cc.flags.iter().map(|&x| x as f32).collect();
                            remap(&f, m, n - 1, nc - 1, nb, false)
                                .into_iter()
                                .map(|x| x as u8)
                                .collect()
                        },
                    }
                })
            };
            SnapPattern {
                route: p.route,
                stops: p.stops.clone(),
                dist_m: p.dist_m.clone(),
                trips: p.trips.clone(),
                class: infer_class(cfg, route.name(), ac_share.map(|s| s as f64))
                    .name
                    .clone(),
                ac_share,
                tickets: pa.map(|a| a.tickets).unwrap_or(0.0),
                expansion_adj: 1.0,
                cells,
            }
        })
        .collect();
    let mut snapshot = Snapshot {
        meta: Meta {
            format_version: FORMAT_VERSION,
            generated_at: chrono::Utc::now().to_rfc3339(),
            date_from,
            date_to,
            n_days: nd,
            expansion,
            ticketed_pax_per_weekday: ticketed_per_weekday,
            feed_version: g.feed_version.clone(),
            tickets_total: tickets.len() as u64,
            tickets_assigned: assigned.len() as u64,
        },
        config: cfg.clone(),
        stops: g.stops.clone(),
        routes: g.routes.clone(),
        patterns,
        trips: g
            .trips
            .iter()
            .map(|t| SnapTrip {
                trip_id: t.trip_id.clone(),
                pattern: t.pattern,
                service_id: t.service_id.clone(),
                times: t.times.clone(),
            })
            .collect(),
        calendar: g.calendar.clone(),
    };
    let adj: Vec<f32> = (0..snapshot.patterns.len() as u32)
        .map(|p| {
            snapshot.capacity_adjustment(groups.canon[p as usize], cfg.capacity_check_quantile)
        })
        .collect();
    let n_adj = adj.iter().filter(|&&a| a < 1.0).count();
    for (p, a) in snapshot.patterns.iter_mut().zip(&adj) {
        p.expansion_adj = *a;
    }
    tracing::info!(routes_adjusted = n_adj, "capacity consistency");
    let derive = |arr: &Vec<Vec<f32>>| -> Vec<Vec<f32>> {
        (0..g.patterns.len())
            .map(|pid| {
                let c = groups.canon[pid] as usize;
                if c == pid {
                    arr[pid].clone()
                } else {
                    let (n, nc) = (g.patterns[pid].stops.len(), g.patterns[c].stops.len());
                    remap(&arr[c], &groups.map[pid], n - 1, nc - 1, nb, false)
                }
            })
            .collect()
    };
    let sup_sched = derive(&sup_sched);
    let sup_obs: Vec<Option<Vec<f32>>> = (0..g.patterns.len())
        .map(|pid| {
            let c = groups.canon[pid] as usize;
            sup_obs[c].as_ref().map(|o| {
                if c == pid {
                    o.clone()
                } else {
                    let (n, nc) = (g.patterns[pid].stops.len(), g.patterns[c].stops.len());
                    remap(o, &groups.map[pid], n - 1, nc - 1, nb, false)
                }
            })
        })
        .collect();
    let inv_capture: Vec<[f32; N_DAYTYPES]> = (0..g.patterns.len())
        .map(|pid| inv_capture[groups.canon[pid] as usize])
        .collect();
    let unmatched_factor: Vec<f32> = (0..g.patterns.len())
        .map(|pid| unmatched_factor[groups.canon[pid] as usize])
        .collect();
    Ok(BuildOutput {
        snapshot,
        report,
        assigned,
        recon,
        sup_sched,
        sup_obs,
        inv_capture,
        dropped_out_of_hours: acc.dropped_out_of_hours,
        cells_supply_raised: raised,
        groups: groups.report.clone(),
        fleet,
        unmatched_factor,
        canon: groups.canon.clone(),
    })
}
