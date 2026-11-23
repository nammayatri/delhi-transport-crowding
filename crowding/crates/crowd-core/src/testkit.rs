//! Small fixtures for tests in this and downstream crates: a two-route feed (34UP/34DOWN
//! from Anand Vihar ISBT to Connaught Place, six trips each way) and a snapshot built from
//! synthetic tickets that make the 08:00 trip crowded between Laxmi Nagar and Mandi House.

use crate::build::build;
use crate::gtfs::Gtfs;
use crate::snapshot::Snapshot;
use crate::tickets::{ist, Interner, RawTicket};
use crate::ModelConfig;

pub fn write_feed() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "crowd-core-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let w = |name: &str, body: &str| {
        std::fs::write(dir.join(name), body).unwrap();
    };
    w(
        "stops.txt",
        "stop_id,stop_name,stop_lat,stop_lon\n\
        A,Anand Vihar ISBT,28.6469,77.3159\n\
        B,Laxmi Nagar Crossing,28.6304,77.2773\n\
        C,ITO,28.6286,77.2410\n\
        D,Mandi House,28.6258,77.2342\n\
        E,Connaught Place (T),28.6315,77.2167\n",
    );
    w(
        "routes.txt",
        "route_id,route_short_name,route_long_name,agency_id\n\
        r1,,34UP,DTC\nr2,,34DOWN,DTC\n",
    );
    w(
        "calendar.txt",
        "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n\
        all,1,1,1,1,1,1,1,20260101,20261231\n",
    );
    let mut trips = String::from("route_id,service_id,trip_id\n");
    let mut st = String::from("trip_id,arrival_time,departure_time,stop_id,stop_sequence\n");
    for (k, h) in [7u32, 8, 9, 17, 18, 19].iter().enumerate() {
        for (dir_k, (route, stops)) in [
            ("r1", ["A", "B", "C", "D", "E"]),
            ("r2", ["E", "D", "C", "B", "A"]),
        ]
        .iter()
        .enumerate()
        {
            let tid = format!("{route}_{k}");
            trips.push_str(&format!("{route},all,{tid}\n"));
            for (i, s) in stops.iter().enumerate() {
                let t = format!("{:02}:{:02}:00", h, (i as u32) * 10 + dir_k as u32);
                st.push_str(&format!("{tid},{t},{t},{s},{}\n", i + 1));
            }
        }
    }
    w("trips.txt", &trips);
    w("stop_times.txt", &st);
    dir
}

pub fn fixture_snapshot() -> Snapshot {
    let g = Gtfs::load(&write_feed()).unwrap();
    let mut cfg = ModelConfig {
        capacity_check_quantile: 0.0,
        ..Default::default()
    };
    // 30 ticketed riders per day on one trip; each stands for 5 riders.
    cfg.ridership.fixed_expansion = 5.0;
    let mut names = Interner::default();
    let mut tickets = Vec::new();
    for day in 5..=9 {
        for i in 0..30 {
            let raw = RawTicket {
                passenger_count: Some(1.0),
                amount: Some(13.5),
                vehicle_number: Some(format!("DL1PC{day:04}")),
                ticket_type_id: Some(1),
                start_stop_name: "Laxmi Nagar Crossing".into(),
                end_stop_name: Some("Mandi House".into()),
                route: "34UP".into(),
                created_at: format!("2026-10-{day:02} 08:10:{i:02}+05:30"),
                is_ac: Some(true),
            };
            tickets.push(raw.parse(&mut names, ist()).unwrap());
        }
    }
    build(&g, &cfg, &tickets, &names, None).unwrap().snapshot
}
