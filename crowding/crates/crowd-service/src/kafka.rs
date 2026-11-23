//! Live tickets from Kafka (JSON messages in the published ticket shape). Built with the
//! `kafka` cargo feature; otherwise the HTTP endpoint `POST /v1/tickets` is the live source.

use std::sync::Arc;

use crate::state::AppState;

#[cfg(feature = "kafka")]
pub async fn run(state: Arc<AppState>) {
    use crowd_core::tickets::RawTicket;
    use futures_util::StreamExt;
    use rdkafka::config::ClientConfig;
    use rdkafka::consumer::{Consumer, StreamConsumer};
    use rdkafka::Message;

    let k = &state.cfg.live.kafka;
    let consumer: StreamConsumer = match ClientConfig::new()
        .set("bootstrap.servers", &k.brokers)
        .set("group.id", &k.group_id)
        .set("enable.auto.commit", "true")
        .set("auto.offset.reset", "latest")
        .create()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "kafka consumer");
            return;
        }
    };
    if let Err(e) = consumer.subscribe(&[k.topic.as_str()]) {
        tracing::error!(error = %e, "kafka subscribe");
        return;
    }
    tracing::info!(topic = %k.topic, "consuming live tickets from kafka");
    let mut stream = consumer.stream();
    while let Some(msg) = stream.next().await {
        let Ok(m) = msg else { continue };
        let Some(payload) = m.payload() else { continue };
        let Ok(l) = state.snap() else { continue };
        let parsed: Result<Vec<RawTicket>, _> = serde_json::from_slice(payload)
            .or_else(|_| serde_json::from_slice::<RawTicket>(payload).map(|t| vec![t]));
        match parsed {
            Ok(list) => {
                for t in &list {
                    if crate::api::ingest(&state, &l, t) {
                        state.metrics.tickets_ingested.inc();
                    } else {
                        state.metrics.tickets_rejected.inc();
                    }
                }
            }
            Err(_) => state.metrics.tickets_rejected.inc(),
        }
    }
}

#[cfg(not(feature = "kafka"))]
pub async fn run(_state: Arc<AppState>) {
    tracing::warn!("live.kafka.enabled is set but the service was built without the kafka feature");
}
