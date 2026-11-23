//! Bus class inference. No fleet list is available, so the class of a route is inferred
//! from configurable rules on the route name and the AC flag carried by its tickets.

use crate::config::{CapacityClass, ModelConfig};

/// Pick the class for a route. `ac_share` is the share of the route's tickets marked AC;
/// `None` when the route has no tickets (it is then treated as AC, the majority of fleet).
pub fn infer_class<'a>(
    cfg: &'a ModelConfig,
    route_name: &str,
    ac_share: Option<f64>,
) -> &'a CapacityClass {
    let is_ac = ac_share.map(|s| s >= 0.5).unwrap_or(true);
    for rule in &cfg.capacity_rules {
        if !route_name.starts_with(rule.route_prefix.as_str()) {
            continue;
        }
        let ok = match rule.ac.as_str() {
            "ac" => is_ac,
            "nonac" => !is_ac,
            _ => true,
        };
        if ok {
            if let Some(c) = cfg.class(&rule.class) {
                return c;
            }
        }
    }
    &cfg.capacity_classes[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_pick_expected_classes() {
        let cfg = ModelConfig::default();
        assert_eq!(infer_class(&cfg, "D-034UP", Some(1.0)).name, "9m-ev");
        assert_eq!(infer_class(&cfg, "761UP", Some(0.1)).name, "12m-nonac");
        assert_eq!(infer_class(&cfg, "8UP", Some(0.9)).name, "12m-ac");
        assert_eq!(infer_class(&cfg, "8UP", None).name, "12m-ac");
    }
}
