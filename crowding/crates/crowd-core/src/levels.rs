//! From an expected onboard load to a crowding level a commuter understands.
//!
//! Internally we keep the GTFS Realtime `OccupancyStatus` scale so the output can be
//! published in a standard feed. Commuters see three states: seats available, standing
//! room, crowded.

use serde::{Deserialize, Serialize};

use crate::config::{CapacityClass, ModelConfig};
use crate::stats::GammaMeanCv;

/// GTFS Realtime `VehiclePosition.OccupancyStatus` values we use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OccupancyStatus {
    Empty = 0,
    ManySeatsAvailable = 1,
    FewSeatsAvailable = 2,
    StandingRoomOnly = 3,
    CrushedStandingRoomOnly = 4,
    Full = 5,
    NoDataAvailable = 7,
}

impl OccupancyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OccupancyStatus::Empty => "EMPTY",
            OccupancyStatus::ManySeatsAvailable => "MANY_SEATS_AVAILABLE",
            OccupancyStatus::FewSeatsAvailable => "FEW_SEATS_AVAILABLE",
            OccupancyStatus::StandingRoomOnly => "STANDING_ROOM_ONLY",
            OccupancyStatus::CrushedStandingRoomOnly => "CRUSHED_STANDING_ROOM_ONLY",
            OccupancyStatus::Full => "FULL",
            OccupancyStatus::NoDataAvailable => "NO_DATA_AVAILABLE",
        }
    }

    pub fn from_load(load: f64, class: &CapacityClass) -> Self {
        if load < 0.1 * class.seats {
            OccupancyStatus::Empty
        } else if load < 0.6 * class.seats {
            OccupancyStatus::ManySeatsAvailable
        } else if load < class.seats {
            OccupancyStatus::FewSeatsAvailable
        } else if load < class.comfortable {
            OccupancyStatus::StandingRoomOnly
        } else if load < class.crush {
            OccupancyStatus::CrushedStandingRoomOnly
        } else {
            OccupancyStatus::Full
        }
    }
}

/// The three commuter-facing levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Seats = 0,
    Standing = 1,
    Crowded = 2,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Seats => "seats",
            Level::Standing => "standing",
            Level::Crowded => "crowded",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Seats => "Seats available",
            Level::Standing => "Standing room",
            Level::Crowded => "Crowded",
        }
    }

    pub fn from_idx(i: usize) -> Level {
        match i {
            0 => Level::Seats,
            1 => Level::Standing,
            _ => Level::Crowded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low = 0,
    Medium = 1,
    High = 2,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }
}

/// Which expansion scenario to read loads under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioPick {
    Low,
    #[default]
    Mid,
    High,
}

/// Everything a consumer needs about one route segment in one time bin.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CellView {
    /// Expected riders on board per bus under the chosen scenario.
    pub load: f64,
    /// Expected load under the low and high expansion scenarios.
    pub load_low: f64,
    pub load_high: f64,
    /// Share of buses in the bin expected to have seats / standing room / crowding.
    pub p_seats: f64,
    pub p_standing: f64,
    pub p_crowded: f64,
    pub level: Level,
    /// True when `level` holds on at least `usually` of the buses.
    pub usually: bool,
    pub status: OccupancyStatus,
    pub confidence: Confidence,
    /// Ticketed riders behind the estimate (kernel-weighted).
    pub tickets: f64,
    /// Scheduled buses per day in this bin.
    pub buses_per_day: f64,
    /// The estimate implied more riders than crush capacity; loads are capped at crush.
    pub over_capacity: bool,
}

/// Expansion factors for the three scenarios.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Expansion {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
}

impl Expansion {
    pub fn pick(&self, s: ScenarioPick) -> f64 {
        match s {
            ScenarioPick::Low => self.low,
            ScenarioPick::Mid => self.mid,
            ScenarioPick::High => self.high,
        }
    }
}

