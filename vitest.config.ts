import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Vitest config is kept separate from vite.config.ts (which carries
// Tauri-specific dev-server options) so the test runner stays lightweight.
export default defineConfig({
  plugins: [react()],
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    css: false,
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
});
