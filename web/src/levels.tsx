import type { Confidence, Level } from "./api";

export const LEVEL_COLOR: Record<Level | "none", string> = {
  seats: "#1f7a55",
  standing: "#b26a00",
  crowded: "#c0392b",
  none: "#8a94a0",
};
export const LEVEL_TINT: Record<Level | "none", string> = {
  seats: "#e3f3ea",
  standing: "#fdf0dc",
  crowded: "#fbe5e2",
  none: "#eef0f3",
};
export const LEVEL_TEXT: Record<Level | "none", string> = {
  seats: "Seats",
  standing: "Standing",
  crowded: "Crowded",
  none: "No data",
};

export function LevelIcon({ level, size = 16 }: { level: Level | null; size?: number }) {
  const c = LEVEL_COLOR[level ?? "none"];
  const s = { width: size, height: size, flex: "none" } as const;
  if (level === "seats")
    return (
      <svg viewBox="0 0 24 24" style={s} aria-hidden>
        <path fill={c} d="M7 3h6a2 2 0 0 1 2 2v7h2a2 2 0 0 1 2 2v7h-2v-4H8l-1 4H5l1.2-5.2A3 3 0 0 1 5 12V5a2 2 0 0 1 2-2z" />
      </svg>
    );
  if (level === "standing")
    return (
      <svg viewBox="0 0 24 24" style={s} aria-hidden>
        <circle cx="12" cy="4" r="2.2" fill={c} />
        <path fill={c} d="M9 8h6l1 7h-2l-.5 7h-3L10 15H8z" />
      </svg>
    );
  if (level === "crowded")
    return (
      <svg viewBox="0 0 24 24" style={s} aria-hidden>
        <circle cx="6" cy="6" r="2" fill={c} />
        <circle cx="12" cy="4.5" r="2.2" fill={c} />
        <circle cx="18" cy="6" r="2" fill={c} />
        <path fill={c} d="M3 10h6l.8 6H8.5L8 22H5l-.5-6H2.4zM9.5 8.5h5l1 7.5h-1.6L13.5 22h-3l-.4-6H8.5zM15 10h6l.6 6h-2l-.6 6h-3l-.5-6h-1.3z" />
      </svg>
    );
  return (
    <svg viewBox="0 0 24 24" style={s} aria-hidden>
      <circle cx="12" cy="12" r="8" fill="none" stroke={c} strokeWidth="2" strokeDasharray="3 3" />
    </svg>
  );
}

export function LevelChip({ level, usually }: { level: Level | null; usually?: boolean }) {
  const k = level ?? "none";
  return (
    <span className="chip" style={{ background: LEVEL_TINT[k], color: LEVEL_COLOR[k] }}>
      <LevelIcon level={level} />
      {usually === false && level ? "Often " + LEVEL_TEXT[k].toLowerCase() : LEVEL_TEXT[k]}
    </span>
  );
}

export function ConfidenceDots({ c }: { c: Confidence }) {
  const n = c === "high" ? 3 : c === "medium" ? 2 : 1;
  return (
    <span className="conf" title={`Estimate confidence: ${c} (based on how many tickets we saw for this bus and time)`}>
      {[0, 1, 2].map((i) => (
        <i key={i} className={i < n ? "on" : ""} />
      ))}
      <span>{c} confidence</span>
    </span>
  );
}

export function Legend() {
  return (
    <div className="legend">
      {(["seats", "standing", "crowded"] as Level[]).map((l) => (
        <span key={l}>
          <i style={{ background: LEVEL_COLOR[l] }} />
          {l === "seats" ? "Seats available" : l === "standing" ? "Standing room" : "Crowded"}
        </span>
      ))}
    </div>
  );
}
