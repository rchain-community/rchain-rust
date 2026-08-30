import { defineConfig } from "vite";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));

/**
 * Builds content-script.js as a standalone IIFE bundle - see
 * src/content-script.ts's comment for why this can't be a module, and
 * vite.config.inject.ts / vite.config.ts for the other two entries. Vite's
 * CLI rejects an array of configs from one file ("config must export or
 * return an object"), and Rollup's IIFE output rejects multiple inputs in
 * one build regardless of sharing - hence three separate config files,
 * chained in the "build" npm script. Runs after vite.config.ts (does not
 * empty dist/).
 */
export default defineConfig({
  root: here,
  build: {
    outDir: "dist",
    emptyOutDir: false,
    rollupOptions: {
      input: { "content-script": resolve(here, "src/content-script.ts") },
      output: { format: "iife", entryFileNames: "[name].js" }
    }
  }
});
