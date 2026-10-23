import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [tailwindcss(), solid()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: "es2022", chunkSizeWarningLimit: 900 },
  // solid's browser build, so signals behave as in the app; the store tests run in jsdom
  resolve: { conditions: ["development", "browser"] },
  test: { environment: "jsdom", include: ["src/**/*.test.ts"] },
});
