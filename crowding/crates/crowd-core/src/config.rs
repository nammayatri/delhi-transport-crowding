//! Model assumptions. Every number the model relies on lives here, so a reviewer can see
//! (and change) each one. Binaries load this from Dhall; [`ModelConfig::default`] mirrors
//! `config/model.dhall` and is used by tests.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Width of a time-of-day bin in minutes.
    pub bin_minutes: u32,
    /// First and last minute of the service day covered by bins (minutes after midnight).
    pub first_minute: u32,
    pub last_minute: u32,
    /// Smoothing kernel applied across neighbouring bins, centred. Normalised on use.
    pub kernel: Vec<f64>,
    /// Half-width, in bins, of the wider window used as the shrinkage prior.
    pub prior_window_bins: u32,
    /// Strength of the shrinkage prior, in trip-days.
    pub shrink_pseudo_trips: f64,
    /// Public holidays, treated like Sundays. ISO dates.
    pub holidays: Vec<String>,
    /// How ticketed riders are expanded to all riders.
    pub ridership: Ridership,
    /// Trip-to-trip coefficient of variation of the load within a bin.
    pub trip_cv: f64,
    pub confidence: ConfidenceRule,
    pub capacity_classes: Vec<CapacityClass>,
    /// First matching rule decides a route's bus class.
    pub capacity_rules: Vec<CapacityRule>,
    pub multipliers: Multipliers,
    /// Probability above which a level is described as "usually".
    pub usually: f64,
    pub matching: Matching,
    /// Share of riders assumed to alight one stop early or late (split evenly). 0 = none.
    pub alight_spread: f64,
    /// A gap longer than this between two tickets on one vehicle starts a new trip.
    pub trip_split_gap_minutes: u32,
    /// Where the number of buses comes from:
    /// - "tickets": only bus trips rebuilt from vehicle-tagged tickets, corrected for trips
    ///   that sold no tagged ticket (the timetable only places buses along the route);
    /// - "timetable": the GTFS timetable, scaled to the fleet seen in the tickets;
    /// - "max": the larger of the two in each bin.
    pub supply_source: String,
    /// Upper bound on the correction for trips that carried no tagged ticket.
    pub capture_cap: f64,
    /// Capacity consistency: if a route's weekday load at this quantile (bus-weighted,
    /// medium or high confidence bins) exceeds crush capacity under the city-wide
    /// expansion, the route's expansion is lowered until it does not. 0 disables.
    pub capacity_check_quantile: f64,
    /// Merge copies of one route number run by different operators (median stop match
    /// distance below `merge_max_median_m`).
    pub merge_route_copies: bool,
    pub merge_max_median_m: f64,
    /// Scale a route's tickets up for its tickets whose boarding stop could not be placed,
    /// at most by this factor (1 disables).
    pub unmatched_correction_cap: f64,
    /// Scale timetable supply to the buses seen running in the tickets (never above 1,
    /// never below `fleet_min_scale`). Disable to trust the timetable as is.
    pub fleet_calibration: bool,
    pub fleet_min_scale: f64,
}

