//! Opt-in machine-learning live adjustment: gradient-boosted trees trained in
//! `analysis/src/nowcast_backtest.py` (scikit-learn, Poisson loss) and evaluated here, so the
//! service needs no Python at run time.
//!
//! The trees forecast ticketed riders in an hour ahead divided by the usual for that hour, from
//! today's tickets so far. In the October replay they cut forecast error by 5.5% against the usual
//! pattern, against 2.0% for the default city-ratio rule, but they get the city's total less
//! right and need retraining as tickets accumulate (technical note 4.2).

use std::path::Path;

use anyhow::{bail, ensure, Context};
use serde::Deserialize;

/// Feature order the trees were trained with (`GBM_FEATURES` in the backtest).
pub const FEATURES: [&str; 12] = [
    "log_usual",     // ln(usual riders in the target hour + 0.1)
    "ahead",         // hours ahead, 1-3
    "hour",          // hour of day now
    "daytype",       // 0 weekday, 1 Saturday, 2 Sunday or holiday
    "log_route_day", // ln(route's usual riders per day + 0.1)
    "r1",            // route: ln((tickets + 1) / (usual + 1)), last hour
    "r2",            // ... last two hours
    "rday",          // ... since 04:00
    "log_exp1",      // ln(route's usual over the last hour + 0.1)
    "log_expday",    // ln(route's usual since 04:00 + 0.1)
    "c1",            // whole city: ln(tickets / usual), last hour (ratio kept within 0.05-20)
    "cday",          // ... since 04:00
];

/// The factor is kept within these bounds whatever the trees say.
pub const MIN_FACTOR: f64 = 0.25;
pub const MAX_FACTOR: f64 = 4.0;

#[derive(Debug, Clone, Deserialize)]
pub struct Tree {
    /// Split feature per node; negative for a leaf.
    pub feature: Vec<i32>,
    pub threshold: Vec<f64>,
    pub left: Vec<u32>,
    pub right: Vec<u32>,
    pub missing_left: Vec<bool>,
    pub value: Vec<f64>,
}

impl Tree {
    fn leaf(&self, x: &[f64]) -> f64 {
        let mut i = 0usize;
        loop {
            let f = self.feature[i];
            if f < 0 {
                return self.value[i];
            }
            let v = x[f as usize];
            let left = if v.is_nan() {
                self.missing_left[i]
            } else {
                v <= self.threshold[i]
            };
            i = if left { self.left[i] } else { self.right[i] } as usize;
        }
    }

    fn check(&self) -> anyhow::Result<()> {
        let n = self.feature.len();
        ensure!(n > 0, "empty tree");
        for len in [
            self.threshold.len(),
            self.left.len(),
            self.right.len(),
            self.missing_left.len(),
            self.value.len(),
        ] {
            ensure!(len == n, "tree arrays differ in length");
        }
        for i in 0..n {
            if self.feature[i] >= 0 {
                ensure!(
                    (self.feature[i] as usize) < FEATURES.len(),
                    "feature out of range"
                );
                // Children come after their parent, so evaluation always ends at a leaf.
                for c in [self.left[i] as usize, self.right[i] as usize] {
                    ensure!(c > i && c < n, "bad child index");
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Check {
    pub x: Vec<f64>,
    pub factor: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveTrees {
    pub features: Vec<String>,
    pub baseline: f64,
    pub trees: Vec<Tree>,
    /// Rows scored by scikit-learn at export, re-scored on load.
    #[serde(default)]
    pub checks: Vec<Check>,
}

impl LiveTrees {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read live trees {}", path.display()))?;
        let t: Self = serde_json::from_str(&text).context("parse live trees")?;
        t.validate()?;
        Ok(t)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.features != FEATURES {
            bail!(
                "live trees were trained on other features: {:?}",
                self.features
            );
        }
        for t in &self.trees {
            t.check()?;
        }
        for c in &self.checks {
            ensure!(
                c.x.len() == FEATURES.len(),
                "check row has the wrong length"
            );
            let got = self.raw_factor(&c.x);
            ensure!(
                (got / c.factor - 1.0).abs() < 1e-6,
                "live trees disagree with their export: {got} vs {}",
                c.factor
            );
        }
        Ok(())
    }

    fn raw_factor(&self, x: &[f64]) -> f64 {
        (self.baseline + self.trees.iter().map(|t| t.leaf(x)).sum::<f64>()).exp()
    }

    /// Riders in the target hour relative to usual, within MIN_FACTOR-MAX_FACTOR.
    pub fn factor(&self, x: &[f64; 12]) -> f64 {
        self.raw_factor(x).clamp(MIN_FACTOR, MAX_FACTOR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One split on r1 (feature 5) at 0.0; missing values go right.
    pub fn tiny() -> LiveTrees {
        let json = serde_json::json!({
            "features": FEATURES,
            "baseline": 0.0,
            "trees": [{
                "feature": [5, -1, -1],
                "threshold": [0.0, 0.0, 0.0],
                "left": [1, 0, 0],
                "right": [2, 0, 0],
                "missing_left": [false, false, false],
                "value": [0.0, -0.2, 0.3]
            }],
            "checks": [{"x": [0.0, 1.0, 9.0, 0.0, 3.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                        "factor": (-0.2f64).exp()}]
        });
        let t: LiveTrees = serde_json::from_value(json).unwrap();
        t.validate().unwrap();
        t
    }

    #[test]
    fn walks_splits_and_missing_values() {
        let t = tiny();
        let mut x = [0.0; 12];
        x[5] = -0.5;
        assert!((t.factor(&x) - (-0.2f64).exp()).abs() < 1e-12);
        x[5] = 0.5;
        assert!((t.factor(&x) - 0.3f64.exp()).abs() < 1e-12);
        x[5] = f64::NAN;
        assert!((t.factor(&x) - 0.3f64.exp()).abs() < 1e-12);
    }

    #[test]
    fn rejects_a_model_that_disagrees_with_its_checks() {
        let mut t = tiny();
        t.checks[0].factor = 2.0;
        assert!(t.validate().is_err());
        let mut t = tiny();
        t.features[0] = "something else".into();
        assert!(t.validate().is_err());
    }
}
