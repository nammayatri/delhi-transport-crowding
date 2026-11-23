import { useMemo, useState } from "react";
import { api, hm, JourneyOption, JourneyResponse, Level, Place } from "../api";
import { ConfidenceDots, LEVEL_COLOR, LevelChip, Legend } from "../levels";
import { MapLayer, MapView } from "./MapView";
import { PlaceInput } from "./PlaceInput";

const DEFAULT_FROM: Place = { stop_id: null, name: "Anand Vihar ISBT", lat: 28.6469, lon: 77.3159, has_data: true, kind: "place" };
const DEFAULT_TO: Place = { stop_id: null, name: "Connaught Place", lat: 28.6315, lon: 77.2167, has_data: true, kind: "place" };

function CrowdStrip({ o }: { o: JourneyOption }) {
  // One bar for the whole journey: walking grey, each bus segment coloured by its level.
  const parts: { w: number; c: string; t: string }[] = [];
  for (const l of o.legs) {
    if (l.mode === "walk") parts.push({ w: l.minutes, c: "#d5dae0", t: `Walk ${Math.round(l.minutes)} min` });
    else if (l.crowding?.segments?.length) {
      for (const s of l.crowding.segments)
        parts.push({
          w: s.ride_secs / 60,
          c: LEVEL_COLOR[(s.level as Level) ?? "none"],
          t: `${l.route}: ${s.from_stop} → ${s.to_stop}${s.load != null ? ` · about ${Math.round(s.load)} on board` : ""}`,
        });
    } else parts.push({ w: l.minutes, c: LEVEL_COLOR.none, t: `${l.route}: no data` });
  }
  const total = parts.reduce((a, p) => a + p.w, 0) || 1;
  return (
    <div className="strip" aria-hidden>
      {parts.map((p, i) => (
        <span key={i} title={p.t} style={{ width: `${(100 * p.w) / total}%`, background: p.c }} />
      ))}
    </div>
  );
}

function OptionCard({ o, active, onClick }: { o: JourneyOption; active: boolean; onClick: () => void }) {
  const bus = o.legs.filter((l) => l.mode === "bus");
  return (
    <button className={"option" + (active ? " active" : "")} onClick={onClick}>
      <div className="labels">
        {o.recommended && <span className="badge rec">Best for you</span>}
        {o.labels.map((l) => (
          <span key={l} className="badge">
            {l}
          </span>
        ))}
      </div>
      <div className="times">
        <strong>
          {hm(o.depart)} → {hm(o.arrive)}
        </strong>
        <span>
          {o.minutes} min · {o.transfers === 0 ? "direct" : `${o.transfers} change${o.transfers > 1 ? "s" : ""}`} · {o.walk_minutes} min walk
        </span>
      </div>
      <CrowdStrip o={o} />
      {bus.map((l, i) => (
        <div key={i} className="leg">
          <span className="route">{l.route}</span>
          <LevelChip level={l.crowding?.worst_level ?? null} />
          <span className="legtext">
            {hm(l.depart)} from {l.from.name}
            <br />
            <small>{l.crowding?.summary}</small>
          </span>
        </div>
      ))}
      <div className="foot">
        {bus[0]?.crowding && <ConfidenceDots c={bus.map((b) => b.crowding?.confidence ?? "low").sort()[0] as any} />}
        {o.crowding_extra_minutes > 0.5 && <span className="feels">Feels like {Math.round(o.minutes + o.crowding_extra_minutes)} min</span>}
      </div>
    </button>
  );
}

export function Planner() {
  const [from, setFrom] = useState<Place | null>(DEFAULT_FROM);
  const [to, setTo] = useState<Place | null>(DEFAULT_TO);
  const [date, setDate] = useState("2026-10-28");
  const [time, setTime] = useState("18:00");
  const [comfort, setComfort] = useState(0.5);
  const [res, setRes] = useState<JourneyResponse | null>(null);
  const [sel, setSel] = useState(0);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const search = async () => {
    if (!from || !to) return;
    setBusy(true);
    setErr(null);
    try {
      const place = (p: Place) => (p.stop_id ? { stop_id: p.stop_id } : { lat: p.lat, lon: p.lon });
      const r = await api.journeys({ from: place(from), to: place(to), depart_at: `${date}T${time}:00+05:30`, comfort });
      setRes(r);
      setSel(0);
    } catch (e: any) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  };

  const o = res?.options[sel];
  const layers = useMemo<MapLayer[]>(() => {
    if (!o) return [];
    const feats: any[] = [];
    for (const l of o.legs) {
      if (l.mode === "walk") {
        feats.push({ type: "Feature", properties: { c: "#7d8894", w: 3, d: 1 }, geometry: { type: "LineString", coordinates: l.geometry } });
      } else if (l.crowding && l.crowding.segments.length === l.geometry.length - 1) {
        l.crowding.segments.forEach((s, i) =>
          feats.push({
            type: "Feature",
            properties: { c: LEVEL_COLOR[(s.level as Level) ?? "none"], w: 6, d: 0, from: s.from_stop, to: s.to_stop, load: s.load, level: s.level ?? "no data" },
            geometry: { type: "LineString", coordinates: [l.geometry[i], l.geometry[i + 1]] },
          }),
        );
      } else {
        feats.push({ type: "Feature", properties: { c: LEVEL_COLOR[l.crowding?.worst_level ?? "none"], w: 6, d: 0 }, geometry: { type: "LineString", coordinates: l.geometry } });
      }
    }
    return [
      {
        id: "journey",
        data: { type: "FeatureCollection", features: feats },
        paint: { "line-color": ["get", "c"], "line-width": ["get", "w"] },
        onClick: (p) => (p.from ? `<b>${p.from} → ${p.to}</b><br/>${p.level}${p.load ? ` · about ${Math.round(p.load)} on board` : ""}` : null),
      },
    ];
  }, [o]);
  const fit = useMemo(() => o?.legs.flatMap((l) => l.geometry), [o]);

  return (
    <div className="split">
      <aside className="panel">
        <h2>Plan a bus trip</h2>
        <p className="lead">See how full each bus usually is, not just how long it takes.</p>
        <PlaceInput label="From" value={from} onChange={setFrom} />
        <PlaceInput label="To" value={to} onChange={setTo} />
        <div className="row">
          <label className="field">
            <span>Date</span>
            <input type="date" value={date} onChange={(e) => setDate(e.target.value)} />
          </label>
          <label className="field">
            <span>Leave at</span>
            <input type="time" value={time} onChange={(e) => setTime(e.target.value)} />
          </label>
        </div>
        <label className="field">
          <span>What matters more?</span>
          <input type="range" min={0} max={1} step={0.25} value={comfort} onChange={(e) => setComfort(+e.target.value)} />
          <div className="scale">
            <span>Get there fastest</span>
            <span>Avoid crowds</span>
          </div>
        </label>
        <button className="primary" onClick={search} disabled={busy || !from || !to}>
          {busy ? "Searching…" : "Find buses"}
        </button>
        {err && <p className="error">{err}</p>}
        {res?.message && <p className="muted">{res.message}</p>}
        <div className="options">
          {res?.options.map((op, i) => (
            <OptionCard key={i} o={op} active={i === sel} onClick={() => setSel(i)} />
          ))}
        </div>
        {res && res.options.length > 0 && (
          <p className="note">
            Crowding is estimated from October 2026 ticket records for the same day type and time, and scaled to all riders. "Usually" means on most buses at that
            time.
          </p>
        )}
      </aside>
      <section className="mapwrap">
        <MapView layers={layers} fit={fit} />
        <Legend />
      </section>
    </div>
  );
}
