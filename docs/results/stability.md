# Week-to-week stability

Train: run `train` (Oct 1-24, 15 weekdays). Test: run `test` (Oct 25-31,
5 weekdays). Both are full model runs on disjoint dates; each is expanded with its own ridership
factor and capacity check. Compared on weekday cells (route pattern x stop-to-stop segment x 30-minute
bin) present in both runs: 1,817,259 of 1,845,204 train and 1,818,983 test cells; the boarding stop of each
segment agrees in 100.00% of joined cells, so cell keys line up.

**Threshold.** The test week has only 5 weekdays, so test estimates are themselves noisy. Headline
numbers use the 157,530 cells with at least **10 kernel-weighted ticketed riders in the
test week** (8.7% of common cells, 18.1% of
their buses). Results for all cells are given below for completeness.

## Headline

![scatter](../figures/stability_scatter.png)

- Rank correlation (Spearman) of load, train vs test: **0.91** (all common cells:
  0.89). Within a route (ranking cells of the same route against each other,
  averaged over routes): 0.88.
- Level agreement: **80.6%** of cells (bus-weighted 84.5%),
  Cohen's kappa 0.66. 55.6% of these cells show "seats" in the test week, so raw
  agreement is flattered by the dominant class; kappa corrects for that.
- When the train period says standing or crowded, the test week agrees (standing or crowded) in
  86.8% of cells; of the test-week standing/crowded cells, the train period caught
  85.2%.
- Route x hour totals (passengers through per day summed over a route's segments, 17,975 route-hours with
  >= 10 test tickets): Spearman **0.95**, Pearson on logs
  0.94, median absolute log ratio 0.22
  (about 25% typical difference). Within a route, the hourly
  profile shape correlates at a median Spearman of 0.94
  (1,095 routes with >= 8 qualifying hours).

## Does time of day add information? Baselines

Each predictor is fitted on the train period and scored on the 157,530 headline cells. "Route all-day
average" predicts every cell of a route with the route's bus-weighted average weekday load; "segment all-day
average" uses the segment's own all-day average (a stronger baseline: it knows where on the route the load
builds up, but not when).

| target | predictor | Spearman | within-route Spearman | MAE (riders) | median abs log ratio | level agreement | kappa |
| :--- | :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| test model | time-of-day model (train) | 0.911 | 0.881 | 6.25 | 0.138 | 80.6% | 0.662 |
| test model | route all-day average (train) | 0.070 | n/a | 22.16 | 0.695 | 55.6% | 0.001 |
| test model | segment all-day average (train) | 0.358 | 0.409 | 19.00 | 0.571 | 56.4% | 0.044 |
| raw test count | time-of-day model (train) | 0.889 | 0.847 | 10.55 | 0.184 | 76.2% | 0.601 |
| raw test count | route all-day average (train) | -0.000 | n/a | 28.37 | 0.754 | 51.8% | 0.001 |
| raw test count | segment all-day average (train) | 0.252 | 0.299 | 25.40 | 0.639 | 52.0% | 0.029 |

The time-of-day model beats the route all-day average
(Spearman 0.91 vs 0.07; mean absolute error 6.2 vs
22.2 riders per bus) and the segment all-day average (Spearman 0.36).
The gap is widest within routes, which is where time of day is the only information: within-route Spearman
0.88 for the model; the route average is a constant within a route, so it
cannot rank cells of the same route at all (its within-route rank correlation is undefined;
the segment average gets 0.41).

Against the raw test count (no shrinkage, so it does not borrow from neighbouring bins or the route), the
model's Spearman is 0.89: lower, because the raw count carries the full Poisson noise of a
few tickets, but the ordering of predictors is the same.

## Confusion matrix of levels (cells with >= 10 test tickets)

![confusion](../figures/stability_confusion.png)

