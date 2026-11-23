//! Crowding service library: configuration, state and the actix app factory.

pub mod api;
pub mod config;
pub mod error;
pub mod gtfsrt;
pub mod journeys;
pub mod kafka;
pub mod live_rt;
pub mod metrics;
pub mod otp;
pub mod otp_index;
pub mod state;

use std::time::Instant;

use actix_web::dev::{ServiceFactory, ServiceRequest, ServiceResponse};
use actix_web::{body::MessageBody, web, App, Error};
use futures_util::FutureExt;

use crate::state::AppState;

/// Register all routes on a service config (used by the binary and the tests).
pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(api::health)
        .service(api::ready)
        .service(api::metrics)
        .service(api::meta)
        .service(api::routes)
        .service(api::route_profile)
        .service(api::search_stops)
        .service(api::map_segments)
        .service(api::annotate)
        .service(api::hotspots)
        .service(api::operator_routes)
        .service(api::post_tickets)
        .service(api::live_status)
        .service(api::live_route)
        .service(otp_index::crowding_index)
        .service(gtfsrt::trip_updates)
        .service(live_rt::live_vehicles)
        .service(live_rt::vehicle_positions)
        .service(live_rt::post_vehicle_positions)
        .service(journeys::journeys);
}

/// Build the app with metrics middleware.
pub fn app(
    state: web::Data<AppState>,
) -> App<
    impl ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse<impl MessageBody>,
        Error = Error,
        InitError = (),
    >,
> {
    let st = state.clone();
    App::new()
        .app_data(state)
        .app_data(web::JsonConfig::default().limit(32 << 20))
        .wrap_fn(move |req, srv| {
            let st = st.clone();
            let t0 = Instant::now();
            actix_web::dev::Service::call(srv, req).map(move |res| {
                if let Ok(r) = &res {
                    let path = r
                        .request()
                        .match_pattern()
                        .unwrap_or_else(|| "unmatched".into());
                    let status = r.status().as_u16().to_string();
                    st.metrics
                        .requests
                        .with_label_values(&[&path, &status])
                        .inc();
                    st.metrics
                        .latency
                        .with_label_values(&[&path])
                        .observe(t0.elapsed().as_secs_f64());
                }
                res
            })
        })
        .configure(routes)
}
