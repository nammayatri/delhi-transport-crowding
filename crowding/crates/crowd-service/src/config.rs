//! Service configuration, loaded from Dhall (`DHALL_CONFIG`) with defaults for every field.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceConfig {
    pub bind: String,
    pub snapshot_path: String,
    /// Check the snapshot file for changes this often (0 disables hot reload).
    pub reload_secs: u64,
    pub default_scenario: String,
    pub otp: OtpConfig,
    pub live: LiveConfig,
    pub rt: RtConfig,
    pub planner: PlannerSettings,
    pub cors_allow_any: bool,
    /// Directory with the built web app to serve at `/` (empty: none).
    pub web_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OtpConfig {
    pub enabled: bool,
    /// OTP GTFS GraphQL endpoint, e.g. http://otp:8080/otp/gtfs/v1
    pub graphql_url: String,
    pub timeout_ms: u64,
    /// The OTP build carries the crowding patch and accepts `crowdingReluctance`.
    pub crowding_patch: bool,
    pub num_itineraries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveConfig {
    pub enabled: bool,
    pub window_secs: i64,
    /// Pseudo-tickets pulling a route's factor toward the anchor.
    pub prior_tickets: f64,
    pub min_factor: f64,
    pub max_factor: f64,
    /// Anchor routes at the whole city's live ratio (true) or at 1 (false).
    pub city_anchor: bool,
    /// The factor's distance from 1 shrinks by this per whole hour ahead.
    pub decay_per_hour: f64,
    /// "wall": now is the wall clock. "replay": now follows the latest ingested ticket.
    pub clock: String,
    pub kafka: KafkaConfig,
}

/// Live bus positions (GTFS Realtime VehiclePositions).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RtConfig {
    pub enabled: bool,
    pub url: String,
    /// Environment variable holding the API key, sent as `x-api-key` (empty: no key).
    pub api_key_env: String,
    pub poll_secs: u64,
    /// A feed older than this is not used for live factors.
    pub max_age_secs: i64,
    /// Pseudo-count of buses pulling the supply factor toward 1, and its bounds.
    pub pseudo_buses: f64,
    pub min_factor: f64,
    pub max_factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KafkaConfig {
    pub enabled: bool,
    pub brokers: String,
    pub topic: String,
    pub group_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlannerSettings {
    pub max_transfers: u32,
    pub max_access_m: f64,
    pub max_transfer_m: f64,
    /// Extra departures tried after the requested time, minutes.
    pub later_departures_min: Vec<u32>,
    /// Crowding reluctance at comfort = 1.
    pub max_reluctance: f64,
    pub transfer_penalty_min: f64,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8090".into(),
            snapshot_path: "data/out/full/snapshot.msgpack.zst".into(),
            reload_secs: 30,
            default_scenario: "mid".into(),
            otp: OtpConfig::default(),
            live: LiveConfig::default(),
            rt: RtConfig::default(),
            planner: PlannerSettings::default(),
            cors_allow_any: true,
            web_dir: String::new(),
        }
    }
}

impl Default for OtpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            graphql_url: "http://localhost:8080/otp/gtfs/v1".into(),
            timeout_ms: 8000,
            crowding_patch: false,
            num_itineraries: 5,
        }
    }
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_secs: 3600,
            prior_tickets: 200.0,
            min_factor: 0.67,
            max_factor: 1.5,
            city_anchor: true,
            decay_per_hour: 0.85,
            clock: "wall".into(),
            kafka: KafkaConfig::default(),
        }
    }
}

impl Default for RtConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            url: "https://dts-backend.transportstack.in/api/dataset/otd/get-file?agency=delhi-buses&category=realtime_gtfs&filename=VehiclePositions.pb".into(),
            api_key_env: "DTS_API_KEY".into(),
            poll_secs: 30,
            max_age_secs: 300,
            pseudo_buses: 1.0,
            min_factor: 0.67,
            max_factor: 1.5,
        }
    }
}

impl Default for KafkaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            brokers: "localhost:9092".into(),
            topic: "tickets".into(),
            group_id: "crowd-service".into(),
        }
    }
}

impl Default for PlannerSettings {
    fn default() -> Self {
        Self {
            max_transfers: 2,
            max_access_m: 900.0,
            max_transfer_m: 350.0,
            later_departures_min: vec![10, 20, 30],
            max_reluctance: 2.5,
            transfer_penalty_min: 5.0,
        }
    }
}

impl ServiceConfig {
    pub fn load() -> anyhow::Result<Self> {
        match std::env::var("DHALL_CONFIG") {
            Ok(path) if !path.is_empty() => serde_dhall::from_file(&path)
                .parse()
                .map_err(|e| anyhow::anyhow!("{path}: {e}")),
            _ => Ok(Self::default()),
        }
    }
}
