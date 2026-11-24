import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development the API runs on :8090; requests to /v1 are proxied there. Assets use relative
// paths so the built app can be served from any sub-path.
export default defineConfig({
  base: "./",
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/v1": "http://localhost:8090",
      "/gtfs-rt": "http://localhost:8090",
    },
  },
});
