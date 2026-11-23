//! Rebuild individual bus trips from tickets that carry a vehicle number.
//!
//! Tickets of one vehicle on one day, ordered by time, are split into trips when the route
//! changes, when the gap between tickets is long, or when boardings jump back towards the
//! start of the route. For each trip we keep the running count of ticketed riders. These
//! trips are too sparse to estimate loads on their own (see [`crate::profile`]) but they
//! show how many tickets one bus trip carries, they link tickets to GPS traces by vehicle,
//! and they feed the live per-vehicle view.

use chrono::NaiveDate;
use serde::Serialize;

use crate::assign::Assigned;
use crate::gtfs::Gtfs;
use crate::tickets::{Interner, Ticket};

#[derive(Debug, Clone, Serialize)]
pub struct ReconTrip {
    pub vehicle: String,
    pub date: NaiveDate,
    pub route_id: String,
    pub route_name: String,
    pub pattern: u32,
    pub first_secs: u32,
    pub last_secs: u32,
    pub tickets: u32,
    pub pax: f32,
    /// Highest running count of ticketed riders on board.
    pub max_onboard: f32,
    /// Scheduled trip nearest to the first ticket.
    pub trip_id: String,
    /// First ticket of the trip: boarding position, time and the scheduled trip whose
    /// running times are used to place the bus along the pattern.
    pub anchor_board: u16,
    pub anchor_secs: u32,
    pub anchor_trip: u32,
}

pub fn reconstruct(
    g: &Gtfs,
    tickets: &[Ticket],
    assigned: &[Assigned],
    names: &Interner,
    gap_minutes: u32,
) -> Vec<ReconTrip> {
    let mut rows: Vec<(u32, NaiveDate, u32, usize)> = assigned
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let t = &tickets[a.ticket as usize];
            t.vehicle.map(|v| (v, t.date, t.secs, i))
        })
        .collect();
    rows.sort_unstable();
    let gap = gap_minutes * 60;
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let (veh, date) = (rows[i].0, rows[i].1);
        let mut j = i;
        while j < rows.len() && rows[j].0 == veh && rows[j].1 == date {
            j += 1;
        }
        let day = &rows[i..j];
        let mut start = 0;
        for k in 1..=day.len() {
            let split = k == day.len() || {
                let (prev, cur) = (&assigned[day[k - 1].3], &assigned[day[k].3]);
                // Same route number run by two operators counts as one route.
                let rn = |p: u32| g.routes[g.patterns[p as usize].route as usize].name();
                let route_change = rn(prev.pattern) != rn(cur.pattern);
                let long_gap = day[k].2.saturating_sub(day[k - 1].2) > gap;
                let restart = cur.board + 3 < prev.board;
                route_change || long_gap || restart
            };
            if split {
                out.push(summarise(g, tickets, assigned, names, &day[start..k]));
                start = k;
            }
        }
        i = j;
    }
    out
}

fn summarise(
    g: &Gtfs,
    tickets: &[Ticket],
    assigned: &[Assigned],
    names: &Interner,
    rows: &[(u32, NaiveDate, u32, usize)],
) -> ReconTrip {
    let first = &assigned[rows[0].3];
    let p = &g.patterns[first.pattern as usize];
    let route = &g.routes[p.route as usize];
    // Running count along stop positions.
    let mut delta = vec![0.0f32; p.stops.len() + 1];
    let mut pax = 0.0;
    for r in rows {
        let a = &assigned[r.3];
        if a.pattern != first.pattern {
            continue;
        }
        delta[a.board as usize] += a.pax;
        for &(al, w) in &a.alight {
            delta[al as usize] -= a.pax * w;
        }
        pax += a.pax;
    }
    let mut run = 0.0f32;
    let mut max_onboard = 0.0f32;
    for d in &delta {
        run += d;
        max_onboard = max_onboard.max(run);
    }
    let t0 = &tickets[first.ticket as usize];
    ReconTrip {
        vehicle: names.get(t0.vehicle.unwrap_or(0)).to_string(),
        date: t0.date,
        route_id: route.route_id.clone(),
        route_name: route.name().to_string(),
        pattern: first.pattern,
        first_secs: rows[0].2,
        last_secs: rows[rows.len() - 1].2,
        tickets: rows.len() as u32,
        pax,
        max_onboard,
        trip_id: g.trips[first.trip as usize].trip_id.clone(),
        anchor_board: first.board,
        anchor_secs: rows[0].2,
        anchor_trip: first.trip,
    }
}
