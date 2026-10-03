import react from "@vitejs/plugin-react";
import { configDefaults, defineConfig } from "vitest/config";

import pkg from "./package.json" with { type: "json" };

export default defineConfig({
  plugins: [react()],
  define: {
    "import.meta.env.VITE_APP_VERSION": JSON.stringify(pkg.version),
  },
  clearScreen: false,
  test: {
    // CCSwitch is an independent nested Vite project with its own alias,
    // setup and test command. Keep root Vitest from loading its tests under
    // the manager's configuration; CCSwitch runs its own full suite.
    exclude: [
      ...configDefaults.exclude,
      ".claude/**",
      ".codex-audit/**",
      "CCSwitch/CCSwitch/**",
    ],
    environment: "jsdom",
    environmentOptions: {
      jsdom: {
        url: "http://localhost/",
      },
    },
    setupFiles: ["./vitest.setup.ts"],
    globals: false,
  },
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
});