| train \ test | seats | standing | crowded |
| :--- | ---: | ---: | ---: |
| seats | 78,547 | 10,124 | 196 |
| standing | 8,864 | 34,258 | 4,547 |
| crowded | 181 | 6,574 | 14,239 |

## By confidence class of the train cell

| train confidence | cells | Spearman | level agreement | kappa | median abs log ratio |
| :--- | ---: | ---: | ---: | ---: | ---: |
| low | 503 | 0.852 | 49.5% | 0.119 | 0.720 |
| medium | 57,772 | 0.923 | 81.8% | 0.634 | 0.173 |
| high | 99,255 | 0.916 | 80.1% | 0.670 | 0.122 |
| low (all common cells) | 1,430,314 | 0.832 | 98.1% | 0.360 | 0.543 |
| medium (all common cells) | 283,481 | 0.864 | 85.5% | 0.557 | 0.206 |
| high (all common cells) | 103,464 | 0.902 | 79.6% | 0.658 | 0.128 |

Rows without "(all common cells)" are restricted to the headline cells. The weak spot is the small group of
cells that were low confidence in the train period but well observed in the test week: their median absolute
log ratio is 0.72 (a typical factor of 2.1 between periods) and kappa 0.12. These are
cells where the train period had too few tickets to say much, so the estimate was mostly the prior. Medium
and high confidence cells behave alike. Over all common cells, the high agreement of low-confidence cells
(98.1%) only reflects that both periods show "seats" for nearly all of them.

## Agreement against test-week tickets

![vs tickets](../figures/stability_vs_tickets.png)

| test tickets | cells | Spearman model | Spearman route average | Spearman model vs raw | level agreement | kappa | median abs log ratio |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0-1 | 981,699 | 0.759 | 0.463 | 0.439 | 99.4% | 0.103 | 0.723 |
| 1-3 | 360,818 | 0.789 | 0.160 | 0.666 | 95.6% | 0.299 | 0.344 |
| 3-10 | 317,212 | 0.859 | 0.088 | 0.806 | 88.1% | 0.511 | 0.235 |
| 10-20 | 96,767 | 0.896 | 0.027 | 0.870 | 81.9% | 0.617 | 0.161 |
| 20-40 | 44,523 | 0.893 | -0.009 | 0.875 | 78.1% | 0.644 | 0.122 |
| 40-100 | 15,232 | 0.906 | -0.078 | 0.883 | 79.5% | 0.668 | 0.085 |
| 100+ | 1,008 | 0.856 | -0.138 | 0.833 | 86.9% | 0.704 | 0.051 |

The typical difference between periods (median absolute log ratio) falls from 0.72
in cells with 0-1 test tickets to 0.05 in the best-observed
group, and kappa rises from 0.10 to 0.70. If the crowding pattern had shifted between
the two periods, well-observed cells would disagree about as much as sparse ones; instead disagreement
shrinks as the test week's sample grows, which is what sampling noise produces. Raw level agreement moves
the other way (near 100% in sparse cells) only because sparse cells are almost all "seats" in both periods.

Two cautions on the sparse end. First, in cells with few tickets both periods' estimates lean on the same
kind of prior (the route's level times the city-wide time-of-day shape), so their Spearman of
0.76 at 0-1 tickets partly measures agreement between priors, not
between observations. Second, selecting cells by test-week tickets could favour cells that happened to be busy in
the test week; the effect is small here (median test/train load ratio in the headline set
1.02). Selecting on both periods instead (train >= 30 and
test >= 10 tickets, 132,294 cells) gives Spearman 0.92, level
agreement 81.0% and kappa 0.68.

## Caveats

- Real change between early and late October (festival season, weather) is counted as disagreement
  here; the numbers above include it rather than correct for it.
- Both runs use the same bus supply (timetable and live feed) and the same matching; errors shared by both (missing
  routes, wrong stop matches, a wrong city-wide expansion factor) cannot show up in a train/test comparison.
  This check measures stability, not absolute accuracy.
