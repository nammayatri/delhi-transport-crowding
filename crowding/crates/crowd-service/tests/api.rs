use std::io::Read;

use actix_web::{test, web};
use crowd_core::testkit::fixture_snapshot;
use crowd_service::config::ServiceConfig;
use crowd_service::state::AppState;
use serde_json::{json, Value};

fn state() -> web::Data<AppState> {
    let mut cfg = ServiceConfig::default();
    cfg.live.clock = "replay".into();
    let st = AppState::new(cfg);
    st.install(fixture_snapshot());
    web::Data::new(st)
}

macro_rules! get_json {
    ($app:expr, $uri:expr) => {{
        let req = test::TestRequest::get().uri($uri).to_request();
        let resp = test::call_service(&$app, req).await;
        assert!(resp.status().is_success(), "{} -> {}", $uri, resp.status());
        let v: Value = test::read_body_json(resp).await;
        v
    }};
}

#[actix_web::test]
async fn health_meta_routes_and_profile() {
    let app = test::init_service(crowd_service::app(state())).await;
    let v = get_json!(app, "/health");
    assert_eq!(v["status"], "ok");
    let v = get_json!(app, "/ready");
    assert_eq!(v["status"], "ready");
    let v = get_json!(app, "/v1/meta");
    assert_eq!(v["routes_with_data"], 1);
    let v = get_json!(app, "/v1/routes?q=34");
    assert_eq!(v.as_array().unwrap().len(), 2);
    let v = get_json!(app, "/v1/routes/r1/profile?daytype=weekday");
    let p = &v["patterns"][0];
    assert_eq!(p["stops"].as_array().unwrap().len(), 5);
    // 08:00 bin is index 8: segment B-C (1) is crowded, segment A-B (0) carries nobody ticketed.
    assert_eq!(p["level"][1][8], 2);
    let v = get_json!(app, "/v1/stops?q=laxmi");
    assert_eq!(v[0]["name"], "Laxmi Nagar Crossing");
    let req = test::TestRequest::get()
        .uri("/v1/routes/nope/profile")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["code"], "NOT_FOUND");
}

#[actix_web::test]
async fn map_annotate_and_operator() {
    let app = test::init_service(crowd_service::app(state())).await;
    let v = get_json!(app, "/v1/map/segments?date=2026-10-06&time=08:05");
    let f = v["features"].as_array().unwrap();
    assert!(f.iter().any(|x| x["properties"]["level"] == "crowded"));

    let req = test::TestRequest::post()
        .uri("/v1/legs:annotate")
        .set_json(json!({"legs": [
            {"trip_id": "r1_1", "from_stop_id": "A", "to_stop_id": "E", "start_time": "2026-10-06T08:00:00+05:30"},
            {"route_id": "r1", "from_stop_id": "1:B", "to_stop_id": "1:D", "start_time": "2026-10-06T09:10:00+05:30"},
            {"trip_id": "r1_1", "from_stop_id": "E", "to_stop_id": "A"}
        ]}))
        .to_request();
    let v: Value = test::call_and_read_body_json(&app, req).await;
    let c = &v["legs"][0]["crowding"];
    assert_eq!(c["worst_level"], "crowded");
    assert_eq!(c["level_at_boarding"], "seats");
    assert_eq!(c["worst_from"], "Laxmi Nagar Crossing");
    assert_eq!(c["worst_to"], "Mandi House");
    assert!(c["summary"]
        .as_str()
        .unwrap()
        .starts_with("Crowded Laxmi Nagar Crossing → Mandi House"));
    assert_eq!(v["legs"][1]["trip_id"], "r1_2");
    assert!(v["legs"][2]["error"].is_string());

    let v = get_json!(app, "/v1/operator/hotspots?from=07:00&to=10:00");
    let h = &v["hotspots"][0];
    assert_eq!(h["route"], "34UP");
    assert!(h["extra_buses_at_peak"].as_f64().unwrap() > 0.0);
    let v = get_json!(app, "/v1/operator/routes?from=07:00&to=10:00");
    assert_eq!(v["routes"][0]["route"], "34UP");
}

