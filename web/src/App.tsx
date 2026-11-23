import { useEffect, useState } from "react";
import { api } from "./api";
import { CrowdMap } from "./components/CrowdMap";
import { Operator } from "./components/Operator";
import { Planner } from "./components/Planner";

type Tab = "plan" | "map" | "ops";

export function App() {
  const [tab, setTab] = useState<Tab>("plan");
  const [meta, setMeta] = useState<any>(null);
  useEffect(() => {
    api.meta().then(setMeta).catch(() => {});
  }, []);
  return (
    <div className="app">
      <header>
        <div className="brand">
          <span className="logo">▮▮</span> Delhi bus crowding
        </div>
        <nav>
          {(
            [
              ["plan", "Plan a trip"],
              ["map", "Crowding map"],
              ["ops", "For operators"],
            ] as [Tab, string][]
          ).map(([k, v]) => (
            <button key={k} className={tab === k ? "on" : ""} onClick={() => setTab(k)}>
              {v}
            </button>
          ))}
        </nav>
        {meta && (
          <div className="meta">
            Model built from {(meta.meta.tickets_total / 1e6).toFixed(2)} M tickets, {meta.meta.date_from} to {meta.meta.date_to}
          </div>
        )}
      </header>
      <main>{tab === "plan" ? <Planner /> : tab === "map" ? <CrowdMap /> : <Operator />}</main>
    </div>
  );
}
