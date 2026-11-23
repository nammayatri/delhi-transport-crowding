//! Batch pipeline: ticket records + static GTFS -> crowding profiles.
//!
//! Outputs (in `--out`):
//! - `snapshot.msgpack.zst`: everything the crowding service loads.
//! - `segment_profiles.parquet`: load and crowding level by route, stop-to-stop segment,
//!   day type and 30-minute bin (mid scenario, with low/high loads).
//! - `stop_activity.parquet`: boardings and alightings by stop, day type and bin.
//! - `assigned_tickets.parquet`: every ticket matched to GTFS, for analysis.
//! - `recon_trips.parquet`: bus trips rebuilt from tickets with a vehicle number.
//! - `route_summary.csv`, `match_report.json`, `meta.json`.

mod table;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use crowd_core::assign::Method;
use crowd_core::daytype::{DayCalendar, DayType, N_DAYTYPES};
use crowd_core::gtfs::Gtfs;
use crowd_core::levels::{Level, ScenarioPick};
use crowd_core::snapshot::Snapshot;
use crowd_core::tickets::{ist, read_csv, Interner};
use crowd_core::ModelConfig;
use serde_json::{json, Value};
use table::{Kind, Table};

#[derive(Parser, Debug)]
#[command(about = "Turn ticket records and a static GTFS feed into crowding profiles")]
struct Args {
    /// Model assumptions (Dhall or JSON).
    #[arg(long, env = "CROWD_MODEL_CONFIG", default_value = "config/model.dhall")]
    config: PathBuf,
    /// Extracted static GTFS directory.
    #[arg(long, env = "CROWD_GTFS_DIR")]
    gtfs: PathBuf,
    /// Ticket CSV files.
    #[arg(
        long = "tickets",
        env = "CROWD_TICKETS",
        value_delimiter = ',',
        required = true
    )]
    tickets: Vec<PathBuf>,
    #[arg(long, default_value = "out")]
    out: PathBuf,
    /// Only use tickets on or after this date (YYYY-MM-DD).
    #[arg(long)]
    date_from: Option<NaiveDate>,
    /// Only use tickets on or before this date.
    #[arg(long)]
    date_to: Option<NaiveDate>,
    /// Override a config value, e.g. `--set alight_spread=0.5`. Repeatable.
    #[arg(long = "set")]
    overrides: Vec<String>,
    /// Skip the Parquet tables (snapshot and JSON only).
    #[arg(long)]
    no_tables: bool,
}

fn load_config(path: &Path, overrides: &[String]) -> Result<ModelConfig> {
    let cfg: ModelConfig = if path.extension().is_some_and(|e| e == "dhall") {
        serde_dhall::from_file(path)
            .parse()
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?
    } else {
        serde_json::from_slice(&std::fs::read(path)?)?
    };
    if overrides.is_empty() {
        return Ok(cfg);
    }
    let mut v = serde_json::to_value(&cfg)?;
    for o in overrides {
        let (k, val) = o
            .split_once('=')
            .with_context(|| format!("bad --set {o}"))?;
        let parsed: Value = serde_json::from_str(val).unwrap_or(Value::String(val.to_string()));
        let mut cur = &mut v;
        for part in k.split('.') {
            cur = cur
                .get_mut(part)
                .with_context(|| format!("unknown config key {k}"))?;
        }
        *cur = parsed;
    }
    Ok(serde_json::from_value(v)?)
}

