import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// https://v2.tauri.app/start/frontend/vite/ -- fixed port + strictPort so Tauri's devUrl
// (src-tauri/tauri.conf.json) can point at a known address; watch.ignored so Vite doesn't
// restart on Rust file changes (cargo's own file watcher already handles those).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "ui-dist",
  },
});
