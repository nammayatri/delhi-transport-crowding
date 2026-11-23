import { useEffect, useState } from "react";
import { api, Place } from "../api";

export function PlaceInput({ label, value, onChange }: { label: string; value: Place | null; onChange: (p: Place) => void }) {
  const [q, setQ] = useState(value?.name ?? "");
  const [list, setList] = useState<Place[]>([]);
  const [open, setOpen] = useState(false);
  useEffect(() => setQ(value?.name ?? ""), [value]);
  useEffect(() => {
    if (!open || q.trim().length < 2) return setList([]);
    const t = setTimeout(() => api.stops(q).then(setList).catch(() => setList([])), 150);
    return () => clearTimeout(t);
  }, [q, open]);
  return (
    <label className="field place">
      <span>{label}</span>
      <input
        value={q}
        placeholder="Stop or place"
        onChange={(e) => {
          setQ(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 150)}
      />
      {open && list.length > 0 && (
        <ul className="suggest">
          {list.map((p) => (
            <li
              key={(p.stop_id ?? "") + p.name}
              onMouseDown={() => {
                onChange(p);
                setOpen(false);
              }}
            >
              {p.name}
              {p.kind === "place" && <em>place</em>}
            </li>
          ))}
        </ul>
      )}
    </label>
  );
}
