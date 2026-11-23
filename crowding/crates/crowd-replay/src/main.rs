//! Replay ticket records as if they were arriving live.
//!
//! Reads the published CSV, keeps one service date (and optionally a time window), and sends
//! tickets in timestamp order to the crowding service (`POST /v1/tickets`) or to a Kafka
//! topic, `speed` times faster than real time. With the service clock in "replay" mode, the
//! service's "now" follows the replayed tickets.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use crowd_core::tickets::{ist, parse_ts, RawTicket};

#[derive(Parser, Debug)]
struct Args {
    /// Ticket CSV in the published format.
    #[arg(long)]
    tickets: std::path::PathBuf,
    /// Service date to replay.
    #[arg(long)]
    date: NaiveDate,
    /// Start and end of the window, HH:MM.
    #[arg(long, default_value = "05:00")]
    from: String,
    #[arg(long, default_value = "23:59")]
    to: String,
    /// Replay speed relative to real time (0 = as fast as possible).
    #[arg(long, default_value_t = 60.0)]
    speed: f64,
    /// Crowding service base URL.
    #[arg(long, default_value = "http://localhost:8090")]
    url: String,
    /// Send to this Kafka topic instead of HTTP (needs the `kafka` feature).
    #[arg(long)]
    kafka_topic: Option<String>,
    #[arg(long, default_value = "localhost:9092")]
    kafka_brokers: String,
    /// Tickets per request.
    #[arg(long, default_value_t = 200)]
    batch: usize,
}

fn secs(hhmm: &str) -> Result<u32> {
    let (h, m) = hhmm.split_once(':').context("time must be HH:MM")?;
    Ok(h.parse::<u32>()? * 3600 + m.parse::<u32>()? * 60)
}

fn read(args: &Args) -> Result<Vec<(i64, RawTicket)>> {
    let (t0, t1) = (secs(&args.from)?, secs(&args.to)?);
    let mut r = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(&args.tickets)?;
    let h = r.headers()?.clone();
    let col = |n: &str| h.iter().position(|x| x.trim_start_matches('\u{feff}') == n);
    let get = |rec: &csv::StringRecord, n: &str| -> Option<String> {
        let v = col(n).and_then(|i| rec.get(i))?.trim();
        (!v.is_empty() && !v.eq_ignore_ascii_case("null")).then(|| v.to_string())
    };
    let mut out = Vec::new();
    for rec in r.records() {
        let rec = rec?;
        let Some(at) = get(&rec, "created_at") else {
            continue;
        };
        let Ok(ts) = parse_ts(&at, ist()) else {
            continue;
        };
        let s = ts.time().num_seconds_from_midnight();
        if ts.date_naive() != args.date || s < t0 || s > t1 {
            continue;
        }
        out.push((
            ts.timestamp(),
            RawTicket {
                passenger_count: get(&rec, "passenger_count").and_then(|v| v.parse().ok()),
                amount: get(&rec, "amount").and_then(|v| v.parse().ok()),
                vehicle_number: get(&rec, "vehicle_number"),
                ticket_type_id: get(&rec, "ticket_type_id").and_then(|v| v.parse().ok()),
                start_stop_name: get(&rec, "start_stop_name").unwrap_or_default(),
                end_stop_name: get(&rec, "end_stop_name"),
                route: get(&rec, "route").unwrap_or_default(),
                created_at: at,
                is_ac: get(&rec, "is_ac").map(|v| v.eq_ignore_ascii_case("true")),
            },
        ));
    }
    out.sort_by_key(|x| x.0);
    Ok(out)
}

use chrono::Timelike;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    let tickets = read(&args)?;
    anyhow::ensure!(
        !tickets.is_empty(),
        "no tickets on {} between {} and {}",
        args.date,
        args.from,
        args.to
    );
    tracing::info!(tickets = tickets.len(), date = %args.date, speed = args.speed, "replaying");
    let sink = Sink::new(&args)?;
    let start_wall = Instant::now();
    let start_ts = tickets[0].0;
    let mut sent = 0usize;
    for chunk in tickets.chunks(args.batch.max(1)) {
        if args.speed > 0.0 {
            let due = Duration::from_secs_f64((chunk[0].0 - start_ts) as f64 / args.speed);
            if let Some(wait) = due.checked_sub(start_wall.elapsed()) {
                tokio::time::sleep(wait).await;
            }
        }
        let body: Vec<&RawTicket> = chunk.iter().map(|x| &x.1).collect();
        sink.send(&body).await?;
        sent += chunk.len();
        if sent % (args.batch * 50) < args.batch {
            tracing::info!(sent, at = %chunk[chunk.len() - 1].1.created_at, "progress");
        }
    }
    tracing::info!(sent, "done");
    Ok(())
}

enum Sink {
    Http {
        client: reqwest::Client,
        url: String,
    },
    #[cfg(feature = "kafka")]
    Kafka {
        producer: rdkafka::producer::FutureProducer,
        topic: String,
    },
}

impl Sink {
    fn new(args: &Args) -> Result<Self> {
        if let Some(_topic) = &args.kafka_topic {
            #[cfg(feature = "kafka")]
            {
                let producer = rdkafka::config::ClientConfig::new()
                    .set("bootstrap.servers", &args.kafka_brokers)
                    .create()?;
                return Ok(Sink::Kafka {
                    producer,
                    topic: _topic.clone(),
                });
            }
            #[cfg(not(feature = "kafka"))]
            anyhow::bail!(
                "built without the kafka feature; use HTTP or rebuild with --features kafka"
            );
        }
        Ok(Sink::Http {
            client: reqwest::Client::new(),
            url: format!("{}/v1/tickets", args.url.trim_end_matches('/')),
        })
    }

    async fn send(&self, batch: &[&RawTicket]) -> Result<()> {
        match self {
            Sink::Http { client, url } => {
                let r = client.post(url).json(batch).send().await?;
                anyhow::ensure!(r.status().is_success(), "ingest failed: {}", r.status());
            }
            #[cfg(feature = "kafka")]
            Sink::Kafka { producer, topic } => {
                let payload = serde_json::to_vec(batch)?;
                producer
                    .send(
                        rdkafka::producer::FutureRecord::<(), _>::to(topic).payload(&payload),
                        Duration::from_secs(5),
                    )
                    .await
                    .map_err(|(e, _)| anyhow::anyhow!(e))?;
            }
        }
        Ok(())
    }
}
