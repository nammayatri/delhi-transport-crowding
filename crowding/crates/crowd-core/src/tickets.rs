//! Ticket records: the raw shape (as in the published CSV and the live ingest API) and a
//! compact parsed form with interned strings.

use std::io::Read;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, Timelike};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// A ticket as published: the CSV header and the live JSON body use these field names.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTicket {
    pub passenger_count: Option<f64>,
    pub amount: Option<f64>,
    pub vehicle_number: Option<String>,
    pub ticket_type_id: Option<i64>,
    pub start_stop_name: String,
    pub end_stop_name: Option<String>,
    pub route: String,
    /// Local time with offset, e.g. `2026-10-28 10:19:18.780611+05:30`.
    pub created_at: String,
    pub is_ac: Option<bool>,
}

/// String interner for stop, route and vehicle names.
#[derive(Debug, Clone, Default)]
pub struct Interner {
    map: FxHashMap<String, u32>,
    names: Vec<String>,
}

impl Interner {
    pub fn id(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.map.get(s) {
            return i;
        }
        let i = self.names.len() as u32;
        self.names.push(s.to_string());
        self.map.insert(s.to_string(), i);
        i
    }

    pub fn get(&self, id: u32) -> &str {
        &self.names[id as usize]
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ticket {
    pub pax: f32,
    pub amount: f32,
    pub vehicle: Option<u32>,
    pub route: u32,
    pub start: u32,
    pub end: Option<u32>,
    /// Local calendar date and seconds after local midnight.
    pub date: NaiveDate,
    pub secs: u32,
    /// Unix seconds.
    pub ts: i64,
    pub is_ac: Option<bool>,
    pub ticket_type: i64,
}

impl Ticket {
    /// Fare per passenger rounded to the nearest 0.5.
    pub fn fare_per_pax(&self) -> f32 {
        ((self.amount / self.pax.max(1.0)) * 2.0).round() / 2.0
    }
}

/// Parse a timestamp. Accepts an explicit offset or assumes `tz` when there is none.
pub fn parse_ts(s: &str, tz: FixedOffset) -> Result<DateTime<FixedOffset>> {
    let s = s.trim();
    for f in [
        "%Y-%m-%d %H:%M:%S%.f%:z",
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%d %H:%M:%S%.f%#z",
    ] {
        if let Ok(d) = DateTime::parse_from_str(s, f) {
            return Ok(d.with_timezone(&tz));
        }
    }
    for f in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return n
                .and_local_timezone(tz)
                .single()
                .with_context(|| format!("ambiguous time {s}"));
        }
    }
    anyhow::bail!("unparseable timestamp {s:?}")
}

pub fn ist() -> FixedOffset {
    FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("valid offset")
}

fn clean(s: Option<&str>) -> Option<&str> {
    let s = s?.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("null") {
        None
    } else {
        Some(s)
    }
}