/// Ticket records cover only part of all boardings: concession and pass holders, cash
/// tickets and fare evasion do not appear. The expansion factor scales ticketed riders to
/// all riders. It is the published weekday ridership divided by the ticketed riders on an
/// average weekday, unless `fixed_expansion` is above zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ridership {
    pub weekday_boardings: Scenario,
    pub fixed_expansion: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Scenario {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceRule {
    /// Minimum ticketed riders behind a bin for "high" and "medium" confidence.
    pub high_min_tickets: f64,
    pub medium_min_tickets: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityClass {
    pub name: String,
    pub label: String,
    /// Seats.
    pub seats: f64,
    /// Total load at which standing stops being comfortable (seats + comfortable standing).
    pub comfortable: f64,
    /// Total load at crush capacity.
    pub crush: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityRule {
    /// Route name prefix; empty matches every route.
    pub route_prefix: String,
    /// "any", "ac" or "nonac", tested against the route's majority of tickets.
    pub ac: String,
    pub class: String,
}

/// Time multipliers for in-vehicle time by crowding level (crowding valuation literature).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Multipliers {
    pub seated: f64,
    pub standing: f64,
    pub crowded: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Matching {
    /// Minimum fuzzy score (0..100) to accept a stop-name match.
    pub min_score: f64,
    /// Radius for the geographic fallback match, metres.
    pub geo_radius_m: f64,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            bin_minutes: 30,
            first_minute: 4 * 60,
            last_minute: 24 * 60,
            kernel: vec![0.25, 0.5, 0.25],
            prior_window_bins: 4,
            shrink_pseudo_trips: 8.0,
            holidays: vec!["2026-10-02".into(), "2026-10-20".into()],
            ridership: Ridership {
                weekday_boardings: Scenario {
                    low: 3_000_000.0,
                    mid: 4_000_000.0,
                    high: 5_000_000.0,
                },
                fixed_expansion: 0.0,
            },
            trip_cv: 0.35,
            confidence: ConfidenceRule {
                high_min_tickets: 40.0,
                medium_min_tickets: 12.0,
            },
            capacity_classes: vec![
                CapacityClass {
                    name: "12m-ac".into(),
                    label: "12 m AC bus".into(),
                    seats: 37.0,
                    comfortable: 62.0,
                    crush: 85.0,
                },
                CapacityClass {
                    name: "12m-nonac".into(),
                    label: "12 m non-AC bus".into(),
                    seats: 40.0,
                    comfortable: 65.0,
                    crush: 90.0,
                },
                CapacityClass {
                    name: "9m-ev".into(),
                    label: "9 m electric bus".into(),
                    seats: 23.0,
                    comfortable: 38.0,
                    crush: 55.0,
                },
            ],
            capacity_rules: vec![
                CapacityRule {
                    route_prefix: "D-".into(),
                    ac: "any".into(),
                    class: "9m-ev".into(),
                },
                CapacityRule {
                    route_prefix: "".into(),
                    ac: "nonac".into(),
                    class: "12m-nonac".into(),
                },
                CapacityRule {
                    route_prefix: "".into(),
                    ac: "any".into(),
                    class: "12m-ac".into(),
                },
            ],
            multipliers: Multipliers {
                seated: 1.0,
                standing: 1.4,
                crowded: 1.9,
            },
            usually: 0.6,
            matching: Matching {
                min_score: 82.0,
                geo_radius_m: 400.0,
            },
            alight_spread: 0.0,
            trip_split_gap_minutes: 90,
            supply_source: "tickets".into(),
            capture_cap: 3.0,
            capacity_check_quantile: 0.99,
            merge_route_copies: true,
            merge_max_median_m: 300.0,
            unmatched_correction_cap: 1.5,
            fleet_calibration: true,
            fleet_min_scale: 0.3,
        }
    }
}

impl ModelConfig {
    pub fn n_bins(&self) -> usize {
        ((self.last_minute - self.first_minute) / self.bin_minutes) as usize
    }

    /// Bin index for a time of day in seconds after midnight, if inside the covered range.
    pub fn bin_of(&self, secs: u32) -> Option<usize> {
        let m = secs / 60;
        if m < self.first_minute || m >= self.last_minute {
            return None;
        }
        Some(((m - self.first_minute) / self.bin_minutes) as usize)
    }

    /// Start of a bin in minutes after midnight.
    pub fn bin_start_minute(&self, bin: usize) -> u32 {
        self.first_minute + bin as u32 * self.bin_minutes
    }

    pub fn class(&self, name: &str) -> Option<&CapacityClass> {
        self.capacity_classes.iter().find(|c| c.name == name)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.bin_minutes > 0, "bin_minutes must be positive");
        anyhow::ensure!(
            self.first_minute < self.last_minute,
            "first_minute < last_minute"
        );
        anyhow::ensure!(
            !self.kernel.is_empty() && self.kernel.len() % 2 == 1,
            "kernel must be odd"
        );
        anyhow::ensure!(!self.capacity_classes.is_empty(), "no capacity classes");
        anyhow::ensure!(
            matches!(self.supply_source.as_str(), "tickets" | "timetable" | "max"),
            "supply_source must be tickets, timetable or max"
        );
        for r in &self.capacity_rules {
            anyhow::ensure!(self.class(&r.class).is_some(), "unknown class {}", r.class);
            anyhow::ensure!(
                matches!(r.ac.as_str(), "any" | "ac" | "nonac"),
                "capacity rule ac must be any, ac or nonac"
            );
        }
        for c in &self.capacity_classes {
            anyhow::ensure!(
                c.seats < c.comfortable && c.comfortable < c.crush,
                "class {}",
                c.name
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid_and_bins_line_up() {
        let c = ModelConfig::default();
        c.validate().unwrap();
        assert_eq!(c.n_bins(), 40);
        assert_eq!(c.bin_of(4 * 3600), Some(0));
        assert_eq!(c.bin_of(18 * 3600 + 10 * 60), Some(28));
        assert_eq!(c.bin_start_minute(28), 18 * 60);
        assert_eq!(c.bin_of(3 * 3600), None);
    }
}
