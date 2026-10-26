import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development, forward API and WebSocket traffic to the Rust gateway.
const gateway = process.env.GATEWAY_URL ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": gateway,
      "/healthz": gateway,
      "/ws": { target: gateway.replace(/^http/, "ws"), ws: true },
    },
  },
});
