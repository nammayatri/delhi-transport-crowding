//! Comfort cost: how much worse a ride feels than its clock time.
//!
//! Each segment's ride time is weighted by a multiplier for the crowding level: seated 1.0,
//! standing about 1.4, crowded about 1.9 (values in the range reported by crowding
//! valuation studies). We use the expected multiplier over the level probabilities, so a
//! segment that is crowded on 40% of buses costs in between.

use crate::config::ModelConfig;
use crate::levels::{expected_multiplier, CellView};

/// Generalised ride time in seconds for a sequence of (ride seconds, crowding) segments.
/// Segments without data count at clock time.
pub fn comfort_seconds(cfg: &ModelConfig, segs: &[(u32, Option<CellView>)]) -> f64 {
    segs.iter()
        .map(|(ride, v)| {
            *ride as f64
                * v.as_ref()
                    .map(|v| expected_multiplier(cfg, v))
                    .unwrap_or(1.0)
        })
        .sum()
}

/// Score an itinerary for ranking: clock time plus `reluctance` times the crowding extra.
pub fn ranked_cost(duration_secs: f64, crowding_extra_secs: f64, reluctance: f64) -> f64 {
    duration_secs + reluctance * crowding_extra_secs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::levels::{evaluate, Expansion, ScenarioPick};

    #[test]
    fn crowded_minutes_cost_more() {
        let cfg = ModelConfig::default();
        let c = cfg.class("12m-ac").unwrap().clone();
        let e = Expansion {
            low: 40.0,
            mid: 50.0,
            high: 60.0,
        };
        let crowded = evaluate(&cfg, &c, &e, ScenarioPick::Mid, 2.0, 200.0, 4.0);
        let empty = evaluate(&cfg, &c, &e, ScenarioPick::Mid, 0.1, 200.0, 4.0);
        let a = comfort_seconds(&cfg, &[(600, Some(crowded))]);
        let b = comfort_seconds(&cfg, &[(600, Some(empty))]);
        assert!(a > 1000.0 && a <= 1140.0 + 1e-6, "{a}");
        assert!((b - 600.0).abs() < 1.0, "{b}");
        assert_eq!(comfort_seconds(&cfg, &[(600, None)]), 600.0);
        assert!(ranked_cost(2700.0, 900.0, 1.0) > ranked_cost(3300.0, 0.0, 1.0));
    }
}