/// Turn a ticketed rate per bus into a crowding view.
///
/// The load on individual buses in a bin varies around the expected value. We model it as
/// a gamma distribution whose coefficient of variation combines the trip-to-trip spread
/// (`trip_cv`) with the sampling error of the ticket count (1 / sqrt(tickets)).
pub fn evaluate(
    cfg: &ModelConfig,
    class: &CapacityClass,
    expansion: &Expansion,
    scenario: ScenarioPick,
    rate: f64,
    tickets: f64,
    buses_per_day: f64,
) -> CellView {
    let e = expansion.pick(scenario);
    let raw = rate * e;
    let over_capacity = raw > class.crush;
    let load = raw.min(class.crush);
    let cv = (cfg.trip_cv.powi(2) + 1.0 / tickets.max(1.0)).sqrt();
    let (p_seats, p_standing, p_crowded) = if load <= 0.0 {
        (1.0, 0.0, 0.0)
    } else {
        let g = GammaMeanCv::new(load, cv);
        let f_seats = g.cdf(class.seats);
        let f_comf = g.cdf(class.comfortable);
        (
            f_seats,
            (f_comf - f_seats).max(0.0),
            (1.0 - f_comf).max(0.0),
        )
    };
    // The level shown is the one the median bus is in.
    let level = if p_crowded >= 0.5 {
        Level::Crowded
    } else if p_crowded + p_standing >= 0.5 {
        Level::Standing
    } else {
        Level::Seats
    };
    let p_level = [p_seats, p_standing, p_crowded][level as usize];
    let confidence = if tickets >= cfg.confidence.high_min_tickets {
        Confidence::High
    } else if tickets >= cfg.confidence.medium_min_tickets {
        Confidence::Medium
    } else {
        Confidence::Low
    };
    CellView {
        load,
        load_low: (rate * expansion.low).min(class.crush),
        load_high: (rate * expansion.high).min(class.crush),
        p_seats,
        p_standing,
        p_crowded,
        level,
        usually: p_level >= cfg.usually,
        status: OccupancyStatus::from_load(load, class),
        confidence,
        tickets,
        buses_per_day,
        over_capacity,
    }
}

/// Expected time multiplier for a segment, weighting each level by its probability.
pub fn expected_multiplier(cfg: &ModelConfig, v: &CellView) -> f64 {
    let m = &cfg.multipliers;
    v.p_seats * m.seated + v.p_standing * m.standing + v.p_crowded * m.crowded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (ModelConfig, Expansion) {
        (
            ModelConfig::default(),
            Expansion {
                low: 30.0,
                mid: 45.0,
                high: 60.0,
            },
        )
    }

    #[test]
    fn low_load_means_seats() {
        let (cfg, e) = setup();
        let c = cfg.class("12m-ac").unwrap().clone();
        let v = evaluate(&cfg, &c, &e, ScenarioPick::Mid, 0.3, 50.0, 4.0);
        assert_eq!(v.level, Level::Seats);
        assert!(v.usually);
        assert_eq!(v.confidence, Confidence::High);
        assert!((v.load - 13.5).abs() < 1e-9);
    }

    #[test]
    fn high_load_means_crowded_on_a_12m_bus() {
        let (cfg, e) = setup();
        let c = cfg.class("12m-ac").unwrap().clone();
        let v = evaluate(&cfg, &c, &e, ScenarioPick::Mid, 2.0, 100.0, 4.0);
        assert_eq!(v.level, Level::Crowded);
        assert_eq!(v.status, OccupancyStatus::Full);
    }

    #[test]
    fn same_load_is_worse_on_a_9m_bus() {
        let (cfg, e) = setup();
        let big = cfg.class("12m-ac").unwrap().clone();
        let small = cfg.class("9m-ev").unwrap().clone();
        let a = evaluate(&cfg, &big, &e, ScenarioPick::Mid, 1.0, 100.0, 4.0);
        let b = evaluate(&cfg, &small, &e, ScenarioPick::Mid, 1.0, 100.0, 4.0);
        assert_eq!(a.level, Level::Standing);
        assert_eq!(b.level, Level::Crowded);
        assert!(expected_multiplier(&cfg, &b) > expected_multiplier(&cfg, &a));
    }

    #[test]
    fn probabilities_sum_to_one_and_sparse_data_is_low_confidence() {
        let (cfg, e) = setup();
        let c = cfg.class("12m-ac").unwrap().clone();
        let v = evaluate(&cfg, &c, &e, ScenarioPick::Mid, 0.9, 3.0, 1.0);
        assert!((v.p_seats + v.p_standing + v.p_crowded - 1.0).abs() < 1e-9);
        assert_eq!(v.confidence, Confidence::Low);
    }
}
