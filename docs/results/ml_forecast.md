# A simple machine-learning forecast, against the crowding model

**Question.** Trained only on the October tickets, can a gradient-boosted model forecast the last week better
than the crowding model (pooled running count with shrinkage)?

**Setup.** Weekday cells (route pattern x stretch x 30-minute bin). The target is ticketed riders per bus pass,
kernel-smoothed but not shrunk, with bus passes as the exposure. scikit-learn `HistGradientBoostingRegressor`
with Poisson loss, trained on features from Oct 1-17 to predict Oct 18-24 (1,818,614 cells), then given
Oct 1-24 to predict Oct 25-31 (1,818,983 cells). Nothing from the evaluation week is used for fitting. The crowding
model is scored on the same cells, from its Oct 1-24 run. Loads use the same expansion, route capacity
adjustment and crush cap for every model; levels use the model's own rule.

Features, all from the feature window or known in advance: the cell's ticketed riders per bus and the tickets
behind it; the same stretch over +/- 1 and 2 hours and all day; the route at the same half hour and all day; the
city at the same half hour; half hour, position and km along the route, route length, bus passes, bus class,
direction (toward Connaught Place or away), distance and bearing from Connaught Place. The second ML model also
gets the crowding model's own estimate and learns a correction factor to it (a Poisson offset), so it starts from
the crowding model rather than from scratch.

| model | Spearman vs raw test | Spearman vs test model | mean abs error (riders/bus) | level agreement | kappa | Poisson deviance, all cells | Poisson deviance, >= 10 tickets | level-matched deviance, all cells | level-matched deviance, >= 10 tickets | forecast / actual week total |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| crowding model (current) | 0.887 | 0.912 | 10.93 | 75.5% | 0.595 | 0.1450 | 0.1308 | 0.1450 | 0.1344 | 1.011 |
| stretch all-day average | 0.281 | 0.385 | 25.35 | 50.6% | 0.036 | 0.5196 | 1.0572 | 0.5195 | 1.0651 | 1.006 |
| route all-day average | 0.019 | 0.086 | 28.49 | 50.3% | 0.001 | 0.6724 | 1.4696 | 0.6724 | 1.4795 | 1.006 |
| ML, tickets only | 0.895 | 0.894 | 11.27 | 74.9% | 0.579 | 0.1323 | 0.1459 | 0.1298 | 0.1231 | 0.939 |
| ML on top of the crowding model | 0.894 | 0.897 | 11.35 | 74.9% | 0.578 | 0.1344 | 0.1471 | 0.1314 | 0.1201 | 0.932 |

Spearman, error and level agreement are on the 157,530 cells with at least 10 test tickets
(as in `stability.md`); "raw test" is the observed last-week value, "test model" the crowding model fitted on the
last week alone. Poisson deviance (lower is better) covers every cell, weighted by bus passes, so it also
measures the sparse cells.

## Reading

- Poisson deviance on all cells, relative to the crowding model: stretch all-day average -258.2%, route all-day average -363.6%, ML, tickets only 8.8%, ML on top of the crowding model 7.4%
  (positive = better forecast).
- Ranking against the raw last week: the tickets-only ML model scores Spearman 0.895 and
  74.9% level agreement; the crowding model 0.887 and
  75.5%; ML on top of the crowding model 0.894 and
  74.9%. Best on this measure: ML, tickets only.
- **The weeks differ in level, and one month cannot teach that.** Ticketed riding in the fitting target week
  (Oct 18-24, Dussehra week) was 0.92x the crowding model's estimate; in the evaluation week (Oct 25-31)
  it was 0.99x. The ML models learn the first week's level and carry it into the second, so they
  under-forecast the last week (forecast / actual: tickets-only
  0.939, crowding model 1.011).
  The level-matched columns rescale each forecast to the week's actual total: they show how well a model spreads
  riders over stretches and hours, apart from the overall level. In use, the live-ticket factor (technical note
  §2.9) supplies that level.
- What the tickets-only model leans on (increase in deviance when the feature is shuffled):
  rate 1.1230; rate_1h 0.0820; route_bin_rate 0.0347; log_buses 0.0195; seg_rate 0.0158; log_route_tix 0.0130.
- Both approaches learn from the same tickets, so neither can fix what the tickets cannot show: the city-wide
  expansion from ticketed riders to all riders, app use that differs by route, and buses that carry no
  tagged ticket. The ML model forecasts ticketed riders per bus; the step to crowding (expansion, bus class,
  capacity) is the same for both.
