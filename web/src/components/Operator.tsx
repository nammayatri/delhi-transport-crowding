import { useEffect, useState } from "react";
import { api, Level } from "../api";
import { LEVEL_COLOR, LevelChip } from "../levels";

const WINDOWS: Record<string, [string, string, string]> = {
  am: ["07:00", "10:00", "Morning peak 07–10"],
  mid: ["10:00", "16:00", "Midday 10–16"],
  pm: ["17:00", "20:00", "Evening peak 17–20"],
  late: ["20:00", "23:30", "Evening 20–23:30"],
};

function Heatmap({ routeId, daytype, scenario }: { routeId: string; daytype: string; scenario: string }) {
  const [p, setP] = useState<any>(null);
  useEffect(() => {
    api.profile(routeId, daytype, scenario).then(setP).catch(() => setP(null));
  }, [routeId, daytype, scenario]);
  if (!p) return <p className="muted">Loading route…</p>;
  const pat = p.patterns.find((x: any) => x.has_data) ?? p.patterns[0];
  const bins: string[] = p.bins;
  const cls = pat.bus_class;
  const color = (load: number | null) => {
    if (load == null) return "#f3f4f6";
    const r = load / cls.comfortable;
    if (load < cls.seats) return `rgba(31,122,85,${0.15 + 0.6 * (load / cls.seats)})`;
    if (r < 1) return LEVEL_COLOR.standing;
    return LEVEL_COLOR.crowded;
  };
  return (
    <div className="heat">
      <h3>
        {p.name}: riders on board by stretch and time ({daytype})
      </h3>
      <p className="muted">
        {cls.label}: {cls.seats} seats, comfortable up to {cls.comfortable}. Each cell is the expected load per bus. Pale = few tickets.
      </p>
      <div className="heatgrid" style={{ gridTemplateColumns: `200px repeat(${bins.length}, 1fr)` }}>
        <div />
        {bins.map((b, i) => (
          <div key={b} className="hbin">
            {i % 4 === 0 ? b : ""}
          </div>
        ))}
        {pat.load.map((row: (number | null)[], seg: number) => (
          <>
            <div key={"n" + seg} className="hname" title={`${pat.stops[seg].name} → ${pat.stops[seg + 1].name}`}>
              {pat.stops[seg].name}
            </div>
            {row.map((v, b) => (
              <div
                key={seg + "-" + b}
                className="hcell"
                style={{ background: color(v), opacity: pat.confidence[seg][b] === 0 ? 0.45 : 1 }}
                title={v == null ? "no bus" : `${pat.stops[seg].name} → ${pat.stops[seg + 1].name} at ${bins[b]}: about ${Math.round(v)} per bus`}
              />
            ))}
          </>
        ))}
      </div>
    </div>
  );
}

export function Operator() {
  const [daytype, setDaytype] = useState("weekday");
  const [win, setWin] = useState("am");
  const [scenario, setScenario] = useState("mid");
  const [hot, setHot] = useState<any[]>([]);
  const [routes, setRoutes] = useState<any[]>([]);
  const [route, setRoute] = useState<string | null>(null);

  useEffect(() => {
    const [f, t] = WINDOWS[win];
    api.hotspots(daytype, f, t, scenario).then((r) => setHot(r.hotspots)).catch(() => setHot([]));
    api.operatorRoutes(daytype, f, t, scenario).then((r) => {
      setRoutes(r.routes);
      if (!route && r.routes[0]) setRoute(r.routes[0].route_id);
    });
  }, [daytype, win, scenario]);

  return (
    <div className="operator">
      <div className="toolbar">
        <h2>Where are buses overloaded?</h2>
        <select value={daytype} onChange={(e) => setDaytype(e.target.value)}>
          <option value="weekday">Weekday</option>
          <option value="saturday">Saturday</option>
          <option value="sunday">Sunday / holiday</option>
        </select>
        <select value={win} onChange={(e) => setWin(e.target.value)}>
          {Object.entries(WINDOWS).map(([k, v]) => (
            <option key={k} value={k}>
              {v[2]}
            </option>
          ))}
        </select>
        <select value={scenario} onChange={(e) => setScenario(e.target.value)}>
          <option value="low">Low ridership</option>
          <option value="mid">Central ridership</option>
          <option value="high">High ridership</option>
        </select>
      </div>
      <div className="cols">
        <section>
          <h3>Routes that need more buses</h3>
          <p className="muted">Extra trips = trips needed so the busiest stretch in each half hour is down to comfortable standing.</p>
          <table>
            <thead>
              <tr>
                <th>Route</th>
                <th>Busiest stretch</th>
                <th>When</th>
                <th>Peak load</th>
                <th>Crowded km</th>
                <th>Extra trips</th>
              </tr>
            </thead>
            <tbody>
              {routes.slice(0, 25).map((r) => (
                <tr key={r.pattern} className={route === r.route_id ? "sel" : ""} onClick={() => setRoute(r.route_id)}>
                  <td>
                    <b>{r.route}</b>
                  </td>
                  <td>
                    {r.peak_from} → {r.peak_to}
                  </td>
                  <td>{r.peak_time}</td>
                  <td>
                    <LevelChip level={r.peak_level as Level} /> {Math.round(r.peak_load)}
                  </td>
                  <td>{Math.round(100 * r.share_crowded)}%</td>
                  <td>{r.extra_trips_needed}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
        <section>
          <h3>Most overloaded stretches</h3>
          <p className="muted">Ranked by riders above comfortable load, summed over buses in the window.</p>
          <table>
            <thead>
              <tr>
                <th>Route</th>
                <th>Stretch</th>
                <th>Peak</th>
                <th>Load</th>
                <th>Conf.</th>
              </tr>
            </thead>
            <tbody>
              {hot.slice(0, 25).map((h, i) => (
                <tr key={i} onClick={() => setRoute(h.route_id)}>
                  <td>
                    <b>{h.route}</b>
                  </td>
                  <td>
                    {h.from} → {h.to}
                  </td>
                  <td>{h.peak_time}</td>
                  <td>{Math.round(h.peak_load)}</td>
                  <td>{h.confidence}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      </div>
      {route && <Heatmap routeId={route} daytype={daytype} scenario={scenario} />}
    </div>
  );
}
