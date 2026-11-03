import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed port and no screen clearing so its own output stays visible.
export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5174, strictPort: true },
  define: { __MOCK__: JSON.stringify(mode === "mock") },
  build: { target: "safari16", outDir: "dist" },
  test: { environment: "jsdom", globals: true },
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
}));
