import { defineConfig } from "vitest/config";

// Separate from vite.config.mts on purpose: the frontend tests are plain .ts with no
// Svelte-component rendering, so this avoids pulling in the svelte plugin (and the type
// clash between vite's and vitest's own bundled vite Plugin types that comes with mixing
// `plugins` into a vitest/config-typed defineConfig).
export default defineConfig({
  test: {
    // Scoped to src/lib so `vitest run` doesn't also try to collect test/**/*.test.js --
    // those are the JS backend's node:test files, run separately via `npm test`.
    include: ["src/lib/**/*.test.ts"],
  },
});
