import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed port. `dev-public/` (mock catalog for browser previews) is served
// in dev only and never copied into the build.
export default defineConfig(({ command }) => ({
  plugins: [svelte()],
  clearScreen: false,
  publicDir: command === "serve" ? "dev-public" : false,
  server: { port: 1420, strictPort: true },
  build: { target: "es2022", cssMinify: true, reportCompressedSize: false },
}));
