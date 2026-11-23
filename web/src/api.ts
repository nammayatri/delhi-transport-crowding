// Thin client for the crowding service. Same origin in production; proxied in dev.
const BASE = (import.meta as any).env?.VITE_API_BASE ?? "";

export type Level = "seats" | "standing" | "crowded";
export type Confidence = "low" | "medium" | "high";

async function get<T>(path: string): Promise<T> {
  const r = await fetch(BASE + path);
  if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error ?? r.statusText);
  return r.json();
}
async function post<T>(path: string, body: unknown): Promise<T> {
  const r = await fetch(BASE + path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error ?? r.statusText);
  return r.json();
}

export interface Place {
  stop_id: string | null;
  name: string;
  lat: number;
  lon: number;
  has_data: boolean;
  kind?: "stop" | "place";
}

export interface Segment {
  from_stop: string;
  to_stop: string;
  depart_secs: number;
  ride_secs: number;
  load?: number;
  load_low?: number;
  load_high?: number;
  level?: Level;
  usually?: boolean;
  p_seats?: number;
  p_standing?: number;
  p_crowded?: number;
  confidence?: Confidence;
}

export interface LegCrowding {
  summary: string;
  level: Level | null;
  level_at_boarding: Level | null;
  worst_level: Level | null;
  worst_from: string | null;
  worst_to: string | null;
  confidence: Confidence;
  crowding_extra_secs: number;
  segments: Segment[];
  has_data: boolean;
  live_factor: number;
}

export interface JourneyLeg {
  mode: "walk" | "bus";
  from: { name: string; lat: number; lon: number; stop_id: string | null };
  to: { name: string; lat: number; lon: number; stop_id: string | null };
  depart: string;
  arrive: string;
  minutes: number;
  route: string | null;
  stops: number;
  geometry: [number, number][];
  crowding: LegCrowding | null;
}

export interface JourneyOption {
  labels: string[];
  recommended: boolean;
  depart: string;
  arrive: string;
  minutes: number;
  minutes_from_request: number;
  transfers: number;
  walk_minutes: number;
  worst_level: Level | null;
  summary: string;
  crowding_extra_minutes: number;
  comfort_minutes: number;
  legs: JourneyLeg[];
  source: string;
}

export interface JourneyResponse {
  options: JourneyOption[];
  message?: string;
  requested?: { depart: string; daytype: string; comfort: number };
}

export const api = {
  meta: () => get<any>("/v1/meta"),
  stops: (q: string) => get<Place[]>(`/v1/stops?q=${encodeURIComponent(q)}&limit=8`),
  journeys: (body: unknown) => post<JourneyResponse>("/v1/journeys", body),
  map: (daytype: string, time: string, scenario: string) =>
    get<any>(`/v1/map/segments?daytype=${daytype}&time=${time}&scenario=${scenario}`),
  hotspots: (daytype: string, from: string, to: string, scenario: string) =>
    get<any>(`/v1/operator/hotspots?daytype=${daytype}&from=${from}&to=${to}&scenario=${scenario}&limit=25`),
  operatorRoutes: (daytype: string, from: string, to: string, scenario: string) =>
    get<any>(`/v1/operator/routes?daytype=${daytype}&from=${from}&to=${to}&scenario=${scenario}&limit=40`),
  liveVehicles: (scenario: string) => get<any>(`/v1/live/vehicles?scenario=${scenario}`),
  profile: (routeId: string, daytype: string, scenario: string) =>
    get<any>(`/v1/routes/${encodeURIComponent(routeId)}/profile?daytype=${daytype}&scenario=${scenario}`),
};

export const hm = (iso: string) => iso.slice(11, 16);