fn hhmm(min: u32) -> String {
    format!("{:02}:{:02}", min / 60, min % 60)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let t0 = Instant::now();
    let cfg = load_config(&args.config, &args.overrides)?;
    cfg.validate()?;
    std::fs::create_dir_all(&args.out)?;
    let cal = DayCalendar::new(&cfg.holidays)?;

    let g = Gtfs::load(&args.gtfs)?;

    let mut names = Interner::default();
    let mut tickets = Vec::new();
    for p in &args.tickets {
        let f = std::fs::File::open(p).with_context(|| format!("open {}", p.display()))?;
        let (t, st) = read_csv(
            std::io::BufReader::with_capacity(1 << 20, f),
            &mut names,
            ist(),
        )?;
        tracing::info!(file = %p.display(), rows = st.rows, parsed = st.parsed, bad = st.bad, "read tickets");
        tickets.extend(t);
    }
    tickets.retain(|t| {
        args.date_from.is_none_or(|d| t.date >= d) && args.date_to.is_none_or(|d| t.date <= d)
    });
    anyhow::ensure!(!tickets.is_empty(), "no tickets in the date range");
    let date_from = tickets.iter().map(|t| t.date).min().unwrap();
    let date_to = tickets.iter().map(|t| t.date).max().unwrap();
    tracing::info!(tickets = tickets.len(), %date_from, %date_to, "tickets in range");

    let out = crowd_core::build::build(&g, &cfg, &tickets, &names)?;
    let snap = out.snapshot;
    snap.save(&args.out.join("snapshot.msgpack.zst"))?;
    tracing::info!(elapsed_s = t0.elapsed().as_secs_f32(), "wrote snapshot");
    let (report, assigned, recon) = (out.report, out.assigned, out.recon);
    let (sup_sched, sup_obs, inv_capture) = (out.sup_sched, out.sup_obs, out.inv_capture);
    let unmatched_factor = out.unmatched_factor;
    let canon = out.canon;

    std::fs::write(
        args.out.join("match_report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    let meta = json!({
        "meta": snap.meta,
        "config": cfg,
        "dropped_out_of_service_hours_pax": out.dropped_out_of_hours,
        "cells_supply_raised_by_observed": out.cells_supply_raised,
        "route_groups": out.groups,
        "fleet_calibration": out.fleet,
        "patterns_with_data": snap.patterns.iter().filter(|p| p.cells.is_some()).count(),
        "patterns_total": snap.patterns.len(),
    });
    std::fs::write(
        args.out.join("meta.json"),
        serde_json::to_vec_pretty(&meta)?,
    )?;

    if !args.no_tables {
        let sup_tables = SupplyTables {
            sched: &sup_sched,
            obs: &sup_obs,
            inv_capture: &inv_capture,
            unmatched_factor: &unmatched_factor,
            canon: &canon,
        };
        write_tables(
            &args.out,
            &g,
            &snap,
            &tickets,
            &assigned,
            &names,
            &cal,
            &recon,
            &sup_tables,
        )?;
    }
    tracing::info!(elapsed_s = t0.elapsed().as_secs_f32(), "done");
    Ok(())
}

struct SupplyTables<'a> {
    sched: &'a [Vec<f32>],
    obs: &'a [Option<Vec<f32>>],
    inv_capture: &'a [[f32; N_DAYTYPES]],
    unmatched_factor: &'a [f32],
    canon: &'a [u32],
}

