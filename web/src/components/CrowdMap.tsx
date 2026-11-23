import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { LEVEL_COLOR, Legend } from "../levels";
import { MapLayer, MapView } from "./MapView";

const times: string[] = [];
for (let m = 4 * 60; m < 24 * 60; m += 30) times.push(`${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`);

export function CrowdMap() {
  const [daytype, setDaytype] = useState("weekday");
  const [ti, setTi] = useState(times.indexOf("08:30"));
  const [scenario, setScenario] = useState("mid");
  const [data, setData] = useState<any>(null);
  const [onlyBusy, setOnlyBusy] = useState(false);

  useEffect(() => {
    api.map(daytype, times[ti], scenario).then(setData).catch(() => setData(null));
  }, [daytype, ti, scenario]);

  // Two views of the same half hour: what riders experience (weighted by riders on board)
  // and what share of bus trips have seats (weighted by buses).
  const stats = useMemo(() => {
    const f = data?.features ?? [];
    const r = { seats: 0, standing: 0, crowded: 0 } as Record<string, number>;
    const b = { seats: 0, standing: 0, crowded: 0 } as Record<string, number>;
    for (const x of f) {
      const p = x.properties;
      const ps = 1 - p.p_crowded - p.p_standing;
      const wb = p.buses_per_day;
      b.crowded += p.p_crowded * wb;
      b.standing += p.p_standing * wb;
      b.seats += ps * wb;
      // Riders, weighted inside each stretch too: fuller buses carry more people.
      r.crowded += p.riders_crowded;
      r.standing += p.riders_standing;
      r.seats += p.riders_per_day - p.riders_crowded - p.riders_standing;
    }
    const tr = r.seats + r.standing + r.crowded || 1;
    const tb = b.seats + b.standing + b.crowded || 1;
    return {
      riders: { seats: r.seats / tr, standing: r.standing / tr, crowded: r.crowded / tr },
      buses: { seats: b.seats / tb, standing: b.standing / tb, crowded: b.crowded / tb },
      n: f.length,
    };
  }, [data]);

  const layers = useMemo<MapLayer[]>(() => {
    if (!data) return [];
    const feats = onlyBusy ? data.features.filter((f: any) => f.properties.level !== "seats") : data.features;
    return [
      {
        id: "segments",
        data: { type: "FeatureCollection", features: feats },
        paint: {
          "line-color": ["match", ["get", "level"], "crowded", LEVEL_COLOR.crowded, "standing", LEVEL_COLOR.standing, LEVEL_COLOR.seats],
          "line-width": ["interpolate", ["linear"], ["get", "buses_per_day"], 0, 1, 4, 2.5, 20, 6],
          "line-opacity": ["match", ["get", "confidence"], "low", 0.45, 0.9],
        },
        onClick: (p) =>
          `<b>${p.from} → ${p.to}</b><br/>About <b>${Math.round(p.load)}</b> on board per bus (${p.level})<br/>` +
          `${p.buses_per_day} buses in this half hour · confidence ${p.confidence}<br/><small>Routes: ${p.routes}</small>`,
      },
    ];
  }, [data, onlyBusy]);

  return (
    <div className="split">
      <aside className="panel">
        <h2>Crowding across the network</h2>
        <p className="lead">Each line is a stretch between two stops, coloured by how full the buses on it usually are. Faded lines have few tickets behind them.</p>
        <div className="row">
          <label className="field">
            <span>Day</span>
            <select value={daytype} onChange={(e) => setDaytype(e.target.value)}>
              <option value="weekday">Weekday</option>
              <option value="saturday">Saturday</option>
              <option value="sunday">Sunday / holiday</option>
            </select>
          </label>
          <label className="field">
            <span>Ridership assumption</span>
            <select value={scenario} onChange={(e) => setScenario(e.target.value)}>
              <option value="low">Low (3.0 M/day)</option>
              <option value="mid">Central (4.0 M/day)</option>
              <option value="high">High (5.0 M/day)</option>
            </select>
          </label>
        </div>
        <label className="field">
          <span>
            Time: <b>{times[ti]}</b>
          </span>
          <input type="range" min={0} max={times.length - 1} value={ti} onChange={(e) => setTi(+e.target.value)} />
        </label>
        <label className="check">
          <input type="checkbox" checked={onlyBusy} onChange={(e) => setOnlyBusy(e.target.checked)} /> Only show standing and crowded stretches
        </label>
        <p className="sharehead">Riders on board in this half hour</p>
        <div className="shares">
          {(["seats", "standing", "crowded"] as const).map((k) => (
            <div key={k}>
              <b style={{ color: LEVEL_COLOR[k] }}>{Math.round(100 * stats.riders[k])}%</b>
              <span>{k === "seats" ? "have a seat" : k === "standing" ? "stand" : "are in a crowded bus"}</span>
            </div>
          ))}
        </div>
        <p className="note">
          Bus trips over each stretch: {Math.round(100 * stats.buses.seats)}% have a free seat, {Math.round(100 * stats.buses.standing)}% standing room,{" "}
          {Math.round(100 * stats.buses.crowded)}% crowded. Riders and buses differ because full buses carry more people, and many trips run against
          the peak flow or on outer stretches. {stats.n.toLocaleString()} stretches shown.
        </p>
      </aside>
      <section className="mapwrap">
        <MapView layers={layers} />
        <Legend />
      </section>
    </div>
  );
}