#[actix_web::test]
async fn live_tickets_shift_the_clock_and_feed_the_nowcast() {
    let app = test::init_service(crowd_service::app(state())).await;
    let t = |at: &str| {
        json!({
            "passenger_count": 1, "amount": 13.5, "vehicle_number": "DL1PD0001", "ticket_type_id": 1,
            "start_stop_name": "Laxmi Nagar Xing", "end_stop_name": "Mandi House", "route": "34UP",
            "created_at": at, "is_ac": true
        })
    };
    let body: Vec<Value> = (0..10).map(|i| t(&format!("2026-10-07 08:{:02}:00+05:30", i))).chain([json!({
        "passenger_count": 1, "amount": 9, "vehicle_number": null, "ticket_type_id": 1,
        "start_stop_name": "X", "end_stop_name": "Y", "route": "999UP", "created_at": "2026-10-07 08:00:00+05:30", "is_ac": true
    })]).collect();
    let req = test::TestRequest::post()
        .uri("/v1/tickets")
        .set_json(body)
        .to_request();
    let v: Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(v["accepted"], 10);
    assert_eq!(v["rejected"], 1);
    let v = get_json!(app, "/v1/live/status");
    assert_eq!(v["tickets"], 10);
    assert!(v["now"].as_str().unwrap().starts_with("2026-10-07T08:09"));
    let v = get_json!(app, "/v1/live/routes/r1");
    assert!(v["live_factor"].as_f64().unwrap() > 0.0);
    let m = test::call_service(&app, test::TestRequest::get().uri("/metrics").to_request()).await;
    let text = String::from_utf8(test::read_body(m).await.to_vec()).unwrap();
    assert!(text.contains("crowd_live_tickets_ingested_total 10"));
}

#[actix_web::test]
async fn otp_index_and_gtfs_rt() {
    let app = test::init_service(crowd_service::app(state())).await;
    let req = test::TestRequest::get()
        .uri("/v1/otp/crowding-index?date=2026-10-06")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    // Buses are counted from the tickets only: the fixture's tickets are all on the
    // 08:00 bus, so only that trip has a crowding estimate.
    assert_eq!(resp.headers().get("X-Crowding-Trips").unwrap(), "1");
    let body = test::read_body(resp).await;
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&body[..])
        .read_to_end(&mut raw)
        .unwrap();
    assert_eq!(&raw[..4], b"CRWD");
    assert_eq!(u16::from_be_bytes([raw[4], raw[5]]), 1);
    assert_eq!(
        i32::from_be_bytes([raw[8], raw[9], raw[10], raw[11]]),
        20261006
    );

    let v = get_json!(
        app,
        "/v1/otp/crowding-index?date=2026-10-06&format=json&limit=10"
    );
    let trip = v["sample"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["trip_id"] == "r1_1")
        .unwrap();
    let cum: Vec<u64> = trip["cum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect();
    assert_eq!(cum[0], 0);
    assert!(
        cum[3] > cum[1],
        "crowding accumulates on the crowded stretch: {cum:?}"
    );
    assert!(cum.windows(2).all(|w| w[1] >= w[0]));

    let req = test::TestRequest::get()
        .uri("/gtfs-rt/trip-updates.pb?at=2026-10-06T08:00:00%2B05:30")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    let v = get_json!(
        app,
        "/gtfs-rt/trip-updates.pb?at=2026-10-06T08:00:00%2B05:30&format=json"
    );
    assert!(v["entities"].as_u64().unwrap() >= 2);
}

#[actix_web::test]
async fn journeys_label_the_fastest_option_and_show_crowding() {
    let app = test::init_service(crowd_service::app(state())).await;
    let req = test::TestRequest::post()
        .uri("/v1/journeys")
        .set_json(json!({
            "from": {"lat": 28.6469, "lon": 77.3159}, "to": {"stop_id": "E"},
            "depart_at": "2026-10-06T07:50:00+05:30", "comfort": 0.5, "engine": "builtin"
        }))
        .to_request();
    let v: Value = test::call_and_read_body_json(&app, req).await;
    let opts = v["options"].as_array().unwrap();
    assert!(!opts.is_empty(), "{v}");
    let fastest = opts
        .iter()
        .find(|o| {
            o["labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l == "Fastest")
        })
        .unwrap();
    assert_eq!(fastest["depart"].as_str().unwrap()[11..16], *"08:00");
    let bus = fastest["legs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["mode"] == "bus")
        .unwrap();
    assert_eq!(bus["route"], "34UP");
    assert_eq!(bus["crowding"]["worst_level"], "crowded");
    assert!(opts.iter().any(|o| o["recommended"] == true));
}

#[actix_web::test]
async fn neutral_live_factor_without_live_data() {
    let app = test::init_service(crowd_service::app(state())).await;
    let v = get_json!(app, "/v1/live/routes/r1");
    assert_eq!(v["live_factor"], 1.0);
}