#[allow(clippy::too_many_arguments)]
fn write_tables(
    out: &Path,
    g: &Gtfs,
    snap: &Snapshot,
    tickets: &[crowd_core::tickets::Ticket],
    assigned: &[crowd_core::assign::Assigned],
    names: &Interner,
    cal: &DayCalendar,
    recon: &[crowd_core::trips::ReconTrip],
    sup: &SupplyTables,
) -> Result<()> {
    let cfg = &snap.config;
    let grid = snap.grid();

    // Segment profiles.
    let mut t = Table::create(
        &out.join("segment_profiles.parquet"),
        &[
            ("route_id", Kind::Str),
            ("route_name", Kind::Str),
            ("pattern", Kind::U32),
            ("seq", Kind::U32),
            ("from_stop_id", Kind::Str),
            ("from_stop_name", Kind::Str),
            ("to_stop_id", Kind::Str),
            ("to_stop_name", Kind::Str),
            ("km_from_start", Kind::F32),
            ("daytype", Kind::Str),
            ("bin_start", Kind::Str),
            ("bin_minute", Kind::U32),
            ("buses_per_day", Kind::F32),
            ("tickets", Kind::F32),
            ("ticketed_rate", Kind::F32),
            ("load", Kind::F32),
            ("load_low", Kind::F32),
            ("load_high", Kind::F32),
            ("p_seats", Kind::F32),
            ("p_standing", Kind::F32),
            ("p_crowded", Kind::F32),
            ("level", Kind::Str),
            ("usually", Kind::Bool),
            ("occupancy_status", Kind::Str),
            ("confidence", Kind::Str),
            ("bus_class", Kind::Str),
            ("seats", Kind::F32),
            ("comfortable", Kind::F32),
            ("sched_buses_per_day", Kind::F32),
            ("observed_buses_per_day", Kind::F32),
            ("over_capacity", Kind::Bool),
        ],
    )?;
    // Route summaries accumulated on the way.
    #[derive(Default)]
    struct RouteAgg {
        peak: Option<(f64, usize, usize)>,
        crowded_bus_segments: f64,
        bus_segments: f64,
        high_conf_bus_segments: f64,
    }
    let mut route_aggs: Vec<RouteAgg> = (0..snap.patterns.len())
        .map(|_| RouteAgg::default())
        .collect();
    for (pid, p) in snap.patterns.iter().enumerate() {
        let Some(c) = &p.cells else { continue };
        if sup.canon[pid] as usize != pid {
            continue; // a second operator's copy of a route: counted on the canonical pattern
        }
        let route = &snap.routes[p.route as usize];
        let class = snap.class_of(pid as u32);
        for seg in 0..p.stops.len() - 1 {
            let (fs, ts) = (
                &snap.stops[p.stops[seg] as usize],
                &snap.stops[p.stops[seg + 1] as usize],
            );
            for dt in DayType::ALL {
                for bin in 0..grid.nb {
                    let Some(v) = snap.cell(pid as u32, seg, dt, bin, ScenarioPick::Mid, 1.0)
                    else {
                        continue;
                    };
                    let i = grid.idx(seg, dt.idx(), bin);
                    t.s(0, &route.route_id)
                        .s(1, route.name())
                        .u(2, pid as u32)
                        .u(3, seg as u32)
                        .s(4, &fs.stop_id)
                        .s(5, &fs.name)
                        .s(6, &ts.stop_id)
                        .s(7, &ts.name)
                        .f(8, p.dist_m[seg] / 1000.0)
                        .s(9, dt.as_str())
                        .s(10, hhmm(cfg.bin_start_minute(bin)))
                        .u(11, cfg.bin_start_minute(bin))
                        .f(12, v.buses_per_day as f32)
                        .f(13, v.tickets as f32)
                        .f(14, c.rate[i])
                        .f(15, v.load as f32)
                        .f(16, v.load_low as f32)
                        .f(17, v.load_high as f32)
                        .f(18, v.p_seats as f32)
                        .f(19, v.p_standing as f32)
                        .f(20, v.p_crowded as f32)
                        .s(21, v.level.as_str())
                        .b(22, v.usually)
                        .s(23, v.status.as_str())
                        .s(24, v.confidence.as_str())
                        .s(25, &class.name)
                        .f(26, class.seats as f32)
                        .f(27, class.comfortable as f32)
                        .f(
                            28,
                            sup.sched[pid][i] / snap.meta.n_days[dt.idx()].max(1) as f32,
                        )
                        .f(
                            29,
                            sup.obs[pid].as_ref().map(|o| o[i]).unwrap_or(0.0)
                                / snap.meta.n_days[dt.idx()].max(1) as f32,
                        )
                        .b(30, v.over_capacity);
                    t.end_row()?;
                    if dt == DayType::Weekday {
                        let ra = &mut route_aggs[pid];
                        if ra.peak.is_none_or(|pk| v.load > pk.0) {
                            ra.peak = Some((v.load, seg, bin));
                        }
                        ra.bus_segments += v.buses_per_day;
                        if v.level == Level::Crowded {
                            ra.crowded_bus_segments += v.buses_per_day;
                        }
                        if v.confidence == crowd_core::levels::Confidence::High {
                            ra.high_conf_bus_segments += v.buses_per_day;
                        }
                    }
                }
            }
        }
    }
    let n = t.finish()?;
    tracing::info!(rows = n, "wrote segment_profiles.parquet");

    // Stop activity (expanded to all riders, mid scenario).
    let e = snap.meta.expansion;
    let mut t = Table::create(
        &out.join("stop_activity.parquet"),
        &[
            ("route_id", Kind::Str),
            ("route_name", Kind::Str),
            ("pattern", Kind::U32),
            ("seq", Kind::U32),
            ("stop_id", Kind::Str),
            ("stop_name", Kind::Str),
            ("lat", Kind::F64),
            ("lon", Kind::F64),
            ("daytype", Kind::Str),
            ("bin_start", Kind::Str),
            ("bin_minute", Kind::U32),
            ("ticketed_boardings_per_day", Kind::F32),
            ("ticketed_alightings_per_day", Kind::F32),
            ("boardings_per_day", Kind::F32),
            ("alightings_per_day", Kind::F32),
        ],
    )?;
    for (pid, p) in snap.patterns.iter().enumerate() {
        let Some(c) = &p.cells else { continue };
        if sup.canon[pid] as usize != pid {
            continue;
        }
        let route = &snap.routes[p.route as usize];
        for (pos, &s) in p.stops.iter().enumerate() {
            let st = &snap.stops[s as usize];
            for dt in DayType::ALL {
                for bin in 0..grid.nb {
                    let i = grid.idx(pos, dt.idx(), bin);
                    let (b, a) = (c.board[i], c.alight[i]);
                    if b <= 0.0 && a <= 0.0 {
                        continue;
                    }
                    t.s(0, &route.route_id)
                        .s(1, route.name())
                        .u(2, pid as u32)
                        .u(3, pos as u32)
                        .s(4, &st.stop_id)
                        .s(5, &st.name)
                        .d(6, st.lat)
                        .d(7, st.lon)
                        .s(8, dt.as_str())
                        .s(9, hhmm(cfg.bin_start_minute(bin)))
                        .u(10, cfg.bin_start_minute(bin))
                        .f(11, b)
                        .f(12, a)
                        .f(13, b * e.mid as f32)
                        .f(14, a * e.mid as f32);
                    t.end_row()?;
                }
            }
        }
    }
    let n = t.finish()?;
    tracing::info!(rows = n, "wrote stop_activity.parquet");

    // Assigned tickets.
    let mut t = Table::create(
        &out.join("assigned_tickets.parquet"),
        &[
            ("ticket", Kind::U32),
            ("date", Kind::Str),
            ("daytype", Kind::Str),
            ("secs", Kind::U32),
            ("ticket_route", Kind::Str),
            ("route_id", Kind::Str),
            ("route_name", Kind::Str),
            ("pattern", Kind::U32),
            ("board_seq", Kind::U32),
            ("board_stop_id", Kind::Str),
            ("alight_seq", Kind::I32),
            ("alight_mean_seq", Kind::F32),
            ("km", Kind::F32),
            ("method", Kind::Str),
            ("score", Kind::F32),
            ("geo", Kind::Bool),
            ("pax", Kind::F32),
            ("fare_per_pax", Kind::F32),
            ("is_ac", Kind::Bool),
            ("vehicle", Kind::Str),
            ("trip_id", Kind::Str),
            ("sched_secs_at_board", Kind::U32),
        ],
    )?;
    for a in assigned {
        let tk = &tickets[a.ticket as usize];
        let p = &g.patterns[a.pattern as usize];
        let route = &g.routes[p.route as usize];
        let mean_al: f32 = a.alight.iter().map(|(x, w)| *x as f32 * w).sum();
        let km: f32 = a
            .alight
            .iter()
            .map(|(x, w)| (p.dist_m[*x as usize] - p.dist_m[a.board as usize]) * w)
            .sum::<f32>()
            / 1000.0;
        let trip = &g.trips[a.trip as usize];
        t.u(0, a.ticket)
            .s(1, tk.date.to_string())
            .s(2, cal.day_type(tk.date).as_str())
            .u(3, tk.secs)
            .s(4, names.get(tk.route))
            .s(5, &route.route_id)
            .s(6, route.name())
            .u(7, a.pattern)
            .u(8, a.board as u32)
            .s(9, &g.stops[p.stops[a.board as usize] as usize].stop_id)
            .n(10, a.alight_exact().map(|x| x as i32).unwrap_or(-1))
            .f(11, mean_al)
            .f(12, km)
            .s(
                13,
                match a.method {
                    Method::Exact => "exact",
                    Method::DirectionFixed => "direction_fixed",
                    Method::Imputed => "imputed",
                },
            )
            .f(14, a.score)
            .b(15, a.geo)
            .f(16, a.pax)
            .f(17, tk.fare_per_pax())
            .b(18, tk.is_ac.unwrap_or(true))
            .s(19, tk.vehicle.map(|v| names.get(v)).unwrap_or(""))
            .s(20, &trip.trip_id)
            .u(21, trip.times[a.board as usize]);
        t.end_row()?;
    }
    let n = t.finish()?;
    tracing::info!(rows = n, "wrote assigned_tickets.parquet");

    // Reconstructed vehicle trips.
    let mut t = Table::create(
        &out.join("recon_trips.parquet"),
        &[
            ("vehicle", Kind::Str),
            ("date", Kind::Str),
            ("route_id", Kind::Str),
            ("route_name", Kind::Str),
            ("first_secs", Kind::U32),
            ("last_secs", Kind::U32),
            ("tickets", Kind::U32),
            ("pax", Kind::F32),
            ("max_onboard_ticketed", Kind::F32),
            ("trip_id", Kind::Str),
        ],
    )?;
    for r in recon {
        t.s(0, &r.vehicle)
            .s(1, r.date.to_string())
            .s(2, &r.route_id)
            .s(3, &r.route_name)
            .u(4, r.first_secs)
            .u(5, r.last_secs)
            .u(6, r.tickets)
            .f(7, r.pax)
            .f(8, r.max_onboard)
            .s(9, &r.trip_id);
        t.end_row()?;
    }
    let n = t.finish()?;
    tracing::info!(rows = n, "wrote recon_trips.parquet");

    // Route summary.
    let mut w = csv::Writer::from_path(out.join("route_summary.csv"))?;
    w.write_record([
        "route_id",
        "route_name",
        "agency",
        "bus_class",
        "ac_share",
        "ticketed_riders",
        "stops",
        "length_km",
        "weekday_trips",
        "weekday_peak_load",
        "weekday_peak_level",
        "weekday_peak_from",
        "weekday_peak_to",
        "weekday_peak_time",
        "weekday_share_crowded",
        "weekday_share_high_confidence",
        "weekday_inverse_capture",
        "expansion_adjustment",
        "unmatched_correction",
    ])?;
    let wd_date = snap.meta.date_from;
    for (pid, p) in snap.patterns.iter().enumerate() {
        if p.cells.is_none() || sup.canon[pid] as usize != pid {
            continue;
        }
        let route = &snap.routes[p.route as usize];
        let ra = &route_aggs[pid];
        let weekday_trips = p
            .trips
            .iter()
            .filter(|&&ti| {
                let sid = &snap.trips[ti as usize].service_id;
                (0..7).any(|k| {
                    let d = wd_date + chrono::Days::new(k);
                    cal.day_type(d) == DayType::Weekday && snap.calendar.active(sid, d)
                })
            })
            .count();
        let (peak_load, peak_level, pf, pt, ptime) = match ra.peak {
            Some((l, seg, bin)) => {
                let v = snap.cell(
                    pid as u32,
                    seg,
                    DayType::Weekday,
                    bin,
                    ScenarioPick::Mid,
                    1.0,
                );
                (
                    format!("{l:.1}"),
                    v.map(|v| v.level.as_str()).unwrap_or("").to_string(),
                    snap.stops[p.stops[seg] as usize].name.clone(),
                    snap.stops[p.stops[seg + 1] as usize].name.clone(),
                    hhmm(cfg.bin_start_minute(bin)),
                )
            }
            None => Default::default(),
        };
        let share = |x: f64| {
            if ra.bus_segments > 0.0 {
                format!("{:.3}", x / ra.bus_segments)
            } else {
                String::new()
            }
        };
        w.write_record([
            route.route_id.as_str(),
            route.name(),
            route.agency_id.as_str(),
            p.class.as_str(),
            &p.ac_share.map(|s| format!("{s:.2}")).unwrap_or_default(),
            &format!("{:.0}", p.tickets),
            &p.stops.len().to_string(),
            &format!("{:.1}", p.dist_m.last().copied().unwrap_or(0.0) / 1000.0),
            &weekday_trips.to_string(),
            &peak_load,
            &peak_level,
            &pf,
            &pt,
            &ptime,
            &share(ra.crowded_bus_segments),
            &share(ra.high_conf_bus_segments),
            &format!("{:.2}", sup.inv_capture[pid][0]),
            &format!("{:.3}", p.expansion_adj),
            &format!("{:.3}", sup.unmatched_factor[pid]),
        ])?;
    }
    w.flush()?;
    Ok(())
}
