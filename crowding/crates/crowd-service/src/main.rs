use std::sync::Arc;

use actix_cors::Cors;
use actix_web::{web, HttpServer};
use crowd_service::config::ServiceConfig;
use crowd_service::state::{reload_loop, AppState};

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cfg = ServiceConfig::load()?;
    tracing::info!(bind = %cfg.bind, snapshot = %cfg.snapshot_path, otp = cfg.otp.enabled, "starting crowd-service");
    let state = Arc::new(AppState::new(cfg.clone()));
    {
        let st = state.clone();
        let path = std::path::PathBuf::from(&cfg.snapshot_path);
        // Load in the background so /health answers at once and /ready flips when done.
        tokio::task::spawn_blocking(move || {
            if let Err(e) = st.load_snapshot(&path) {
                tracing::error!(error = %e, "could not load snapshot");
            }
        });
    }
    tokio::spawn(reload_loop(state.clone()));
    tokio::spawn(crowd_service::live_rt::run(state.clone()));
    if cfg.live.kafka.enabled {
        tokio::spawn(crowd_service::kafka::run(state.clone()));
    }
    let data = web::Data::from(state);
    let web_dir = cfg.web_dir.clone();
    let cors_any = cfg.cors_allow_any;
    HttpServer::new(move || {
        let cors = if cors_any {
            Cors::permissive()
        } else {
            Cors::default()
        };
        let app = crowd_service::app(data.clone()).wrap(cors);
        if web_dir.is_empty() {
            app.default_service(web::to(|| async {
                actix_web::HttpResponse::NotFound().finish()
            }))
        } else {
            app.default_service(actix_files::Files::new("/", &web_dir).index_file("index.html"))
        }
    })
    .bind(&cfg.bind)?
    .run()
    .await?;
    Ok(())
}
