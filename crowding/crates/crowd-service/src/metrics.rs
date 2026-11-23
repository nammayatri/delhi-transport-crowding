use prometheus::{
    Encoder, Gauge, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, Opts, Registry,
    TextEncoder,
};

pub struct Metrics {
    pub registry: Registry,
    pub requests: IntCounterVec,
    pub latency: HistogramVec,
    pub tickets_ingested: IntCounter,
    pub tickets_rejected: IntCounter,
    pub snapshot_loaded: Gauge,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let requests = IntCounterVec::new(
            Opts::new("crowd_http_requests_total", "HTTP requests"),
            &["path", "status"],
        )
        .unwrap();
        let latency = HistogramVec::new(
            HistogramOpts::new(
                "crowd_http_request_duration_seconds",
                "HTTP request latency",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["path"],
        )
        .unwrap();
        let tickets_ingested =
            IntCounter::new("crowd_live_tickets_ingested_total", "Live tickets accepted").unwrap();
        let tickets_rejected =
            IntCounter::new("crowd_live_tickets_rejected_total", "Live tickets rejected").unwrap();
        let snapshot_loaded = Gauge::new(
            "crowd_snapshot_loaded_timestamp",
            "Unix time the snapshot was loaded",
        )
        .unwrap();
        let info = IntCounterVec::new(
            Opts::new("crowd_build_info", "Build information"),
            &["version"],
        )
        .unwrap();
        info.with_label_values(&[env!("CARGO_PKG_VERSION")]).inc();
        for c in [
            Box::new(requests.clone()) as Box<dyn prometheus::core::Collector>,
            Box::new(latency.clone()),
            Box::new(tickets_ingested.clone()),
            Box::new(tickets_rejected.clone()),
            Box::new(snapshot_loaded.clone()),
            Box::new(info),
        ] {
            registry.register(c).unwrap();
        }
        Self {
            registry,
            requests,
            latency,
            tickets_ingested,
            tickets_rejected,
            snapshot_loaded,
        }
    }

    pub fn render(&self) -> String {
        let mut buf = Vec::new();
        TextEncoder::new()
            .encode(&self.registry.gather(), &mut buf)
            .ok();
        String::from_utf8(buf).unwrap_or_default()
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}
