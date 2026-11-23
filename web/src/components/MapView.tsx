import { useEffect, useRef } from "react";
import maplibregl, { Map as MlMap } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";

// Free vector basemap, no key needed (https://openfreemap.org).
const STYLE = "https://tiles.openfreemap.org/styles/positron";

export interface MapLayer {
  id: string;
  data: any; // GeoJSON FeatureCollection
  paint: any;
  type?: "line" | "circle";
  onClick?: (props: any, lngLat: [number, number]) => string | null;
}

export function MapView({ layers, fit }: { layers: MapLayer[]; fit?: [number, number][] }) {
  const el = useRef<HTMLDivElement>(null);
  const map = useRef<MlMap | null>(null);
  const ready = useRef(false);
  const pending = useRef<() => void>(() => {});
  const ours = useRef<Set<string>>(new Set());

  useEffect(() => {
    const m = new maplibregl.Map({ container: el.current!, style: STYLE, center: [77.21, 28.62], zoom: 10.3 });
    m.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
    m.on("load", () => {
      ready.current = true;
      pending.current();
    });
    map.current = m;
    return () => m.remove();
  }, []);

  useEffect(() => {
    const apply = () => {
      const m = map.current!;
      const want = new Set(layers.map((l) => l.id));
      for (const id of [...ours.current]) {
        if (!want.has(id)) {
          if (m.getLayer(id)) m.removeLayer(id);
          if (m.getSource(id)) m.removeSource(id);
          ours.current.delete(id);
        }
      }
      for (const l of layers) {
        const src = m.getSource(l.id) as maplibregl.GeoJSONSource | undefined;
        if (src) {
          src.setData(l.data);
          for (const [k, v] of Object.entries(l.paint)) m.setPaintProperty(l.id, k, v);
        } else {
          m.addSource(l.id, { type: "geojson", data: l.data });
          ours.current.add(l.id);
          m.addLayer({ id: l.id, type: l.type ?? "line", source: l.id, paint: l.paint, layout: l.type === "circle" ? {} : { "line-cap": "round", "line-join": "round" } } as any);
          if (l.onClick) {
            m.on("click", l.id, (e) => {
              const f = e.features?.[0];
              if (!f) return;
              const html = l.onClick!(f.properties, [e.lngLat.lng, e.lngLat.lat]);
              if (html) new maplibregl.Popup({ maxWidth: "320px" }).setLngLat(e.lngLat).setHTML(html).addTo(m);
            });
            m.on("mouseenter", l.id, () => (m.getCanvas().style.cursor = "pointer"));
            m.on("mouseleave", l.id, () => (m.getCanvas().style.cursor = ""));
          }
        }
      }
      if (fit && fit.length > 1) {
        const b = new maplibregl.LngLatBounds(fit[0], fit[0]);
        fit.forEach((p) => b.extend(p));
        m.fitBounds(b, { padding: 60, maxZoom: 14, duration: 500 });
      }
    };
    if (ready.current) apply();
    else pending.current = apply;
  }, [layers, fit]);

  return <div ref={el} className="map" />;
}