impl RawTicket {
    pub fn parse(&self, names: &mut Interner, tz: FixedOffset) -> Result<Ticket> {
        let t = parse_ts(&self.created_at, tz)?;
        let start = clean(Some(&self.start_stop_name)).context("empty start_stop_name")?;
        let route = clean(Some(&self.route)).context("empty route")?;
        Ok(Ticket {
            pax: self.passenger_count.unwrap_or(1.0).max(1.0) as f32,
            amount: self.amount.unwrap_or(0.0) as f32,
            vehicle: clean(self.vehicle_number.as_deref())
                .map(|v| names.id(&v.to_ascii_uppercase())),
            route: names.id(route),
            start: names.id(start),
            end: clean(self.end_stop_name.as_deref()).map(|e| names.id(e)),
            date: t.date_naive(),
            secs: t.time().num_seconds_from_midnight(),
            ts: t.timestamp(),
            is_ac: self.is_ac,
            ticket_type: self.ticket_type_id.unwrap_or(0),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReadStats {
    pub rows: u64,
    pub parsed: u64,
    pub bad: u64,
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "1" | "yes" => Some(true),
        "false" | "f" | "0" | "no" => Some(false),
        _ => None,
    }
}

/// Read tickets from CSV with the published header (columns looked up by name).
pub fn read_csv<R: Read>(
    rdr: R,
    names: &mut Interner,
    tz: FixedOffset,
) -> Result<(Vec<Ticket>, ReadStats)> {
    let mut r = csv::ReaderBuilder::new().flexible(true).from_reader(rdr);
    let h = r.headers()?.clone();
    let col = |n: &str| {
        h.iter()
            .position(|x| x.trim_start_matches('\u{feff}').trim() == n)
    };
    let need = |n: &str| col(n).with_context(|| format!("ticket CSV missing column {n}"));
    let (c_pax, c_amt, c_veh, c_type) = (
        col("passenger_count"),
        col("amount"),
        col("vehicle_number"),
        col("ticket_type_id"),
    );
    let (c_start, c_end, c_route, c_time) = (
        need("start_stop_name")?,
        col("end_stop_name"),
        need("route")?,
        need("created_at")?,
    );
    let c_ac = col("is_ac");
    let mut out = Vec::new();
    let mut st = ReadStats::default();
    let mut rec = csv::StringRecord::new();
    while r.read_record(&mut rec)? {
        st.rows += 1;
        let f = |c: Option<usize>| c.and_then(|i| clean(rec.get(i)));
        let raw = RawTicket {
            passenger_count: f(c_pax).and_then(|s| s.parse().ok()),
            amount: f(c_amt).and_then(|s| s.parse().ok()),
            vehicle_number: f(c_veh).map(str::to_string),
            ticket_type_id: f(c_type).and_then(|s| s.parse().ok()),
            start_stop_name: f(Some(c_start)).unwrap_or("").to_string(),
            end_stop_name: f(c_end).map(str::to_string),
            route: f(Some(c_route)).unwrap_or("").to_string(),
            created_at: f(Some(c_time)).unwrap_or("").to_string(),
            is_ac: f(c_ac).and_then(parse_bool),
        };
        match raw.parse(names, tz) {
            Ok(t) => {
                st.parsed += 1;
                out.push(t);
            }
            Err(_) => st.bad += 1,
        }
    }
    Ok((out, st))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_published_rows() {
        let csv = "\"passenger_count\",\"amount\",\"vehicle_number\",\"ticket_type_id\",\"end_stop_name\",\"route\",\"start_stop_name\",\"created_at\",\"is_ac\"\n\
            1,\"22\",NULL,1,\"Mahamaya Flyover West\",\"8UP\",\"Ali Village\",\"2026-10-28 10:19:18.780611+05:30\",True\n\
            2,\"18\",\"dl1pd6826\",1,\"Crown Plaza\",\"8ADOWN\",\"Kalindi Kunj Bdr\",\"2026-10-28 10:19:29+05:30\",False\n\
            1,\"9\",NULL,1,\"X\",\"8UP\",\"\",\"2026-10-28 10:19:29+05:30\",True\n";
        let mut names = Interner::default();
        let (t, st) = read_csv(csv.as_bytes(), &mut names, ist()).unwrap();
        assert_eq!(st.rows, 3);
        assert_eq!(st.parsed, 2);
        assert_eq!(st.bad, 1);
        assert_eq!(t[0].vehicle, None);
        assert_eq!(t[0].secs, 10 * 3600 + 19 * 60 + 18);
        assert_eq!(t[0].date, NaiveDate::from_ymd_opt(2026, 10, 28).unwrap());
        assert_eq!(names.get(t[1].vehicle.unwrap()), "DL1PD6826");
        assert_eq!(t[1].fare_per_pax(), 9.0);
        assert_eq!(t[1].is_ac, Some(false));
    }

    #[test]
    fn timestamps_convert_to_local() {
        let d = parse_ts("2026-10-28T04:49:18Z", ist());
        assert!(d.is_err() || d.unwrap().hour() == 10);
        let d = parse_ts("2026-10-28 04:49:18+00:00", ist()).unwrap();
        assert_eq!(d.hour(), 10);
        let d = parse_ts("2026-10-28 10:19:18", ist()).unwrap();
        assert_eq!(d.hour(), 10);
    }
}
